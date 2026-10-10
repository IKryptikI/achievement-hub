//! Input gate: while the overlay panel is open, the game sees no keyboard or
//! controller input, and it never sees the overlay's own hotkey.
//!
//! Works by patching the game exe's import table (IAT) for the input APIs it
//! polls, so only the game's own calls are affected; the overlay keeps
//! reading real input. Many older games poll input directly (no window
//! messages), which is why blocking messages alone isn't enough.

use std::ffi::{c_void, CStr};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicUsize, Ordering};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{VirtualProtect, PAGE_READWRITE};

/// True while the panel is open: the game gets no input.
pub static BLOCK: AtomicBool = AtomicBool::new(false);
/// The panel hotkey's main key (VK code, 0 = none) and modifier VKs
/// (0 = unused). The game never sees this combination.
pub static HOTKEY_VK: AtomicU16 = AtomicU16::new(0);
pub static HOTKEY_MODS: [AtomicU16; 3] = [AtomicU16::new(0), AtomicU16::new(0), AtomicU16::new(0)];

/// How many times the game tried to read the hotkey's key while it was held,
/// per input route (diagnostics): GetAsyncKeyState, GetKeyboardState,
/// DirectInput state, DirectInput buffered data.
pub static HOTKEY_HIDDEN: [std::sync::atomic::AtomicU32; 4] = [
    std::sync::atomic::AtomicU32::new(0),
    std::sync::atomic::AtomicU32::new(0),
    std::sync::atomic::AtomicU32::new(0),
    std::sync::atomic::AtomicU32::new(0),
];

fn hidden(route: usize) {
    HOTKEY_HIDDEN[route].fetch_add(1, Ordering::Relaxed);
}

// Previous IAT targets (the functions we forward to).
static ORIG_GET_ASYNC_KEY_STATE: AtomicUsize = AtomicUsize::new(0);
static ORIG_GET_KEYBOARD_STATE: AtomicUsize = AtomicUsize::new(0);
static ORIG_JOY_GET_POS_EX: AtomicUsize = AtomicUsize::new(0);
static ORIG_JOY_GET_POS: AtomicUsize = AtomicUsize::new(0);

type GetAsyncKeyStateFn = unsafe extern "system" fn(i32) -> i16;
type GetKeyboardStateFn = unsafe extern "system" fn(*mut u8) -> i32;
type JoyGetPosExFn = unsafe extern "system" fn(u32, *mut JoyInfoEx) -> u32;
type JoyGetPosFn = unsafe extern "system" fn(u32, *mut JoyInfo) -> u32;

#[repr(C)]
struct JoyInfoEx {
    size: u32,
    flags: u32,
    axes: [u32; 6], // X Y Z R U V
    buttons: u32,
    button_number: u32,
    pov: u32,
    reserved: [u32; 2],
}

#[repr(C)]
struct JoyInfo {
    x: u32,
    y: u32,
    z: u32,
    buttons: u32,
}

const JOY_POVCENTERED: u32 = 0xFFFF;
const AXIS_CENTER: u32 = 0x7FFF;

fn hotkey_held(orig: GetAsyncKeyStateFn) -> bool {
    HOTKEY_MODS.iter().map(|m| m.load(Ordering::Relaxed)).filter(|&m| m != 0)
        // SAFETY: forwarding to the original GetAsyncKeyState.
        .all(|m| unsafe { orig(m as i32) } < 0)
}

unsafe extern "system" fn get_async_key_state(vk: i32) -> i16 {
    // SAFETY: only installed after the original was saved.
    let orig: GetAsyncKeyStateFn = unsafe { std::mem::transmute(ORIG_GET_ASYNC_KEY_STATE.load(Ordering::Relaxed)) };
    if BLOCK.load(Ordering::Relaxed) {
        return 0;
    }
    let hk = HOTKEY_VK.load(Ordering::Relaxed) as i32;
    if hk != 0 && vk == hk && hotkey_held(orig) {
        if unsafe { orig(vk) } < 0 {
            hidden(0);
        }
        return 0;
    }
    unsafe { orig(vk) }
}

