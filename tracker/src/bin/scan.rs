//! Memory scanner for finding where a game stores a value (health, flags...).
//!
//!   scan new [--width 1|2|4] [--all] [EXE...]   snapshot memory, start a search
//!   scan changed | same | inc | dec             keep values that changed / didn't / went up / down
//!   scan eq N | ne N                            keep values equal / not equal to N
//!   scan range LO HI                            keep values between LO and HI (inclusive)
//!   scan show [COUNT]                           list remaining candidates
//!
//! By default only the game exe's own data area is searched (where games of
//! this era keep their state); --all searches all writable memory below 2 GB.
//! State lives in $AH_SCAN_DIR (default ~/achievement-hub/.scan).

use std::fs;
use std::path::PathBuf;
use std::process::exit;
use tracker::GameProcess;

const DEFAULT_EXES: &[&str] = &["LeonU.exe", "ClaireU.exe"];

struct Snapshot {
    regions: Vec<(u64, Vec<u8>)>,
}

impl Snapshot {
    fn take(game: &GameProcess, ranges: &[(u64, u64)]) -> Snapshot {
        let regions = ranges
            .iter()
            .map(|&(start, end)| {
                let mut buf = vec![0u8; (end - start) as usize];
                // Read page by page so one unmapped page doesn't lose the region.
                for (i, page) in buf.chunks_mut(4096).enumerate() {
                    if game.read(start + i as u64 * 4096, page).is_err() {
                        page.fill(0);
                    }
                }
                (start, buf)
            })
            .collect();
        Snapshot { regions }
    }

    fn value(&self, addr: u64, width: usize) -> Option<u32> {
        let i = self.regions.partition_point(|(s, _)| *s <= addr).checked_sub(1)?;
        let (start, data) = &self.regions[i];
        let off = (addr - start) as usize;
        let bytes = data.get(off..off + width)?;
        let mut b = [0u8; 4];
        b[..width].copy_from_slice(bytes);
        Some(u32::from_le_bytes(b))
    }

    fn all_addresses(&self, width: usize) -> impl Iterator<Item = u64> + '_ {
        self.regions.iter().flat_map(move |(start, data)| {
            (0..data.len().saturating_sub(width - 1))
                .step_by(width)
                .map(move |off| start + off as u64)
        })
    }

    fn save(&self, dir: &PathBuf) {
        let mut index = String::new();
        let mut blob = Vec::new();
        for (start, data) in &self.regions {
            index += &format!("{start:x} {:x}\n", data.len());
            blob.extend_from_slice(data);
        }
        fs::write(dir.join("regions"), index).unwrap();
        fs::write(dir.join("snap.bin"), blob).unwrap();
    }

    fn load(dir: &PathBuf) -> Snapshot {
        let index = fs::read_to_string(dir.join("regions")).unwrap_or_else(|_| no_search());
        let blob = fs::read(dir.join("snap.bin")).unwrap_or_else(|_| no_search());
        let mut pos = 0;
        let regions = index
            .lines()
            .map(|l| {
                let (s, n) = l.split_once(' ').unwrap();
                let (s, n) = (u64::from_str_radix(s, 16).unwrap(), usize::from_str_radix(n, 16).unwrap());
                let data = blob[pos..pos + n].to_vec();
                pos += n;
                (s, data)
            })
            .collect();
        Snapshot { regions }
    }

    fn ranges(&self) -> Vec<(u64, u64)> {
        self.regions.iter().map(|(s, d)| (*s, s + d.len() as u64)).collect()
    }
}

fn no_search() -> ! {
    eprintln!("no search in progress; start one with `scan new`");
    exit(1)
}

fn state_dir() -> PathBuf {
    let dir = std::env::var_os("AH_SCAN_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap()).join("achievement-hub/.scan"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn find_game(exes: &[&str]) -> GameProcess {
    GameProcess::find(exes).unwrap_or_else(|| {
        eprintln!("game not running ({})", exes.join(" / "));
        exit(1)
    })
}

fn load_meta(dir: &PathBuf) -> (usize, Vec<String>) {
    let meta = fs::read_to_string(dir.join("meta")).unwrap_or_else(|_| no_search());
    let mut it = meta.split_whitespace();
    let width = it.next().unwrap().parse().unwrap();
    (width, it.map(String::from).collect())
}

fn load_candidates(dir: &PathBuf) -> Option<Vec<u64>> {
    let raw = fs::read(dir.join("cands.bin")).ok()?;
    Some(raw.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect())
}

fn save_candidates(dir: &PathBuf, cands: &[u64]) {
    let raw: Vec<u8> = cands.iter().flat_map(|a| a.to_le_bytes()).collect();
    fs::write(dir.join("cands.bin"), raw).unwrap();
}

fn signed(v: u32, width: usize) -> i64 {
    match width {
        1 => v as u8 as i8 as i64,
        2 => v as u16 as i16 as i64,
        _ => v as i32 as i64,
    }
}

fn cmd_new(args: &[String]) {
    let mut width = 2;
    let mut all = false;
    let mut exes = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--width" => width = it.next().and_then(|w| w.parse().ok()).unwrap_or(2),
            "--all" => all = true,
            other => exes.push(other.to_string()),
        }
    }
    if ![1, 2, 4].contains(&width) {
        eprintln!("--width must be 1, 2 or 4");
        exit(1);
    }
    let exe_refs: Vec<&str> = if exes.is_empty() { DEFAULT_EXES.to_vec() } else { exes.iter().map(String::as_str).collect() };
    let game = find_game(&exe_refs);

    let lo = game.module_base();
    let hi = lo + game.exe_version().map(|v| v.image_size as u64).unwrap_or(0);
    let ranges: Vec<(u64, u64)> = game
        .writable_regions()
        .into_iter()
        .filter(|&(s, e)| if all { e <= 0x8000_0000 } else { s < hi && e > lo })
        .map(|(s, e)| if all { (s, e) } else { (s.max(lo), e.min(hi)) })
        .collect();

    let snap = Snapshot::take(&game, &ranges);
    let dir = state_dir();
    let _ = fs::remove_file(dir.join("cands.bin"));
    snap.save(&dir);
    fs::write(dir.join("meta"), format!("{width} {}", exe_refs.join(" "))).unwrap();
    let total: usize = snap.regions.iter().map(|(_, d)| d.len()).sum();
    println!(
        "new search in {} (pid {}): {} regions, {} KB, {}-byte values",
        game.exe_name(),
        game.pid(),
        snap.regions.len(),
        total / 1024,
        width
    );
}

