//! A stand-in for a Windows DLL the game already loads (winmm.dll or
//! version.dll), placed in the game's folder: every export jumps straight
//! to the real DLL, and once loaded it brings in overlay.dll from the same
//! folder. This attaches Achievement Hub however the game is started
//! (Steam, GOG, Heroic, a shortcut), like ReShade does.

#![allow(static_mut_refs)]

use core::ffi::c_void;
use windows_sys::Win32::Foundation::{HINSTANCE, HMODULE};
use windows_sys::Win32::System::LibraryLoader::{GetModuleFileNameW, GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows_sys::Win32::System::Threading::CreateThread;

include!(concat!(env!("OUT_DIR"), "/names.rs"));

/// Lets the desktop app recognise (and safely remove) its own stand-in.
#[used]
static MARKER: [u8; 24] = *b"AchievementHub-proxy-v1\0";

/// Real addresses, filled in when we're loaded; the stubs jump through it.
static mut AH_TABLE: [usize; NAMES.len()] = [0; NAMES.len()];
static mut OUR_MODULE: HMODULE = core::ptr::null_mut();

core::arch::global_asm!(include_str!(concat!(env!("OUT_DIR"), "/stubs.s")), table = sym AH_TABLE);

/// `a` followed by `b` as a NUL-terminated wide string in `buf`.
fn join(buf: &mut [u16; 600], dir_len: usize, name: &str) -> *const u16 {
    let mut i = dir_len;
    for c in name.encode_utf16() {
        if i + 1 >= buf.len() {
            break;
        }
        buf[i] = c;
        i += 1;
    }
    buf[i] = 0;
    buf.as_ptr()
}

unsafe extern "system" fn load_overlay(_: *mut c_void) -> u32 {
    // overlay.dll next to us.
    let mut buf = [0u16; 600];
    // SAFETY: plain Win32 calls with valid buffers.
    unsafe {
        let n = GetModuleFileNameW(OUR_MODULE, buf.as_mut_ptr(), 512) as usize;
        let dir = buf[..n].iter().rposition(|&c| c == '\\' as u16 || c == '/' as u16).map_or(0, |p| p + 1);
        LoadLibraryW(join(&mut buf, dir, "overlay.dll"));
    }
    0
}

/// # Safety
/// Called by the Windows loader.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllMain(module: HINSTANCE, reason: u32, _: *mut c_void) -> i32 {
    if reason != 1 {
        return 1;
    }
    // SAFETY: loader-time setup; the real DLL is loaded by full path so we
    // never find ourselves, and the overlay is loaded off the loader lock.
    unsafe {
        OUR_MODULE = module;
        let mut buf = [0u16; 600];
        // A system DLL is the real one in the system folder; a DLL the game
        // ships (binkw32) is the game's own copy, renamed <name>_ah.dll.
        let real = if LIB == "binkw32" {
            let n = GetModuleFileNameW(module, buf.as_mut_ptr(), 512) as usize;
            let dir = buf[..n].iter().rposition(|&c| c == '\\' as u16 || c == '/' as u16).map_or(0, |p| p + 1);
            LoadLibraryW(join(&mut buf, dir, "binkw32_ah.dll"))
        } else {
            let n = GetSystemDirectoryW(buf.as_mut_ptr(), 512) as usize;
            buf[n] = '\\' as u16;
            LoadLibraryW(join(&mut buf, n + 1, &format!("{LIB}.dll")))
        };
        if real.is_null() {
            return 0;
        }
        for (i, n) in NAMES.iter().enumerate() {
            AH_TABLE[i] = GetProcAddress(real, n.as_ptr()).map_or(0, |f| f as usize);
        }
        CreateThread(core::ptr::null(), 0, Some(load_overlay), core::ptr::null(), 0, core::ptr::null_mut());
    }
    1
}