unsafe extern "system" fn get_keyboard_state(buf: *mut u8) -> i32 {
    let orig: GetKeyboardStateFn = unsafe { std::mem::transmute(ORIG_GET_KEYBOARD_STATE.load(Ordering::Relaxed)) };
    let r = unsafe { orig(buf) };
    if r != 0 && !buf.is_null() {
        // SAFETY: GetKeyboardState fills a 256-byte buffer.
        let keys = unsafe { std::slice::from_raw_parts_mut(buf, 256) };
        if BLOCK.load(Ordering::Relaxed) {
            keys.fill(0);
        } else {
            let hk = HOTKEY_VK.load(Ordering::Relaxed) as usize;
            let mods_held = HOTKEY_MODS
                .iter()
                .map(|m| m.load(Ordering::Relaxed) as usize)
                .filter(|&m| m != 0)
                .all(|m| keys[m] & 0x80 != 0);
            if hk != 0 && mods_held {
                if keys[hk] & 0x80 != 0 {
                    hidden(1);
                }
                keys[hk] = 0;
            }
        }
    }
    r
}

unsafe extern "system" fn joy_get_pos_ex(id: u32, info: *mut JoyInfoEx) -> u32 {
    let orig: JoyGetPosExFn = unsafe { std::mem::transmute(ORIG_JOY_GET_POS_EX.load(Ordering::Relaxed)) };
    let r = unsafe { orig(id, info) };
    if r == 0 && !info.is_null() && BLOCK.load(Ordering::Relaxed) {
        // SAFETY: the caller passed a valid JOYINFOEX.
        let info = unsafe { &mut *info };
        info.axes = [AXIS_CENTER; 6];
        info.buttons = 0;
        info.button_number = 0;
        info.pov = JOY_POVCENTERED;
    }
    r
}

unsafe extern "system" fn joy_get_pos(id: u32, info: *mut JoyInfo) -> u32 {
    let orig: JoyGetPosFn = unsafe { std::mem::transmute(ORIG_JOY_GET_POS.load(Ordering::Relaxed)) };
    let r = unsafe { orig(id, info) };
    if r == 0 && !info.is_null() && BLOCK.load(Ordering::Relaxed) {
        let info = unsafe { &mut *info };
        info.x = AXIS_CENTER;
        info.y = AXIS_CENTER;
        info.z = AXIS_CENTER;
        info.buttons = 0;
    }
    r
}

/// Replaces `dll!func` in the main exe's import table with `hook`, storing
/// the previous target in `orig`. Returns false if the exe doesn't import it.
unsafe fn patch_iat(dll: &str, func: &str, hook: usize, orig: &AtomicUsize) -> bool {
    unsafe {
        let base = GetModuleHandleW(std::ptr::null()) as usize;
        if base == 0 {
            return false;
        }
        let rd = |off: usize| *((base + off) as *const u32) as usize;
        let pe = rd(0x3C);
        // Import directory: data directory 1. Its offset in the optional
        // header differs between PE32 (96) and PE32+ (112).
        let magic = *((base + pe + 24) as *const u16);
        let dd = pe + 24 + if magic == 0x20B { 112 } else { 96 };
        let imports = rd(dd + 8);
        if imports == 0 {
            return false;
        }
        let mut desc = base + imports;
        let ptr = std::mem::size_of::<usize>();
        loop {
            let names = *(desc as *const u32) as usize;
            let name_rva = *((desc + 12) as *const u32) as usize;
            let iat = *((desc + 16) as *const u32) as usize;
            if name_rva == 0 {
                return false;
            }
            let dll_name = CStr::from_ptr((base + name_rva) as *const i8).to_string_lossy();
            if dll_name.eq_ignore_ascii_case(dll) && names != 0 {
                let mut i = 0;
                loop {
                    let thunk = *((base + names + i * ptr) as *const usize);
                    if thunk == 0 {
                        break;
                    }
                    let by_ordinal = thunk & (1usize << (ptr * 8 - 1)) != 0;
                    if !by_ordinal {
                        let fname = CStr::from_ptr((base + thunk + 2) as *const i8).to_string_lossy();
                        if fname == func {
                            let slot = (base + iat + i * ptr) as *mut usize;
                            let mut old = 0;
                            if VirtualProtect(slot as *const c_void, ptr, PAGE_READWRITE, &mut old) == 0 {
                                return false;
                            }
                            orig.store(*slot, Ordering::Relaxed);
                            *slot = hook;
                            VirtualProtect(slot as *const c_void, ptr, old, &mut old);
                            return true;
                        }
                    }
                    i += 1;
                }
            }
            desc += 20;
        }
    }
}

