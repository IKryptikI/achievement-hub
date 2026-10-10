//! ah-launch: starts a game and loads the Achievement Hub overlay into it.
//!
//! Put ah-launch.exe, overlay.dll and achievement-hub.ini in the game folder
//! and point the game's launcher (Heroic, Steam, a shortcut...) at
//! ah-launch.exe instead of the game.
//!
//! achievement-hub.ini:
//!   launch=RE2Launcher.exe          program to start (relative to this folder)
//!   inject=LeonU.exe,ClaireU.exe    processes to load the overlay into
//!
//! Extra command-line arguments are passed through to the launched program.
//! Runs without a console window; progress goes to ah-launch.log.

#![windows_subsystem = "windows"]

#[path = "../../common/anticheat.rs"]
#[allow(dead_code)]
mod anticheat;

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Write;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
use windows_sys::Win32::System::Memory::{
    VirtualAllocEx, VirtualFreeEx, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
};
use windows_sys::Win32::System::Threading::{
    CreateRemoteThread, GetExitCodeThread, OpenProcess, WaitForSingleObject, PROCESS_CREATE_THREAD,
    PROCESS_QUERY_INFORMATION, PROCESS_VM_OPERATION, PROCESS_VM_READ, PROCESS_VM_WRITE,
};

const CONFIG: &str = "achievement-hub.ini";
const DLL: &str = "overlay.dll";
/// Give a new process a moment to finish loading before injecting.
const SETTLE: Duration = Duration::from_millis(1500);
/// Stop waiting if no target process shows up this long after launch.
const GIVE_UP: Duration = Duration::from_secs(120);

/// PE TimeDateStamp and SizeOfImage of an exe file on disk.
fn exe_build(path: &Path) -> Option<(u32, u32)> {
    let d = fs::read(path).ok()?;
    let rd = |o: usize| d.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    if d.get(0..2)? != b"MZ" {
        return None;
    }
    let pe = rd(0x3C)? as usize;
    if d.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    Some((rd(pe + 8)?, rd(pe + 24 + 56)?))
}

/// (timestamp, image_size) pairs from every [[build]] in games/*.toml: the
/// exact exe builds Achievement Hub has vetted (definitions only exist for
/// single-player games without anti-cheat).
fn known_builds(dir: &Path) -> Vec<(u32, u32)> {
    let num = |v: &str| {
        let v = v.trim();
        v.strip_prefix("0x").map_or_else(|| v.parse().ok(), |h| u32::from_str_radix(&h.replace('_', ""), 16).ok())
    };
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(dir.join("games")) else { return out };
    for e in rd.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "toml")) {
        let text = fs::read_to_string(e.path()).unwrap_or_default();
        let (mut ts, mut size) = (None, None);
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                ts = None;
                size = None;
            }
            if let Some((k, v)) = line.split_once('=') {
                match k.trim() {
                    "timestamp" => ts = num(v),
                    "image_size" => size = num(v),
                    _ => {}
                }
            }
            if let (Some(t), Some(s)) = (ts, size) {
                out.push((t, s));
                ts = None;
                size = None;
            }
        }
    }
    out
}

/// Finds `name` in the game folder (up to 3 levels deep).
fn find_file(dir: &Path, name: &str, depth: u32) -> Option<PathBuf> {
    for e in fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name)) {
            return Some(p);
        }
        if depth > 0 && p.is_dir() {
            if let Some(f) = find_file(&p, name, depth - 1) {
                return Some(f);
            }
        }
    }
    None
}

/// Tells the player why there's no overlay this time (non-blocking).
fn notify(message: String) {
    std::thread::spawn(move || {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_SETFOREGROUND, MB_TOPMOST};
        let text: Vec<u16> = message.encode_utf16().chain([0]).collect();
        let title: Vec<u16> = "Achievement Hub".encode_utf16().chain([0]).collect();
        // SAFETY: valid NUL-terminated strings.
        unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_ICONINFORMATION | MB_TOPMOST | MB_SETFOREGROUND) };
    });
}

struct Log(Option<File>);

impl Log {
    fn line(&mut self, msg: &str) {
        if let Some(f) = &mut self.0 {
            let _ = writeln!(f, "{msg}");
        }
    }
}

struct Config {
    launch: String,
    inject: Vec<String>,
}

fn read_config(dir: &Path) -> Result<Config, String> {
    let text = fs::read_to_string(dir.join(CONFIG)).map_err(|e| format!("can't read {CONFIG}: {e}"))?;
    let mut launch = None;
    let mut inject = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with(['#', ';', '[']) || line.is_empty() {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        match k.trim() {
            "launch" => launch = Some(v.trim().to_string()),
            "inject" => inject = v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            _ => {}
        }
    }
    let launch = launch.ok_or(format!("{CONFIG} has no launch= line"))?;
    if inject.is_empty() {
        return Err(format!("{CONFIG} has no inject= line"));
    }
    Ok(Config { launch, inject })
}

/// (pid, exe name) of every running process.
fn processes() -> Vec<(u32, String)> {
    let mut out = Vec::new();
    // SAFETY: plain Win32 calls with correctly sized structs.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut pe: PROCESSENTRY32W = std::mem::zeroed();
        pe.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut pe) != 0;
        while ok {
            let end = pe.szExeFile.iter().position(|&c| c == 0).unwrap_or(pe.szExeFile.len());
            out.push((pe.th32ProcessID, String::from_utf16_lossy(&pe.szExeFile[..end])));
            ok = Process32NextW(snap, &mut pe) != 0;
        }
        CloseHandle(snap);
    }
    out
}

