//! Games with achievement sets: the definitions embedded at build time.

use serde::Deserialize;
use std::path::Path;

include!(concat!(env!("OUT_DIR"), "/definitions.rs"));

#[derive(Deserialize, Clone)]
pub struct GameInfo {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub developer: Option<String>,
    #[serde(default)]
    pub year: Option<u16>,
}

#[derive(Deserialize, Clone)]
pub struct Build {
    #[serde(default)]
    pub exe: Option<String>,
    pub timestamp: u32,
    pub image_size: u32,
}

#[derive(Deserialize, Clone)]
pub struct AchievementInfo {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub points: u32,
    #[serde(default)]
    pub when: Vec<toml::Value>,
    #[serde(default)]
    pub steam_api: Option<String>,
    #[serde(default)]
    pub platinum: bool,
}

#[derive(Deserialize, Clone)]
pub struct Definition {
    pub game: GameInfo,
    pub build: Vec<Build>,
    #[serde(default)]
    pub achievement: Vec<AchievementInfo>,
    /// File name, e.g. "re2-classic-gog-leon.toml".
    #[serde(skip)]
    pub file: String,
    #[serde(skip)]
    pub text: String,
}

impl Definition {
    pub fn total_points(&self) -> u32 {
        self.achievement.iter().map(|a| a.points).sum()
    }

    /// Achievements the overlay can currently detect.
    pub fn trackable(&self) -> usize {
        self.achievement.iter().filter(|a| a.platinum || !a.when.is_empty() || a.steam_api.is_some()).count()
    }
}

/// Every definition: the ones downloaded from the online database (see
/// dbsync) win over the ones built into the app, file by file.
pub fn all() -> Vec<Definition> {
    let mut texts: std::collections::BTreeMap<String, String> = DEFINITIONS.iter().map(|(f, t)| (f.to_string(), t.to_string())).collect();
    for e in std::fs::read_dir(crate::dbsync::dir()).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.ends_with(".toml") {
            if let Ok(t) = std::fs::read_to_string(e.path()) {
                texts.insert(name, t);
            }
        }
    }
    texts
        .into_iter()
        .filter_map(|(file, text)| {
            let mut d: Definition = toml::from_str(&text).ok()?;
            d.file = file;
            d.text = text;
            Some(d)
        })
        .collect()
}

/// PE TimeDateStamp, SizeOfImage and 64-bit flag of an exe on disk.
pub fn exe_build(path: &Path) -> Option<(u32, u32, bool)> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut d = vec![0u8; 4096];
    let n = f.read(&mut d).ok()?;
    d.truncate(n);
    let rd16 = |o: usize| d.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let rd32 = |o: usize| d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    if d.get(0..2)? != b"MZ" {
        return None;
    }
    let pe = rd32(0x3C)? as usize;
    if d.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    Some((rd32(pe + 8)?, rd32(pe + 24 + 56)?, rd16(pe + 4)? == 0x8664))
}

/// DLL names an exe imports (lowercase).
pub fn exe_imports(path: &Path) -> Vec<String> {
    let Ok(d) = std::fs::read(path) else { return Vec::new() };
    let rd16 = |o: usize| d.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize);
    let rd32 = |o: usize| d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    (|| {
        let pe = rd32(0x3C)?;
        let opt = pe + 24;
        let x64 = rd16(opt)? == 0x20b;
        let nsec = rd16(pe + 6)?;
        let optsz = rd16(pe + 20)?;
        let imp_rva = rd32(opt + if x64 { 120 } else { 104 })?;
        let secs: Vec<(usize, usize, usize, usize)> = (0..nsec)
            .filter_map(|i| {
                let s = opt + optsz + i * 40;
                Some((rd32(s + 8)?, rd32(s + 12)?, rd32(s + 16)?, rd32(s + 20)?))
            })
            .collect();
        let off = |rva: usize| secs.iter().find(|(vs, va, rs, _)| rva >= *va && rva < va + (*vs).max(*rs)).map(|(_, va, _, rp)| rva - va + rp);
        let mut o = off(imp_rva)?;
        let mut out = Vec::new();
        for _ in 0..256 {
            let name_rva = rd32(o + 12)?;
            if name_rva == 0 {
                break;
            }
            let n = off(name_rva)?;
            let end = d[n..].iter().position(|&b| b == 0)? + n;
            out.push(String::from_utf8_lossy(&d[n..end]).to_lowercase());
            o += 20;
        }
        Some(out)
    })()
    .unwrap_or_default()
}

