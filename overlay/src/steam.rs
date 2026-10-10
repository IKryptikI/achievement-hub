//! Games that have Steam achievements already report each unlock to Steam
//! (`ISteamUserStats::SetAchievement`). We hook that call, so those games
//! need no memory mapping: the game itself says what was earned, by its
//! Steam API name, and the definition (or the game's schema) maps that name
//! to our achievement.
//!
//! Works with Valve's steam_api.dll and with the drop-in replacements some
//! copies ship, since both hand the game the same interface.

use std::ffi::{CStr, CString, c_char, c_void};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use windows_sys::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleA, GetProcAddress};
use windows_sys::Win32::System::Memory::{PAGE_READWRITE, VirtualProtect};

/// API names the game reported (or that Steam says are already earned).
static REPORTED: Mutex<Vec<String>> = Mutex::new(Vec::new());
static mut ORIGINAL_SET: usize = 0;

type SetFn = unsafe extern "thiscall" fn(*mut c_void, *const c_char) -> bool;
type GetFn = unsafe extern "thiscall" fn(*mut c_void, *const c_char, *mut bool) -> bool;

unsafe extern "thiscall" fn set_achievement(this: *mut c_void, name: *const c_char) -> bool {
    if !name.is_null() {
        let n = unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned();
        if let Ok(mut q) = REPORTED.lock() {
            q.push(n);
        }
    }
    let original: SetFn = unsafe { std::mem::transmute(ORIGINAL_SET) };
    unsafe { original(this, name) }
}

/// Other stats calls the game makes (logged, to see how a game reports).
static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());
static ORIGINAL: [std::sync::atomic::AtomicUsize; 16] = [const { std::sync::atomic::AtomicUsize::new(0) }; 16];

fn note(what: String) {
    if let Ok(mut q) = SEEN.lock() {
        q.push(what);
    }
}

/// SetStat (int or float: both a name and 4 bytes).
unsafe extern "thiscall" fn set_stat<const S: usize>(this: *mut c_void, name: *const c_char, v: u32) -> bool {
    if !name.is_null() {
        note(format!("SetStat {} = {v:#x}", unsafe { CStr::from_ptr(name) }.to_string_lossy()));
    }
    let f: unsafe extern "thiscall" fn(*mut c_void, *const c_char, u32) -> bool = unsafe { std::mem::transmute(ORIGINAL[S].load(std::sync::atomic::Ordering::Relaxed)) };
    unsafe { f(this, name, v) }
}

unsafe extern "thiscall" fn store_stats<const S: usize>(this: *mut c_void) -> bool {
    note("StoreStats".into());
    let f: unsafe extern "thiscall" fn(*mut c_void) -> bool = unsafe { std::mem::transmute(ORIGINAL[S].load(std::sync::atomic::Ordering::Relaxed)) };
    unsafe { f(this) }
}

/// Stats calls seen since the last call (deduplicated).
pub fn take_seen() -> Vec<String> {
    let mut v = SEEN.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
    v.dedup();
    v
}

unsafe fn patch(vtable: *mut usize, slot: usize, hook: usize) -> Option<usize> {
    let p = unsafe { vtable.add(slot) };
    if unsafe { *p } == hook {
        return None;
    }
    let mut old = 0;
    if unsafe { VirtualProtect(p.cast(), 4, PAGE_READWRITE, &mut old) } == 0 {
        return None;
    }
    let orig = unsafe { *p };
    unsafe { *p = hook };
    unsafe { VirtualProtect(p.cast(), 4, old, &mut old) };
    Some(orig)
}

/// Everything reported since the last call.
pub fn take() -> Vec<String> {
    REPORTED.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
}

/// "STEAMUSERSTATS_INTERFACE_VERSION0NN" in a file, as NN.
fn interface_version(path: &Path) -> Option<(String, u32)> {
    let d = std::fs::read(path).ok()?;
    let tag = b"STEAMUSERSTATS_INTERFACE_VERSION";
    let at = d.windows(tag.len()).position(|w| w == tag)?;
    let digits: Vec<u8> = d[at + tag.len()..].iter().take_while(|b| b.is_ascii_digit()).copied().collect();
    let v = std::str::from_utf8(&digits).ok()?.parse().ok()?;
    Some((format!("STEAMUSERSTATS_INTERFACE_VERSION{}", String::from_utf8_lossy(&digits)), v))
}