/// Installs the gate; returns the names of the APIs that were hooked.
pub fn install() -> Vec<&'static str> {
    let hooks: [(&str, &str, usize, &AtomicUsize); 4] = [
        ("USER32.dll", "GetAsyncKeyState", get_async_key_state as GetAsyncKeyStateFn as usize, &ORIG_GET_ASYNC_KEY_STATE),
        ("USER32.dll", "GetKeyboardState", get_keyboard_state as GetKeyboardStateFn as usize, &ORIG_GET_KEYBOARD_STATE),
        ("WINMM.dll", "joyGetPosEx", joy_get_pos_ex as JoyGetPosExFn as usize, &ORIG_JOY_GET_POS_EX),
        ("WINMM.dll", "joyGetPos", joy_get_pos as JoyGetPosFn as usize, &ORIG_JOY_GET_POS),
    ];
    hooks
        .into_iter()
        // SAFETY: each hook has the same signature as the function it replaces.
        .filter(|(dll, func, hook, orig)| unsafe { patch_iat(dll, func, *hook, orig) })
        .map(|(_, func, _, _)| func)
        .collect()
}

// ── DirectInput ─────────────────────────────────────────────────────────
//
// Many games (RE2 included) load dinput.dll at runtime, so the IAT patch
// above never sees them. Instead we create our own DirectInput keyboard and
// joystick devices to find the implementation of IDirectInputDevice's
// GetDeviceState/GetDeviceData, and hook those functions directly; every
// device of the same kind in the process shares that code.

use hudhook::mh::{MH_CreateHook, MH_EnableHook, MH_Initialize};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, MapVirtualKeyW, MAPVK_VK_TO_VSC};

type GetDeviceStateFn = unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32;
type GetDeviceDataFn = unsafe extern "system" fn(*mut c_void, u32, *mut u8, *mut u32, u32) -> i32;
type GetPropertyFn = unsafe extern "system" fn(*mut c_void, usize, *mut DiPropRange) -> i32;

// Originals, keyed by the hooked function's address (keyboard and joystick
// devices may or may not share an implementation).
static DI_HOOKS: std::sync::Mutex<Vec<(usize, usize, usize)>> = std::sync::Mutex::new(Vec::new()); // (state fn, orig state, orig data)

#[repr(C)]
struct DiPropRange {
    size: u32,
    header_size: u32,
    obj: u32,
    how: u32,
    min: i32,
    max: i32,
}

const DIPROP_RANGE: usize = 4;
const DIPH_BYOFFSET: u32 = 1;
const KEYBOARD_STATE: u32 = 256;

/// Scan code (= DirectInput key index) of a virtual key.
fn dik(vk: u16) -> usize {
    // SAFETY: plain mapping query.
    unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) as usize }
}

/// Whether the hotkey's modifiers are physically held right now.
fn hotkey_mods_held() -> bool {
    HOTKEY_MODS
        .iter()
        .map(|m| m.load(Ordering::Relaxed))
        .filter(|&m| m != 0)
        // SAFETY: the real GetAsyncKeyState (our own import, not the game's).
        .all(|m| unsafe { GetAsyncKeyState(m as i32) } < 0)
}

fn originals(this: *mut c_void) -> Option<(usize, usize)> {
    // The device's vtable tells us which implementation is being called.
    // SAFETY: `this` is a live IDirectInputDevice passed by the game.
    let state_fn = unsafe { *(*(this as *const *const usize)).add(9) };
    let hooks = DI_HOOKS.lock().ok()?;
    hooks.iter().find(|(f, _, _)| *f == state_fn).map(|&(_, s, d)| (s, d))
}

unsafe extern "system" fn di_get_device_state(this: *mut c_void, size: u32, data: *mut c_void) -> i32 {
    let Some((orig, _)) = originals(this) else { return -1 };
    let orig: GetDeviceStateFn = unsafe { std::mem::transmute(orig) };
    let hr = unsafe { orig(this, size, data) };
    if hr < 0 || data.is_null() {
        return hr;
    }
    // SAFETY: the caller's buffer is `size` bytes.
    let buf = unsafe { std::slice::from_raw_parts_mut(data as *mut u8, size as usize) };
    if size == KEYBOARD_STATE {
        if BLOCK.load(Ordering::Relaxed) {
            buf.fill(0);
        } else {
            let hk = HOTKEY_VK.load(Ordering::Relaxed);
            if hk != 0 && hotkey_mods_held() {
                let k = dik(hk).min(255);
                if buf[k] & 0x80 != 0 {
                    hidden(2);
                }
                buf[k] = 0;
            }
        }
    } else if BLOCK.load(Ordering::Relaxed) && size >= 80 {
        // DIJOYSTATE / DIJOYSTATE2: 6 axes, 2 sliders, 4 POVs, then buttons.
        // Centre each axis within the range the game configured.
        let mut centers = [0i32; 6];
        // SAFETY: vtable slot 5 is GetProperty on every IDirectInputDevice.
        let get_prop: GetPropertyFn = unsafe { std::mem::transmute(*(*(this as *const *const usize)).add(5)) };
        for (i, c) in centers.iter_mut().enumerate() {
            let mut r = DiPropRange { size: 24, header_size: 16, obj: i as u32 * 4, how: DIPH_BYOFFSET, min: 0, max: 0 };
            if unsafe { get_prop(this, DIPROP_RANGE, &mut r) } >= 0 {
                *c = (r.min + r.max) / 2;
            }
        }
        let ints = data as *mut i32;
        for (i, c) in centers.iter().enumerate() {
            unsafe { *ints.add(i) = *c };
        }
        for i in 8..12 {
            unsafe { *ints.add(i) = -1 }; // POVs centred
        }
        buf[48..].fill(0); // buttons (and DIJOYSTATE2 extras)
    }
    hr
}

