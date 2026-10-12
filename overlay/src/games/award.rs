//! Achievements the game itself awards (Fallout 3's script engine).
//!
//! Bethesda's scripts call `AddAchievement <number>` at the exact moment an
//! achievement is earned. In the Steam release that command does nothing
//! (the Games for Windows Live achievements were taken out), but the
//! scripts still call it. We put our own handler in its slot in the game's
//! command table: it reads the number with the game's own argument reader
//! and remembers it, and conditions with `award = <number>` then hold.

use std::collections::HashSet;
use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Numbers the game has awarded this session.
static AWARDED: Mutex<Option<HashSet<u32>>> = Mutex::new(None);
/// Numbers awarded since the last `take_new` (for the log).
static NEW: Mutex<Vec<u32>> = Mutex::new(Vec::new());
/// The game's argument reader (ExtractArgs), once installed.
static EXTRACT: AtomicUsize = AtomicUsize::new(0);

/// Whether the game awarded this achievement number this session.
pub fn awarded(n: u32) -> bool {
    AWARDED.lock().ok().and_then(|a| a.as_ref().map(|s| s.contains(&n))).unwrap_or(false)
}

/// Numbers the game awarded since the last call (for the log).
pub fn take_new() -> Vec<u32> {
    NEW.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default()
}

/// ExtractArgs(paramInfo, scriptData, opcodeOffsetPtr, thisObj, containingObj,
/// script, eventList, ...outputs): cdecl and variadic; one integer out here.
type ExtractArgs = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut u32, *mut c_void, *mut c_void, *mut c_void, *mut c_void, *mut u32) -> bool;

/// The command's new handler (the engine's command signature, cdecl).
unsafe extern "C" fn add_achievement(
    param_info: *mut c_void,
    script_data: *mut c_void,
    this_obj: *mut c_void,
    containing: *mut c_void,
    script: *mut c_void,
    event_list: *mut c_void,
    result: *mut f64,
    offset: *mut u32,
) -> bool {
    let f = EXTRACT.load(Ordering::Relaxed);
    if f == 0 {
        return true;
    }
    let mut n: u32 = 0;
    // SAFETY: called by the game's script engine with its own valid
    // arguments, which go straight to the game's own argument reader.
    let ok = unsafe {
        if !result.is_null() {
            *result = 0.0;
        }
        std::mem::transmute::<usize, ExtractArgs>(f)(param_info, script_data, offset, this_obj, containing, script, event_list, &mut n)
    };
    if ok && n > 0 && n < 1000 {
        if let Ok(mut a) = AWARDED.lock() {
            a.get_or_insert_with(HashSet::new).insert(n);
        }
        if let Ok(mut v) = NEW.lock() {
            v.push(n);
        }
    }
    true
}

/// Puts our handler in the command table entry named `command`. The table
/// lives in the exe's writable data; an entry is {long name, short name,
/// opcode, help, needs parent (u16), param count (u16), params, execute,
/// parse, ...}. Checks everything before touching anything.
pub fn install(command: &str, extract_args_rva: u64) -> Result<String, String> {
    let base = crate::recorder::exe_base();
    if base == 0 {
        return Err("no exe base".into());
    }
    // SAFETY: reads within the exe's own mapped image (headers give its size).
    let size = unsafe {
        let pe = base + *((base + 0x3c) as *const u32) as usize;
        *((pe + 0x50) as *const u32) as usize
    };
    let image = unsafe { std::slice::from_raw_parts(base as *const u8, size) };
    let mut name = command.as_bytes().to_vec();
    name.push(0);
    let text = image.windows(name.len()).position(|w| w == name.as_slice()).ok_or(format!("{command}: name not found"))?;
    let want = ((base + text) as u32).to_le_bytes();
    let mut at = None;
    for i in (0..size.saturating_sub(0x20)).step_by(4) {
        if image[i..i + 4] == want {
            let u = |o: usize| u32::from_le_bytes(image[i + o..i + o + 4].try_into().unwrap_or_default()) as usize;
            let params = u16::from_le_bytes([image[i + 0x12], image[i + 0x13]]);
            let exec = u(0x18);
            // A real entry: one parameter, and its handler inside the exe.
            if params == 1 && exec > base && exec < base + size {
                at = Some(i);
                break;
            }
        }
    }
    let i = at.ok_or(format!("{command}: command table entry not found"))?;
    let slot = (base + i + 0x18) as *mut usize;
    let extract = base + extract_args_rva as usize;
    if extract_args_rva == 0 || extract >= base + size {
        return Err("extract_args_rva is outside the exe".into());
    }
    // The argument reader starts like a normal function (push ebp; mov ebp, esp).
    if image[extract_args_rva as usize..extract_args_rva as usize + 3] != [0x55, 0x8b, 0xec] {
        return Err("extract_args_rva doesn't look like the argument reader (different game build?)".into());
    }
    EXTRACT.store(extract, Ordering::Relaxed);
    use windows_sys::Win32::System::Memory::{PAGE_EXECUTE_READWRITE, VirtualProtect};
    let mut old = 0u32;
    // SAFETY: `slot` is the execute pointer of the entry found above, in the
    // exe's data; made writable for this one store, then restored.
    unsafe {
        if VirtualProtect(slot as *const _, 4, PAGE_EXECUTE_READWRITE, &mut old) == 0 {
            return Err("couldn't unprotect the command table".into());
        }
        let was = *slot;
        *slot = add_achievement as *const () as usize;
        VirtualProtect(slot as *const _, 4, old, &mut old);
        Ok(format!("{command} handler {:#x} -> ours (entry at rva {:#x})", was - base, i))
    }
}
