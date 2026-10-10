//! Record mode: learning an achievement's memory condition by playing.
//!
//! While a game has achievements nobody has mapped yet, this watches the
//! game's own static memory (the exe's writable sections, where older games
//! keep their progress flags) once a second. A byte that changes twice in
//! quick succession is noise (timers, positions, animation); one that
//! changed recently, and not rapidly, is a candidate. When the player says "I just earned this",
//! the recent candidates are ranked (single bits switching on, counters
//! going up and very recent changes first) and saved per achievement.
//! Recording the same achievement again keeps only candidates that changed
//! the same way both times, so a couple of recordings usually pin it down.
//! Once a single candidate is left it becomes the achievement's condition.

use crate::games::rules::{Condition, Read};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;

/// How far back (seconds) a change still counts as "just earned".
const WINDOW: u32 = 90;
/// Most writable memory we'll watch (keeps the overlay's footprint sane).
const MAX_BYTES: usize = 192 << 20;
/// Most single changes remembered before old ones are dropped.
const MAX_CHANGES: usize = 3_000_000;
const CHUNK: usize = 1 << 20;
/// A byte that changes this many seconds in a row is noise for good (timers,
/// positions, animation). Occasional changes are fine: a counter ticking up
/// a few times, or a reload turning a flag back off before the next
/// recording sees it switch on again.
const STREAK: u8 = 4;

struct Region {
    base: usize,
    prev: Vec<u8>,
    /// One bit per byte: changes too often to be a flag (never a candidate).
    noise: Vec<u64>,
}

#[derive(Clone, Copy)]
struct Change {
    tick: u32,
    old: u8,
    /// Seconds in a row it has changed.
    streak: u8,
}

struct State {
    regions: Vec<Region>,
    /// Address -> its latest change.
    changes: HashMap<usize, Change>,
    tick: u32,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static ACTIVE: AtomicBool = AtomicBool::new(false);
static GAME: Mutex<String> = Mutex::new(String::new());
/// Achievement -> tick of its last recording this session (a new recording
/// only looks at changes after it).
static LAST: Mutex<Option<HashMap<String, u32>>> = Mutex::new(None);
/// (game, achievement) -> (recordings, candidates left), for the UI.
static STATUS: Mutex<Option<HashMap<(String, String), (u32, usize)>>> = Mutex::new(None);

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub struct Candidate {
    pub addr: u32,
    pub size: u8,
    pub old: u32,
    pub new: u32,
}

impl Candidate {
    /// The one bit that switched on, if that's all that happened.
    fn flag_bit(&self) -> Option<u32> {
        let (on, off) = (self.new & !self.old, self.old & !self.new);
        (off == 0 && on.count_ones() == 1).then(|| on.trailing_zeros())
    }

    fn score(&self, age: u32) -> i32 {
        let mut s = 0;
        if self.flag_bit().is_some() {
            s += 4;
        }
        if self.new == self.old.wrapping_add(1) {
            s += 3;
        }
        if self.old == 0 && self.new <= 0xff {
            s += 2;
        }
        if self.new > 0xffff && self.flag_bit().is_none() {
            s -= 2;
        }
        // A lone byte at the top of a 4-byte value is usually a float's
        // exponent (positions, timers): it creeps up by one like a counter.
        if self.size == 1 && self.addr % 4 == 3 {
            s -= 5;
        } else if self.size == 1 && self.addr % 4 != 0 {
            s -= 1;
        }
        s + match age {
            0..=20 => 3,
            21..=45 => 2,
            _ => 0,
        }
    }

    /// Changed the same way as `other` (same bit switched on, or the same
    /// new value), so both recordings agree on it.
    fn is_increment(&self) -> bool {
        self.new == self.old.wrapping_add(1)
    }

    /// `later` (a newer recording) changed the same way as this one: the
    /// same bit switched on, the same counter ticked up further, or the same
    /// new value.
    fn agrees(&self, later: &Candidate) -> bool {
        self.addr == later.addr
            && self.size == later.size
            && if self.is_increment() && later.is_increment() && (self.old > 0 || later.old > 0) {
                // A counter: it must have kept going up.
                later.new > self.new
            } else {
                matches!((self.flag_bit(), later.flag_bit()), (Some(a), Some(b)) if a == b) || self.new == later.new
            }
    }

