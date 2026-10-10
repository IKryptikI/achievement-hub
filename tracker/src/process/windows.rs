//! Windows backend: Toolhelp snapshots to find the process and its main
//! module, ReadProcessMemory to read it.

use super::{name_matches, Found};
use std::io;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_TIMEOUT};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Module32FirstW, Module32NextW, Process32FirstW, Process32NextW,
    MODULEENTRY32W, PROCESSENTRY32W, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Memory::{
    VirtualQueryEx, MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_EXECUTE_READWRITE,
    PAGE_EXECUTE_WRITECOPY, PAGE_GUARD, PAGE_READWRITE, PAGE_WRITECOPY,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
    PROCESS_SYNCHRONIZE,
};

pub struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: we own this handle from OpenProcess.
        unsafe { CloseHandle(self.0) };
    }
}

fn wide_to_string(w: &[u16]) -> String {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

pub fn find(exe_names: &[&str]) -> Option<Found> {
    // SAFETY: plain Win32 calls with correctly sized structs.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut pe: PROCESSENTRY32W = std::mem::zeroed();
        pe.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut result = None;
        let mut ok = Process32FirstW(snap, &mut pe) != 0;
        while ok {
            if let Some(exe_name) = name_matches(&wide_to_string(&pe.szExeFile), exe_names) {
                let pid = pe.th32ProcessID;
                if let Some(module_base) = module_base(pid, &exe_name) {
                    let h = OpenProcess(
                        PROCESS_VM_READ | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                        0,
                        pid,
                    );
                    if !h.is_null() {
                        result = Some(Found { pid, exe_name, module_base, handle: Handle(h) });
                        break;
                    }
                }
            }
            ok = Process32NextW(snap, &mut pe) != 0;
        }
        CloseHandle(snap);
        result
    }
}

fn module_base(pid: u32, exe_name: &str) -> Option<u64> {
    // SAFETY: as above.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid);
        if snap == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut me: MODULEENTRY32W = std::mem::zeroed();
        me.dwSize = std::mem::size_of::<MODULEENTRY32W>() as u32;
        let mut result = None;
        let mut ok = Module32FirstW(snap, &mut me) != 0;
        while ok {
            if wide_to_string(&me.szModule).eq_ignore_ascii_case(exe_name) {
                result = Some(me.modBaseAddr as u64);
                break;
            }
            ok = Module32NextW(snap, &mut me) != 0;
        }
        CloseHandle(snap);
        result
    }
}

pub fn read(h: &Handle, addr: u64, buf: &mut [u8]) -> io::Result<()> {
    let mut n = 0usize;
    // SAFETY: `buf` is valid for `buf.len()` bytes; `n` receives the count.
    let ok = unsafe {
        ReadProcessMemory(h.0, addr as *const _, buf.as_mut_ptr().cast(), buf.len(), &mut n)
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    if n != buf.len() {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "partial read"));
    }
    Ok(())
}

pub fn writable_regions(h: &Handle) -> Vec<(u64, u64)> {
    let writable = PAGE_READWRITE | PAGE_WRITECOPY | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY;
    let mut out = Vec::new();
    let mut addr = 0usize;
    // SAFETY: VirtualQueryEx fills a correctly sized struct.
    unsafe {
        let mut mbi: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
        while VirtualQueryEx(h.0, addr as *const _, &mut mbi, std::mem::size_of_val(&mbi)) != 0 {
            let start = mbi.BaseAddress as usize;
            let end = start + mbi.RegionSize;
            if mbi.State == MEM_COMMIT && mbi.Protect & writable != 0 && mbi.Protect & PAGE_GUARD == 0 {
                out.push((start as u64, end as u64));
            }
            if end <= addr {
                break;
            }
            addr = end;
        }
    }
    out
}

pub fn is_alive(h: &Handle) -> bool {
    // SAFETY: valid process handle opened with SYNCHRONIZE.
    unsafe { WaitForSingleObject(h.0, 0) == WAIT_TIMEOUT }
}