unsafe extern "system" fn di_get_device_data(this: *mut c_void, obj_size: u32, data: *mut u8, count: *mut u32, flags: u32) -> i32 {
    let Some((_, orig)) = originals(this) else { return -1 };
    let orig: GetDeviceDataFn = unsafe { std::mem::transmute(orig) };
    let hr = unsafe { orig(this, obj_size, data, count, flags) };
    if hr < 0 || data.is_null() || count.is_null() || obj_size < 8 {
        return hr;
    }
    // SAFETY: on success `count` holds the number of events written.
    let n = unsafe { *count } as usize;
    if BLOCK.load(Ordering::Relaxed) {
        unsafe { *count = 0 };
        return hr;
    }
    let hk = HOTKEY_VK.load(Ordering::Relaxed);
    if hk == 0 || !hotkey_mods_held() {
        return hr;
    }
    // Drop the hotkey's own key events (dwOfs == scan code), compacting.
    let stride = obj_size as usize;
    let key = dik(hk) as u32;
    let mut kept = 0;
    for i in 0..n {
        let ev = unsafe { data.add(i * stride) };
        if unsafe { *(ev as *const u32) } == key {
            hidden(3);
            continue;
        }
        if kept != i {
            unsafe { std::ptr::copy(ev, data.add(kept * stride), stride) };
        }
        kept += 1;
    }
    unsafe { *count = kept as u32 };
    hr
}

type DirectInputCreateFn = unsafe extern "system" fn(isize, u32, *mut *mut c_void, *mut c_void) -> i32;
type CreateDeviceFn = unsafe extern "system" fn(*mut c_void, *const [u8; 16], *mut *mut c_void, *mut c_void) -> i32;
type EnumDevicesFn = unsafe extern "system" fn(*mut c_void, u32, unsafe extern "system" fn(*const u8, *mut c_void) -> i32, *mut c_void, u32) -> i32;
type ReleaseFn = unsafe extern "system" fn(*mut c_void) -> u32;

// GUID_SysKeyboard {6F1D2B61-D5A0-11CF-BFC7-444553540000}, as bytes.
const GUID_SYS_KEYBOARD: [u8; 16] = [
    0x61, 0x2B, 0x1D, 0x6F, 0xA0, 0xD5, 0xCF, 0x11, 0xBF, 0xC7, 0x44, 0x45, 0x53, 0x54, 0x00, 0x00,
];
const DIDEVTYPE_JOYSTICK: u32 = 4;
const DIEDFL_ATTACHEDONLY: u32 = 1;

unsafe extern "system" fn first_device(inst: *const u8, out: *mut c_void) -> i32 {
    // DIDEVICEINSTANCE: dwSize, then guidInstance.
    unsafe { std::ptr::copy_nonoverlapping(inst.add(4), out as *mut u8, 16) };
    0 // DIENUM_STOP
}

fn vtable_slot(obj: *mut c_void, i: usize) -> usize {
    // SAFETY: obj is a live COM object.
    unsafe { *(*(obj as *const *const usize)).add(i) }
}