    /// The condition for this change. `target` is the count the achievement
    /// asks for ("defeat 100 enemies"), used when this looks like a counter.
    /// `base`: added to the (exe-relative) address.
    pub fn condition(&self, target: Option<u32>, base: u32) -> Condition {
        let c = Candidate { addr: self.addr.wrapping_add(base), ..*self };
        c.absolute_condition(target)
    }

    fn absolute_condition(&self, target: Option<u32>) -> Condition {
        let read = match self.size {
            1 => Read::U8,
            2 => Read::U16,
            _ => Read::U32,
        };
        if self.is_increment() && (target.is_some() || self.old == 0) {
            return Condition::memory(read, self.addr as u64, None, None, Some(target.unwrap_or(1) as i64));
        }
        if let Some(bit) = self.flag_bit() {
            let mask = 1i64 << (bit % 8);
            return Condition::memory(Read::U8, (self.addr + bit / 8) as u64, Some(mask), Some(mask), None);
        }
        if self.new > self.old {
            Condition::memory(read, self.addr as u64, None, None, Some(self.new as i64))
        } else {
            Condition::memory(read, self.addr as u64, None, Some(self.new as i64), None)
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Learned {
    /// Exe build (PE timestamp, image size) the addresses belong to.
    build: [u32; 2],
    /// Addresses are relative to the exe's base (older files: absolute).
    #[serde(default)]
    relative: bool,
    recordings: u32,
    /// The count the achievement asks for, if it names one.
    #[serde(default)]
    target: Option<u32>,
    candidates: Vec<Candidate>,
}

/// The count an achievement's description asks for ("Defeat 100 enemies",
/// "Ram 250 Cars"): its first number of 10 or more.
pub fn target_in(description: &str) -> Option<u32> {
    description
        .split(|c: char| !c.is_ascii_digit() && c != ',')
        .filter_map(|w| w.replace(',', "").parse::<u32>().ok())
        .find(|&n| n >= 10)
}

fn file(profile: &Path, game: &str, achievement: &str) -> PathBuf {
    profile.join("mapping").join(game).join(format!("{achievement}.json"))
}

/// Whether record mode is watching this game this session.
pub fn watching(game: &str) -> bool {
    ACTIVE.load(Ordering::Relaxed) && GAME.lock().is_ok_and(|g| *g == game)
}

/// (recordings, candidates left) for an achievement record mode learned.
pub fn status(game: &str, achievement: &str) -> Option<(u32, usize)> {
    STATUS.lock().ok()?.as_ref()?.get(&(game.to_string(), achievement.to_string())).copied()
}

fn set_status(game: &str, achievement: &str, s: (u32, usize)) {
    if let Ok(mut m) = STATUS.lock() {
        m.get_or_insert_with(HashMap::new).insert((game.to_string(), achievement.to_string()), s);
    }
}

/// Conditions learned earlier for this game and exe build.
pub fn load(profile: &Path, game: &str, build: (u32, u32)) -> Vec<(String, Condition)> {
    let Ok(dir) = std::fs::read_dir(profile.join("mapping").join(game)) else { return Vec::new() };
    dir.flatten()
        .filter_map(|e| {
            let path = e.path();
            let id = path.file_stem()?.to_string_lossy().into_owned();
            let l: Learned = serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()?;
            if l.build != [build.0, build.1] {
                return None;
            }
            set_status(game, &id, (l.recordings, l.candidates.len()));
            // Only a pinned-down condition unlocks anything by itself.
            let base = if l.relative { exe_base() as u32 } else { 0 };
            (l.candidates.len() == 1).then(|| (id, l.candidates[0].condition(l.target, base)))
        })
        .collect()
}

/// Where the exe is loaded (it can move between launches when the exe allows
/// relocation, so learned addresses are stored relative to it).
pub fn exe_base() -> usize {
    unsafe { GetModuleHandleW(std::ptr::null()) as usize }
}

/// The exe's writable sections.
fn writable_sections() -> Vec<(usize, usize)> {
    unsafe {
        let base = GetModuleHandleW(std::ptr::null()) as *const u8;
        if base.is_null() {
            return Vec::new();
        }
        let pe = base.add(*(base.add(0x3C) as *const u32) as usize);
        let sections = *(pe.add(6) as *const u16) as usize;
        let table = pe.add(24 + *(pe.add(20) as *const u16) as usize);
        (0..sections)
            .map(|i| table.add(i * 40))
            .filter(|s| *(s.add(36) as *const u32) & 0x8000_0000 != 0)
            .map(|s| (base as usize + *(s.add(12) as *const u32) as usize, *(s.add(8) as *const u32) as usize))
            .filter(|(_, size)| *size > 0)
            .collect()
    }
}

/// Copies watched memory. The exe's sections are always mapped, so this is
/// a plain copy: ReadProcessMemory would go through wineserver under Wine,
/// and copying megabytes a second through it stalls everything else that
/// talks to it (input included).
fn read(addr: usize, buf: &mut [u8]) -> bool {
    unsafe { std::ptr::copy_nonoverlapping(addr as *const u8, buf.as_mut_ptr(), buf.len()) };
    true
}

/// Starts watching (once per session).
pub fn start(game: &str, log: impl Fn(&str) + Send + 'static) {
    if ACTIVE.swap(true, Ordering::Relaxed) {
        return;
    }
    if let Ok(mut g) = GAME.lock() {
        *g = game.to_string();
    }
    std::thread::spawn(move || {
        let mut total = 0;
        let mut regions = Vec::new();
        for (base, size) in writable_sections() {
            let size = size.min(MAX_BYTES - total);
            if size == 0 {
                break;
            }
            let mut prev = vec![0u8; size];
            for (i, c) in prev.chunks_mut(CHUNK).enumerate() {
                read(base + i * CHUNK, c);
            }
            regions.push(Region { base, prev, noise: vec![0; size.div_ceil(64)] });
            total += size;
        }
        log(&format!("record mode: watching {} KB of game memory in {} sections", total / 1024, regions.len()));
        *STATE.lock().unwrap() = Some(State { regions, changes: HashMap::new(), tick: 0 });
        let mut buf = vec![0u8; CHUNK];
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let Ok(mut guard) = STATE.lock() else { return };
            let Some(st) = guard.as_mut() else { return };
            st.tick += 1;
            sample(st, &mut buf);
        }
    });
}

fn sample(st: &mut State, buf: &mut [u8]) {
    let tick = st.tick;
    for r in &mut st.regions {
        let len = r.prev.len();
        let mut off = 0;
        while off < len {
            let n = CHUNK.min(len - off);
            let cur = &mut buf[..n];
            if read(r.base + off, cur) {
                let prev = &mut r.prev[off..off + n];
                // Fast path: compare 8 bytes at a time.
                for w in (0..n).step_by(8) {
                    let end = (w + 8).min(n);
                    if prev[w..end] == cur[w..end] {
                        continue;
                    }
                    for i in w..end {
                        if prev[i] == cur[i] {
                            continue;
                        }
                        let at = off + i;
                        if r.noise[at / 64] & (1 << (at % 64)) == 0 {
                            let addr = r.base + at;
                            match st.changes.get_mut(&addr) {
                                Some(c) if tick - c.tick <= 1 && c.streak + 1 >= STREAK => {
                                    st.changes.remove(&addr);
                                    r.noise[at / 64] |= 1 << (at % 64);
                                }
                                Some(c) => {
                                    let streak = if tick - c.tick <= 1 { c.streak + 1 } else { 1 };
                                    *c = Change { tick, old: prev[i], streak };
                                }
                                None => {
                                    st.changes.insert(addr, Change { tick, old: prev[i], streak: 1 });
                                }
                            }
                        }
                    }
                    prev[w..end].copy_from_slice(&cur[w..end]);
                }
            }
            off += n;
        }
    }
    // Forget old one-off changes (they can't be "just earned" any more).
    if tick % 60 == 0 || st.changes.len() > MAX_CHANGES {
        let keep_from = tick.saturating_sub(if st.changes.len() > MAX_CHANGES { WINDOW } else { 600 });
        st.changes.retain(|_, c| c.tick >= keep_from);
    }
}

/// The current value of a watched byte.
fn current(st: &State, addr: usize) -> Option<u8> {
    st.regions.iter().find(|r| addr >= r.base && addr < r.base + r.prev.len()).map(|r| r.prev[addr - r.base])
}

/// "I just earned this": ranks what changed lately, merges with earlier
/// recordings, saves it, and returns the condition to use now plus how many
/// candidates are left.
pub fn capture(profile: &Path, game: &str, achievement: &str, target: Option<u32>, build: (u32, u32)) -> Result<(Condition, u32, usize), String> {
    let mut found: Vec<(i32, Candidate)> = {
        let guard = STATE.lock().map_err(|_| "record mode stopped")?;
        let st = guard.as_ref().ok_or("record mode is still starting up")?;
        let mut last = LAST.lock().map_err(|_| "record mode stopped")?;
        let last = last.get_or_insert_with(HashMap::new);
        let since = st.tick.saturating_sub(WINDOW).max(last.get(achievement).map_or(0, |t| t + 1));
        last.insert(achievement.to_string(), st.tick);
        let mut recent: Vec<(usize, Change)> = st.changes.iter().filter(|(_, c)| c.tick >= since).map(|(a, c)| (*a, *c)).collect();
        recent.sort_by_key(|(a, _)| *a);
        // Runs of neighbouring bytes that changed together form one value.
        let mut out = Vec::new();
        let mut i = 0;
        while i < recent.len() {
            let (start, first) = recent[i];
            let mut j = i + 1;
            while j < recent.len() && recent[j].0 == start + (j - i) && recent[j].1.tick == first.tick {
                j += 1;
            }
            let run = j - i;
            i = j;
            if run > 4 {
                continue; // a buffer or string being rewritten, not a flag
            }
            let (addr, size) = match run {
                1 => (start, 1),
                2 if start % 2 == 0 => (start, 2),
                _ => (start & !3, 4),
            };
            let (mut old, mut new) = (0u32, 0u32);
            for k in 0..size {
                let a = addr + k;
                let now = current(st, a).unwrap_or(0) as u32;
                let before = st.changes.get(&a).filter(|c| c.tick == first.tick).map_or(now, |c| c.old as u32);
                old |= before << (8 * k);
                new |= now << (8 * k);
            }
            if old == new {
                continue;
            }
            let c = Candidate { addr: (addr - exe_base()) as u32, size: size as u8, old, new };
            out.push((c.score(st.tick - first.tick), c));
        }
        out
    };
    if found.is_empty() {
        return Err("nothing in the game's memory changed just now; try again right after it happens".into());
    }
    found.sort_by_key(|(s, c)| (-s, c.addr));
    found.dedup_by_key(|(_, c)| (c.addr, c.size));
    let fresh: Vec<Candidate> = found.into_iter().take(5000).map(|(_, c)| c).collect();

    let path = file(profile, game, achievement);
    let earlier: Option<Learned> =
        std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).filter(|l: &Learned| l.build == [build.0, build.1] && l.relative);
    let learned = match earlier {
        Some(l) => {
            let both: Vec<Candidate> = fresh.iter().filter(|c| l.candidates.iter().any(|o| o.agrees(c))).copied().collect();
            if both.is_empty() {
                // Nothing in common: the earlier recording was probably off.
                Learned { build: [build.0, build.1], relative: true, recordings: 1, target, candidates: fresh }
            } else {
                Learned { build: [build.0, build.1], relative: true, recordings: l.recordings + 1, target, candidates: both }
            }
        }
        None => Learned { build: [build.0, build.1], relative: true, recordings: 1, target, candidates: fresh },
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(&learned).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    set_status(game, achievement, (learned.recordings, learned.candidates.len()));
    Ok((learned.candidates[0].condition(target, exe_base() as u32), learned.recordings, learned.candidates.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    static mut FLAGS: [u8; 64] = [0; 64];
    static mut NOISE: [u32; 16] = [0; 16];
    static mut COUNTER: u16 = 0;
    static mut TRANSFORMS: u32 = 0;

    fn wait(secs: u64) {
        std::thread::sleep(Duration::from_millis(secs * 1000 + 100));
    }

    fn noisy(n: u32) {
        unsafe {
            for x in (*(&raw mut NOISE)).iter_mut() {
                *x = x.wrapping_add(n);
            }
        }
    }

    #[test]
    fn finds_a_flag_and_confirms_it_after_a_reload() {
        let dir = std::env::temp_dir().join("ah-recorder-test");
        let _ = std::fs::remove_dir_all(&dir);
        start("test", |m| println!("{m}"));
        while STATE.lock().unwrap().is_none() {
            std::thread::sleep(Duration::from_millis(50));
        }
        let b = exe_base() as u32;
        let flag = (&raw const FLAGS) as u32 + 13 - b;
        let noise = (&raw const NOISE) as u32 - b;
        for i in 1..7 {
            noisy(i);
            wait(1);
        }
        // Earned: one flag bit on, plus an unrelated one-off counter.
        unsafe {
            FLAGS[13] |= 0x20;
            COUNTER = 1;
        }
        noisy(7);
        wait(2);
        let (_, recs, left) = capture(&dir, "test", "flag", None, (1, 2)).unwrap();
        let l: Learned = serde_json::from_str(&std::fs::read_to_string(file(&dir, "test", "flag")).unwrap()).unwrap();
        println!("1st: {left} candidates {:x?}", &l.candidates[..left.min(4)]);
        assert_eq!(recs, 1);
        assert!(l.candidates.iter().all(|c| c.addr + (c.size as u32) <= noise || c.addr >= noise + 64));
        assert!(l.candidates.iter().any(|c| c.addr <= flag && flag < c.addr + c.size as u32));

        // Reload a save from before (flag off), then earn it again.
        wait(3);
        unsafe {
            FLAGS[13] = 0;
            COUNTER = 0;
        }
        wait(3);
        unsafe {
            FLAGS[13] |= 0x20;
            COUNTER = 7;
        }
        wait(2);
        let (c, recs, left) = capture(&dir, "test", "flag", None, (1, 2)).unwrap();
        let l: Learned = serde_json::from_str(&std::fs::read_to_string(file(&dir, "test", "flag")).unwrap()).unwrap();
        println!("2nd: {recs} recordings, {left} candidates {:x?}", l.candidates);
        assert_eq!(recs, 2);
        assert!(l.candidates.iter().all(|c| c.addr != (&raw const COUNTER) as u32 - b));
        assert!(l.candidates[0].addr <= flag && flag < l.candidates[0].addr + l.candidates[0].size as u32);
        let mut mem = crate::games::rules::Memory::default();
        assert!(c.holds(&mut mem));

        // A counter bumped a few times in quick succession (transforming
        // three times) is still a candidate, and agrees across recordings
        // even though its value differs.
        let counter = (&raw const TRANSFORMS) as u32 - b;
        for _ in 0..3 {
            unsafe { TRANSFORMS += 1 };
            wait(1);
        }
        capture(&dir, "test", "transform", None, (1, 2)).unwrap();
        wait(3);
        unsafe { TRANSFORMS += 1 };
        wait(1);
        let (c, recs, _) = capture(&dir, "test", "transform", Some(500), (1, 2)).unwrap();
        let l: Learned = serde_json::from_str(&std::fs::read_to_string(file(&dir, "test", "transform")).unwrap()).unwrap();
        println!("counter: {recs} recordings, {:x?}", l.candidates);
        assert_eq!(recs, 2);
        assert!(l.candidates.iter().any(|c| c.addr == counter));
        let pick = l.candidates.iter().find(|c| c.addr == counter).unwrap().condition(Some(500), b);
        assert!(!pick.holds(&mut mem));
        let _ = c;
        assert_eq!(target_in("Ram 250 Cars"), Some(250));
        assert_eq!(target_in("Complete Suburbs Chapter 1 using only melee attacks"), None);
    }
}
