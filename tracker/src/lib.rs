//! Achievement tracker core: find a running game and read its memory.
//!
//! Everything outside `process::{linux,windows}` is platform-independent.

pub mod process;

pub use process::GameProcess;

/// Identifies the exact build of a game executable, so achievement rules
/// written for one version are never applied to another (memory addresses
/// move between builds).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExeVersion {
    /// PE header link timestamp (seconds since 1970).
    pub timestamp: u32,
    /// PE SizeOfImage.
    pub image_size: u32,
}

impl GameProcess {
    pub fn read_u8(&self, addr: u64) -> std::io::Result<u8> {
        let mut b = [0u8; 1];
        self.read(addr, &mut b)?;
        Ok(b[0])
    }

    pub fn read_u16(&self, addr: u64) -> std::io::Result<u16> {
        let mut b = [0u8; 2];
        self.read(addr, &mut b)?;
        Ok(u16::from_le_bytes(b))
    }

    pub fn read_u32(&self, addr: u64) -> std::io::Result<u32> {
        let mut b = [0u8; 4];
        self.read(addr, &mut b)?;
        Ok(u32::from_le_bytes(b))
    }

    /// Reads the PE header of the main executable to identify its build.
    pub fn exe_version(&self) -> std::io::Result<ExeVersion> {
        let base = self.module_base();
        if self.read_u16(base)? != 0x5A4D {
            return Err(bad_data("no MZ header at module base"));
        }
        let pe = base + self.read_u32(base + 0x3C)? as u64;
        if self.read_u32(pe)? != 0x0000_4550 {
            return Err(bad_data("no PE signature"));
        }
        // COFF header: TimeDateStamp at +8; optional header starts at +24,
        // SizeOfImage at optional+56 (same offset for PE32 and PE32+).
        Ok(ExeVersion {
            timestamp: self.read_u32(pe + 8)?,
            image_size: self.read_u32(pe + 24 + 56)?,
        })
    }
}

fn bad_data(msg: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, msg)
}
