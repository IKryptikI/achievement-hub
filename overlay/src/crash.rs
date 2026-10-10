//! Writes a line to overlay.log when the game crashes: what went wrong and
//! in which module, so a crash can be told apart as the game's or ours.

use std::path::PathBuf;
use std::sync::OnceLock;
use windows_sys::Win32::Foundation::HMODULE;
use windows_sys::Win32::System::Diagnostics::Debug::{EXCEPTION_POINTERS, LPTOP_LEVEL_EXCEPTION_FILTER, SetUnhandledExceptionFilter};
use windows_sys::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW, GetModuleHandleExW,
};

static DIR: OnceLock<PathBuf> = OnceLock::new();
static PREVIOUS: OnceLock<LPTOP_LEVEL_EXCEPTION_FILTER> = OnceLock::new();
static LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The module containing `addr`, and the offset into it.
fn module_of(addr: usize) -> (String, usize) {
    let mut module: HMODULE = std::ptr::null_mut();
    let flags = GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
    if unsafe { GetModuleHandleExW(flags, addr as *const u16, &mut module) } == 0 {
        return ("?".into(), addr);
    }
    let mut buf = [0u16; 260];
    let n = unsafe { GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) } as usize;
    let path = String::from_utf16_lossy(&buf[..n]);
    (path.rsplit(['\\', '/']).next().unwrap_or(&path).to_string(), addr - module as usize)
}

unsafe extern "system" fn on_crash(info: *const EXCEPTION_POINTERS) -> i32 {
    let first = !LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed);
    if let (true, Some(dir), Some(rec)) = (first, DIR.get(), unsafe { info.as_ref() }.and_then(|i| unsafe { i.ExceptionRecord.as_ref() })) {
        let at = rec.ExceptionAddress as usize;
        let (module, offset) = module_of(at);
        let mut msg = format!("CRASH: exception {:#010x} at {module}+{offset:#x}", rec.ExceptionCode as u32);
        if rec.ExceptionCode as u32 == 0xC000_0005 && rec.NumberParameters >= 2 {
            let what = if rec.ExceptionInformation[0] == 0 { "reading" } else { "writing" };
            msg += &format!(" ({what} {:#x})", rec.ExceptionInformation[1]);
        }
        msg += if module.eq_ignore_ascii_case("overlay.dll") || module.eq_ignore_ascii_case("AchievementHub.asi") {
            " - in the Achievement Hub overlay"
        } else {
            " - not in the overlay"
        };
        crate::log(dir, &msg);
    }
    match PREVIOUS.get().copied().flatten() {
        Some(prev) => unsafe { prev(info) },
        None => 0, // EXCEPTION_CONTINUE_SEARCH
    }
}

pub fn install(dir: PathBuf) {
    if DIR.set(dir).is_ok() {
        let prev = unsafe { SetUnhandledExceptionFilter(Some(on_crash)) };
        let _ = PREVIOUS.set(prev);
    }
}

#[cfg(test)]
mod tests {
    /// Crashes on purpose: `cargo test -- --ignored crash_is_logged`, then
    /// check overlay.log in the temp folder.
    #[test]
    #[ignore]
    fn crash_is_logged() {
        let dir = std::env::temp_dir();
        let _ = std::fs::remove_file(dir.join("overlay.log"));
        super::install(dir);
        unsafe { std::ptr::write_volatile(0x10 as *mut u32, 1) };
    }
}
