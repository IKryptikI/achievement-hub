//! Linux backend. Proton/Wine runs each Windows exe as a normal Linux
//! process, so we find it via /proc and read it with process_vm_readv.
//! Requires the same user and kernel.yama.ptrace_scope = 0 (Fedora default).

use super::{name_matches, Found};
use std::fs;
use std::io;

pub struct Handle {
    pid: libc::pid_t,
}

pub fn find(exe_names: &[&str]) -> Option<Found> {
    for entry in fs::read_dir("/proc").ok()?.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Some(exe_name) = exe_name_of(pid, exe_names) else {
            continue;
        };
        if let Some(module_base) = module_base(pid, &exe_name) {
            return Some(Found {
                pid,
                exe_name,
                module_base,
                handle: Handle { pid: pid as libc::pid_t },
            });
        }
    }
    None
}

/// Wine sets the Linux process name to the Windows exe name, and argv[0]
/// to its Windows path; check both.
fn exe_name_of(pid: u32, exe_names: &[&str]) -> Option<String> {
    if let Ok(comm) = fs::read_to_string(format!("/proc/{pid}/comm")) {
        if let Some(n) = name_matches(comm.trim(), exe_names) {
            return Some(n);
        }
    }
    let cmdline = fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let argv0 = cmdline.split(|&b| b == 0).next()?;
    name_matches(&String::from_utf8_lossy(argv0), exe_names)
}

/// Lowest address at which the exe file itself is mapped.
fn module_base(pid: u32, exe_name: &str) -> Option<u64> {
    let maps = fs::read_to_string(format!("/proc/{pid}/maps")).ok()?;
    maps.lines()
        .filter(|line| {
            line.split_whitespace()
                .nth(5)
                .and_then(|path| line[line.find(path)?..].rsplit('/').next())
                .is_some_and(|file| file.eq_ignore_ascii_case(exe_name))
        })
        .filter_map(|line| u64::from_str_radix(line.split('-').next()?, 16).ok())
        .min()
}

pub fn read(h: &Handle, addr: u64, buf: &mut [u8]) -> io::Result<()> {
    let local = libc::iovec {
        iov_base: buf.as_mut_ptr().cast(),
        iov_len: buf.len(),
    };
    let remote = libc::iovec {
        iov_base: addr as *mut libc::c_void,
        iov_len: buf.len(),
    };
    // SAFETY: `local` points at `buf`, which is valid for `buf.len()` bytes;
    // the remote address is only dereferenced by the kernel.
    let n = unsafe { libc::process_vm_readv(h.pid, &local, 1, &remote, 1, 0) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    if n as usize != buf.len() {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "partial read"));
    }
    Ok(())
}

pub fn writable_regions(h: &Handle) -> Vec<(u64, u64)> {
    let Ok(maps) = fs::read_to_string(format!("/proc/{}/maps", h.pid)) else {
        return Vec::new();
    };
    maps.lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            let (range, perms) = (f.next()?, f.next()?);
            if !perms.contains('w') {
                return None;
            }
            let (a, b) = range.split_once('-')?;
            Some((u64::from_str_radix(a, 16).ok()?, u64::from_str_radix(b, 16).ok()?))
        })
        .collect()
}

pub fn is_alive(h: &Handle) -> bool {
    fs::metadata(format!("/proc/{}", h.pid)).is_ok()
}