/// A PE file's data directory `idx` (0 exports, 1 imports) as a file
/// offset, plus a mapper from RVAs to file offsets.
fn pe_dir(d: &[u8], idx: usize) -> Option<(usize, impl Fn(usize) -> Option<usize> + '_)> {
    let rd16 = |o: usize| d.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize);
    let rd32 = |o: usize| d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let pe = rd32(0x3C)?;
    let opt = pe + 24;
    let x64 = rd16(opt)? == 0x20b;
    let (nsec, optsz) = (rd16(pe + 6)?, rd16(pe + 20)?);
    let rva = rd32(opt + if x64 { 112 } else { 96 } + idx * 8)?;
    let secs: Vec<(usize, usize, usize, usize)> = (0..nsec)
        .filter_map(|i| {
            let s = opt + optsz + i * 40;
            Some((rd32(s + 8)?, rd32(s + 12)?, rd32(s + 16)?, rd32(s + 20)?))
        })
        .collect();
    let off = move |rva: usize| secs.iter().find(|(vs, va, rs, _)| rva >= *va && rva < va + (*vs).max(*rs)).map(|(_, va, _, rp)| rva - va + rp);
    Some((off(rva)?, off))
}

fn cstr(d: &[u8], o: usize) -> Option<String> {
    let end = d.get(o..)?.iter().position(|&b| b == 0)? + o;
    Some(String::from_utf8_lossy(&d[o..end]).into_owned())
}

/// Function names an exe imports from `dll` (by name; ordinals skipped).
pub fn imported_functions(exe: &Path, dll: &str) -> Vec<String> {
    let Ok(d) = std::fs::read(exe) else { return Vec::new() };
    let rd = |o: usize| d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    (|| {
        let (mut o, off) = pe_dir(&d, 1)?;
        for _ in 0..256 {
            let name_rva = rd(o + 12)?;
            if name_rva == 0 {
                return Some(Vec::new());
            }
            if cstr(&d, off(name_rva)?)?.eq_ignore_ascii_case(dll) {
                let thunks = rd(o).filter(|&r| r != 0).or(rd(o + 16))?;
                let mut t = off(thunks)?;
                let mut out = Vec::new();
                while let Some(v) = rd(t).filter(|&v| v != 0) {
                    if v & 0x8000_0000 == 0 {
                        out.push(cstr(&d, off(v)? + 2)?);
                    }
                    t += 4;
                }
                return Some(out);
            }
            o += 20;
        }
        None
    })()
    .unwrap_or_default()
}

/// Names a DLL exports.
pub fn exported_names(dll: &Path) -> Vec<String> {
    let Ok(d) = std::fs::read(dll) else { return Vec::new() };
    let rd = |o: usize| d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    (|| {
        let (e, off) = pe_dir(&d, 0)?;
        let names = off(rd(e + 32)?)?;
        (0..rd(e + 24)?.min(65536)).map(|i| cstr(&d, off(rd(names + i * 4)?)?)).collect::<Option<Vec<_>>>()
    })()
    .unwrap_or_default()
}

/// The definitions matching any of these exes (by exact build).
pub fn for_exes<'a>(defs: &'a [Definition], exes: &[std::path::PathBuf]) -> Vec<&'a Definition> {
    let builds: Vec<(u32, u32)> = exes.iter().filter_map(|e| exe_build(e)).map(|(t, s, _)| (t, s)).collect();
    defs.iter().filter(|d| d.build.iter().any(|b| builds.contains(&(b.timestamp, b.image_size)))).collect()
}