fn cmd_filter(op: &str, arg: Option<&String>, arg2: Option<&String>) {
    let dir = state_dir();
    let (width, exes) = load_meta(&dir);
    let exe_refs: Vec<&str> = exes.iter().map(String::as_str).collect();
    let game = find_game(&exe_refs);
    let old = Snapshot::load(&dir);
    let new = Snapshot::take(&game, &old.ranges());
    let n: Option<u32> = arg.map(|a| {
        let a = a.trim();
        let parsed = if let Some(h) = a.strip_prefix("0x") { u32::from_str_radix(h, 16).ok() } else { a.parse::<i64>().ok().map(|v| v as u32) };
        parsed.unwrap_or_else(|| {
            eprintln!("not a number: {a}");
            exit(1)
        })
    });
    let hi: Option<i64> = arg2.and_then(|a| a.parse().ok());
    if op == "range" && (n.is_none() || hi.is_none()) {
        eprintln!("usage: scan range LO HI");
        exit(1);
    }
    let mask = if width == 4 { u32::MAX } else { (1u32 << (width * 8)) - 1 };

    let keep = |addr: u64| -> bool {
        let (Some(o), Some(c)) = (old.value(addr, width), new.value(addr, width)) else { return false };
        match op {
            "changed" => o != c,
            "same" => o == c,
            "inc" => signed(c, width) > signed(o, width),
            "dec" => signed(c, width) < signed(o, width),
            "eq" => c == n.unwrap() & mask,
            "ne" => c != n.unwrap() & mask,
            "range" => (signed(n.unwrap(), 4)..=hi.unwrap()).contains(&signed(c, width)),
            _ => unreachable!(),
        }
    };
    let before;
    let cands: Vec<u64> = match load_candidates(&dir) {
        Some(c) => {
            before = c.len();
            c.into_iter().filter(|&a| keep(a)).collect()
        }
        None => {
            before = old.all_addresses(width).count();
            old.all_addresses(width).filter(|&a| keep(a)).collect()
        }
    };
    save_candidates(&dir, &cands);
    new.save(&dir);
    println!("{op}{}: {before} -> {} candidates", n.map(|v| format!(" {}", signed(v, width))).unwrap_or_default(), cands.len());
    if cands.len() <= 20 {
        show(&dir, &game, width, cands.len());
    }
}

fn show(dir: &PathBuf, game: &GameProcess, width: usize, count: usize) {
    let snap = Snapshot::load(dir);
    let Some(cands) = load_candidates(dir) else {
        println!("no filters applied yet; every address is still a candidate");
        return;
    };
    let mut buf = [0u8; 4];
    for &a in cands.iter().take(count) {
        let prev = snap.value(a, width).unwrap_or(0);
        let now = game.read(a, &mut buf[..width]).map(|_| u32::from_le_bytes(buf)).ok();
        let now = now.map(|v| signed(v, width).to_string()).unwrap_or("?".into());
        println!("  {a:#010x}  last {:>6}  now {now:>6}", signed(prev, width));
    }
    if cands.len() > count {
        println!("  ... {} more", cands.len() - count);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("new") => cmd_new(&args[1..]),
        Some(op @ ("changed" | "same" | "inc" | "dec")) => cmd_filter(op, None, None),
        Some(op @ ("eq" | "ne")) => cmd_filter(op, args.get(1), None),
        Some("range") => cmd_filter("range", args.get(1), args.get(2)),
        Some("show") => {
            let dir = state_dir();
            let (width, exes) = load_meta(&dir);
            let exe_refs: Vec<&str> = exes.iter().map(String::as_str).collect();
            let game = find_game(&exe_refs);
            let count = args.get(1).and_then(|c| c.parse().ok()).unwrap_or(30);
            show(&dir, &game, width, count);
        }
        _ => {
            eprintln!("usage: scan new [--width 1|2|4] [--all] [EXE...] | changed | same | inc | dec | eq N | ne N | range LO HI | show [COUNT]");
            exit(1);
        }
    }
}