/// Hooks DirectInput device reads if the game has dinput.dll loaded.
/// Returns a description for the log.
pub fn install_dinput() -> String {
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
    // SAFETY: COM calls on objects we create; hooks keep the same signatures.
    unsafe {
        let module = GetModuleHandleA(c"dinput.dll".as_ptr().cast());
        if module.is_null() {
            return "no dinput.dll loaded".into();
        }
        let Some(create) = GetProcAddress(module, c"DirectInputCreateA".as_ptr().cast()) else {
            return "dinput.dll has no DirectInputCreateA".into();
        };
        let create: DirectInputCreateFn = std::mem::transmute(create);
        let mut di = std::ptr::null_mut();
        let exe = GetModuleHandleW(std::ptr::null()) as isize;
        if create(exe, 0x0500, &mut di, std::ptr::null_mut()) < 0 || di.is_null() {
            return "DirectInputCreateA failed".into();
        }
        let create_device: CreateDeviceFn = std::mem::transmute(vtable_slot(di, 3));
        let enum_devices: EnumDevicesFn = std::mem::transmute(vtable_slot(di, 4));

        let mut guids = vec![("keyboard", GUID_SYS_KEYBOARD)];
        let mut joy = [0u8; 16];
        if enum_devices(di, DIDEVTYPE_JOYSTICK, first_device, joy.as_mut_ptr().cast(), DIEDFL_ATTACHEDONLY) >= 0 && joy != [0; 16] {
            guids.push(("joystick", joy));
        }

        let _ = MH_Initialize();
        let mut done = Vec::new();
        for (what, guid) in guids {
            let mut dev = std::ptr::null_mut();
            if create_device(di, &guid, &mut dev, std::ptr::null_mut()) < 0 || dev.is_null() {
                done.push(format!("{what}: no device"));
                continue;
            }
            let state_fn = vtable_slot(dev, 9);
            let data_fn = vtable_slot(dev, 10);
            let already = DI_HOOKS.lock().map(|h| h.iter().any(|(f, _, _)| *f == state_fn)).unwrap_or(true);
            if !already {
                let mut orig_state = std::ptr::null_mut();
                let mut orig_data = std::ptr::null_mut();
                let ok = MH_CreateHook(state_fn as _, di_get_device_state as *mut c_void, &mut orig_state).ok().is_ok()
                    && MH_CreateHook(data_fn as _, di_get_device_data as *mut c_void, &mut orig_data).ok().is_ok();
                if ok {
                    if let Ok(mut h) = DI_HOOKS.lock() {
                        h.push((state_fn, orig_state as usize, orig_data as usize));
                    }
                    let _ = MH_EnableHook(state_fn as _);
                    let _ = MH_EnableHook(data_fn as _);
                    done.push(format!("{what}: hooked"));
                } else {
                    done.push(format!("{what}: hook failed"));
                }
            } else {
                done.push(format!("{what}: shares keyboard hook"));
            }
            let release: ReleaseFn = std::mem::transmute(vtable_slot(dev, 2));
            release(dev);
        }
        let release: ReleaseFn = std::mem::transmute(vtable_slot(di, 2));
        release(di);
        format!("DirectInput {}", done.join(", "))
    }
}

// ── Window messages ─────────────────────────────────────────────────────
//
// Some games (RE2: F1 pauses) react to WM_KEYDOWN in their window
// procedure. A message hook on the game's UI thread removes the overlay
// hotkey's key messages before the game sees them.

use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, SetWindowsHookExW, MSG, PM_REMOVE, WH_GETMESSAGE, WM_KEYDOWN, WM_KEYUP, WM_NULL,
    WM_SYSKEYDOWN, WM_SYSKEYUP,
};

static MSG_HOOK_INSTALLED: AtomicBool = AtomicBool::new(false);
/// Hotkey key messages removed so far (diagnostics).
pub static DROPPED_MESSAGES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

unsafe extern "system" fn key_message_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && wparam as u32 == PM_REMOVE {
        // SAFETY: for WH_GETMESSAGE, lparam points at the MSG being retrieved.
        let msg = unsafe { &mut *(lparam as *mut MSG) };
        let is_key = matches!(msg.message, WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP);
        let hk = HOTKEY_VK.load(Ordering::Relaxed) as usize;
        if is_key && hk != 0 && msg.wParam == hk && hotkey_mods_held() {
            msg.message = WM_NULL;
            DROPPED_MESSAGES.fetch_add(1, Ordering::Relaxed);
        }
    }
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

/// Installs the key-message filter on the calling thread (call from the
/// game's render thread, which owns its window). Returns true the first
/// time it is installed.
pub fn install_message_hook() -> bool {
    if MSG_HOOK_INSTALLED.swap(true, Ordering::Relaxed) {
        return false;
    }
    // SAFETY: thread-local hook on our own process; the proc lives in this DLL.
    let h = unsafe { SetWindowsHookExW(WH_GETMESSAGE, Some(key_message_hook), std::ptr::null_mut(), GetCurrentThreadId()) };
    !h.is_null()
}