/// Classic LoadLibraryW injection via a remote thread.
fn inject(pid: u32, dll: &Path) -> Result<(), String> {
    let wide: Vec<u16> = dll.as_os_str().encode_wide().chain([0]).collect();
    let bytes = wide.len() * 2;
    // SAFETY: every handle and allocation is checked and released below.
    unsafe {
        let proc = OpenProcess(
            PROCESS_CREATE_THREAD | PROCESS_QUERY_INFORMATION | PROCESS_VM_OPERATION | PROCESS_VM_WRITE | PROCESS_VM_READ,
            0,
            pid,
        );
        if proc.is_null() {
            return Err("OpenProcess failed".into());
        }
        let result = (|| {
            let remote = VirtualAllocEx(proc, std::ptr::null(), bytes, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
            if remote.is_null() {
                return Err("VirtualAllocEx failed".to_string());
            }
            let r = (|| {
                if WriteProcessMemory(proc, remote, wide.as_ptr().cast(), bytes, std::ptr::null_mut()) == 0 {
                    return Err("WriteProcessMemory failed".to_string());
                }
                // kernel32 is mapped at the same address in every process of
                // the same bitness, so our LoadLibraryW address is valid there.
                let load = GetProcAddress(GetModuleHandleA(c"kernel32.dll".as_ptr().cast()), c"LoadLibraryW".as_ptr().cast())
                    .ok_or("LoadLibraryW not found")?;
                let thread = CreateRemoteThread(proc, std::ptr::null(), 0, Some(std::mem::transmute(load)), remote, 0, std::ptr::null_mut());
                if thread.is_null() {
                    return Err("CreateRemoteThread failed".to_string());
                }
                let waited = WaitForSingleObject(thread, 10_000);
                let mut code = 0u32;
                GetExitCodeThread(thread, &mut code);
                CloseHandle(thread);
                if waited != WAIT_OBJECT_0 {
                    return Err("LoadLibraryW timed out".to_string());
                }
                if code == 0 {
                    return Err("LoadLibraryW returned NULL (DLL failed to load)".to_string());
                }
                Ok(())
            })();
            VirtualFreeEx(proc, remote, 0, MEM_RELEASE);
            r
        })();
        CloseHandle(proc);
        result
    }
}

fn run(dir: &Path, log: &mut Log) -> Result<(), String> {
    let cfg = read_config(dir)?;
    let dll = dir.join(DLL);
    if !dll.exists() {
        return Err(format!("{DLL} not found next to ah-launch.exe"));
    }

    // Never inject into a game with anti-cheat: it could get the player banned.
    let anti_cheat = anticheat::scan_dir(dir);
    if let Some(ac) = anti_cheat {
        log.line(&format!("anti-cheat detected ({ac}); starting the game WITHOUT the overlay"));
        notify(format!(
            "This game uses {ac}.\n\nAchievement Hub never runs in games with anti-cheat, so the game will start without the overlay."
        ));
    }
    let builds = known_builds(dir);

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut child = Command::new(dir.join(&cfg.launch))
        .args(&args)
        .current_dir(dir)
        .spawn()
        .map_err(|e| format!("can't start {}: {e}", cfg.launch))?;
    log.line(&format!("started {} {}", cfg.launch, args.join(" ")));

    let started = Instant::now();
    let mut seen: HashSet<u32> = HashSet::new();
    let mut first_seen: Vec<(u32, String, Instant)> = Vec::new();
    let mut child_done = false;
    let mut ever_targeted = false;

    loop {
        if !child_done {
            if let Ok(Some(status)) = child.try_wait() {
                log.line(&format!("{} exited ({status})", cfg.launch));
                child_done = true;
            }
        }

        let targets: Vec<(u32, String)> = processes()
            .into_iter()
            .filter(|(_, name)| cfg.inject.iter().any(|t| t.eq_ignore_ascii_case(name)))
            .collect();

        for (pid, name) in &targets {
            ever_targeted = true;
            if seen.insert(*pid) {
                log.line(&format!("found {name} (pid {pid})"));
                first_seen.push((*pid, name.clone(), Instant::now()));
            }
        }
        first_seen.retain(|(pid, name, at)| {
            if at.elapsed() < SETTLE {
                return true;
            }
            if anti_cheat.is_some() {
                return false;
            }
            // Only inject into exact builds that have a vetted definition.
            let build = find_file(dir, name, 3).and_then(|p| exe_build(&p));
            if !build.is_some_and(|b| builds.contains(&b)) {
                log.line(&format!("{name}: build {build:x?} has no achievement definition; not injecting"));
                notify(format!("{name} isn't a supported version for Achievement Hub, so the game will run without the overlay."));
                return false;
            }
            match inject(*pid, &dll) {
                Ok(()) => log.line(&format!("overlay loaded into {name} (pid {pid})")),
                Err(e) => log.line(&format!("injecting into {name} (pid {pid}) failed: {e}")),
            }
            false
        });

        // Stay alive while the game runs, so launchers see it as "playing".
        if child_done && targets.is_empty() && first_seen.is_empty() {
            if ever_targeted || started.elapsed() > GIVE_UP {
                break;
            }
        }
        sleep(Duration::from_millis(250));
    }
    log.line("done");
    Ok(())
}

fn main() {
    let dir: PathBuf = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    let mut log = Log(File::create(dir.join("ah-launch.log")).ok());
    if let Err(e) = run(&dir, &mut log) {
        log.line(&format!("error: {e}"));
        std::process::exit(1);
    }
}