pub fn exe_path() -> std::path::PathBuf {
    let mut buf = [0u16; 1024];
    let n = unsafe { GetModuleFileNameW(std::ptr::null_mut(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
    String::from_utf16_lossy(&buf[..n]).into()
}

/// The game's ISteamUserStats object and its interface version, once
/// Steam is up.
unsafe fn user_stats() -> Option<(*mut c_void, u32)> {
    let module = unsafe { GetModuleHandleA(c"steam_api.dll".as_ptr().cast()) };
    if module.is_null() {
        return None;
    }
    // Older SDKs: steam_api.dll hands it out itself.
    if let Some(f) = unsafe { GetProcAddress(module, c"SteamUserStats".as_ptr().cast()) } {
        let f: unsafe extern "C" fn() -> *mut c_void = unsafe { std::mem::transmute(f) };
        let stats = unsafe { f() };
        if stats.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let n = unsafe { GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) } as usize;
        let v = interface_version(Path::new(&String::from_utf16_lossy(&buf[..n]))).map_or(7, |(_, v)| v);
        return Some((stats, v));
    }
    // Newer SDKs: the game asks for the exact version it was built with
    // (the same object comes back for the same version).
    let (name, v) = interface_version(&exe_path())?;
    let user = unsafe { GetProcAddress(module, c"SteamAPI_GetHSteamUser".as_ptr().cast()) }?;
    let find = unsafe { GetProcAddress(module, c"SteamInternal_FindOrCreateUserInterface".as_ptr().cast()) }?;
    let user: unsafe extern "C" fn() -> i32 = unsafe { std::mem::transmute(user) };
    let find: unsafe extern "C" fn(i32, *const c_char) -> *mut c_void = unsafe { std::mem::transmute(find) };
    let h = unsafe { user() };
    if h == 0 {
        return None;
    }
    let name = CString::new(name).ok()?;
    let stats = unsafe { find(h, name.as_ptr()) };
    (!stats.is_null()).then_some((stats, v))
}

/// Watches for Steam in the background: hooks SetAchievement once it's up,
/// then every few seconds asks which of `known` (API names) are already
/// earned, so unlocks from before the overlay was installed count too.
pub fn start(known: Vec<String>, log: impl Fn(&str) + Send + 'static) {
    std::thread::spawn(move || {
        let (stats, version) = loop {
            if let Some(s) = unsafe { user_stats() } {
                break s;
            }
            std::thread::sleep(Duration::from_secs(1));
        };
        // Version 13 dropped RequestCurrentStats from the front of the table.
        let set_slot = if version >= 13 { 6 } else { 7 };
        if version < 3 {
            log(&format!("steam: user stats interface v{version} is too old to hook"));
            return;
        }
        let get: GetFn = unsafe {
            let vtable = *(stats as *const *mut usize);
            let slot = vtable.add(set_slot);
            if *slot != set_achievement as *const () as usize {
                let mut old = 0;
                if VirtualProtect(slot.cast(), 4, PAGE_READWRITE, &mut old) == 0 {
                    log("steam: couldn't hook SetAchievement");
                    return;
                }
                ORIGINAL_SET = *slot;
                *slot = set_achievement as *const () as usize;
                VirtualProtect(slot.cast(), 4, old, &mut old);
            }
            // SetStat x2 sit 4 and 3 slots before SetAchievement; StoreStats
            // follows ClearAchievement (and GetAchievementAndUnlockTime from v7).
            let store_slot = set_slot + if version >= 7 { 3 } else { 2 };
            for (slot, hook) in [
                (set_slot - 4, set_stat::<0> as *const () as usize),
                (set_slot - 3, set_stat::<1> as *const () as usize),
                (store_slot, store_stats::<2> as *const () as usize),
            ] {
                let k = if slot == store_slot { 2 } else { slot + 4 - set_slot };
                if let Some(orig) = patch(vtable, slot, hook) {
                    ORIGINAL[k].store(orig, std::sync::atomic::Ordering::Relaxed);
                }
            }
            std::mem::transmute(*vtable.add(set_slot - 1))
        };
        log(&format!("steam: watching achievements (user stats v{version})"));
        let names: Vec<CString> = known.iter().filter_map(|n| CString::new(n.as_str()).ok()).collect();
        let mut earned = vec![false; names.len()];
        loop {
            for (i, n) in names.iter().enumerate() {
                let mut got = false;
                if !earned[i] && unsafe { get(stats, n.as_ptr(), &mut got) } && got {
                    earned[i] = true;
                    if let Ok(mut q) = REPORTED.lock() {
                        q.push(known[i].clone());
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(5));
        }
    });
}

/// API name -> English display name from a Steam achievement schema the
/// game folder carries (`steam_settings/achievements.json`), if any.
pub fn local_schema(exe_dir: &Path) -> Vec<(String, String)> {
    let Ok(text) = std::fs::read_to_string(exe_dir.join("steam_settings").join("achievements.json")) else { return Vec::new() };
    let Ok(serde_json::Value::Array(list)) = serde_json::from_str(&text) else { return Vec::new() };
    list.iter()
        .filter_map(|a| {
            let api = a.get("name")?.as_str()?;
            let shown = a.get("displayName")?;
            let shown = shown.as_str().or_else(|| shown.get("english")?.as_str())?;
            Some((api.to_string(), shown.to_string()))
        })
        .collect()
}
