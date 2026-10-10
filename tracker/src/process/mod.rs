//! Finding a game process and reading its memory.
//!
//! The public API is identical on every OS; only the backend differs.
//! On Linux, Proton/Wine games are ordinary Linux processes whose memory
//! layout matches Windows, so addresses found on one OS work on the other.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as backend;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as backend;

pub struct GameProcess {
    pid: u32,
    exe_name: String,
    module_base: u64,
    inner: backend::Handle,
}

impl GameProcess {
    /// Finds the first running process whose executable name matches one of
    /// `exe_names` (case-insensitive, e.g. `["LeonU.exe", "ClaireU.exe"]`).
    pub fn find(exe_names: &[&str]) -> Option<GameProcess> {
        let found = backend::find(exe_names)?;
        Some(GameProcess {
            pid: found.pid,
            exe_name: found.exe_name,
            module_base: found.module_base,
            inner: found.handle,
        })
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn exe_name(&self) -> &str {
        &self.exe_name
    }

    /// Address where the main executable is loaded.
    pub fn module_base(&self) -> u64 {
        self.module_base
    }

    /// Fills `buf` with memory starting at `addr` in the game process.
    pub fn read(&self, addr: u64, buf: &mut [u8]) -> std::io::Result<()> {
        backend::read(&self.inner, addr, buf)
    }

    /// Writable memory ranges `(start, end)` of the process, sorted by address.
    pub fn writable_regions(&self) -> Vec<(u64, u64)> {
        backend::writable_regions(&self.inner)
    }

    /// False once the game has exited.
    pub fn is_alive(&self) -> bool {
        backend::is_alive(&self.inner)
    }
}

struct Found {
    pid: u32,
    exe_name: String,
    module_base: u64,
    handle: backend::Handle,
}

fn name_matches(candidate: &str, exe_names: &[&str]) -> Option<String> {
    // Wine paths may use either slash; take the last component.
    let base = candidate.rsplit(['/', '\\']).next().unwrap_or(candidate);
    exe_names
        .iter()
        .find(|n| n.eq_ignore_ascii_case(base))
        .map(|n| n.to_string())
}
