//! Conditions: a value from game memory compared with a number.
//!
//! The value is either read directly (`{ read = "i16", address = 0x994646,
//! eq = 200 }`) or asked of the game engine, for games whose `[engine]` the
//! overlay understands: a quest's current stage (`{ quest = "MQ01", ge =
//! 100 }`) or the player's rank in a faction (`{ faction = 0x2228F, ge = 3
//! }`; -1 when not a member).

use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Read {
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
}

impl Read {
    fn size(self) -> usize {
        match self {
            Read::U8 | Read::I8 => 1,
            Read::U16 | Read::I16 => 2,
            Read::U32 | Read::I32 => 4,
        }
    }

    fn decode(self, b: [u8; 4]) -> i64 {
        match self {
            Read::U8 => b[0] as i64,
            Read::I8 => b[0] as i8 as i64,
            Read::U16 => u16::from_le_bytes([b[0], b[1]]) as i64,
            Read::I16 => i16::from_le_bytes([b[0], b[1]]) as i64,
            Read::U32 => u32::from_le_bytes(b) as i64,
            Read::I32 => i32::from_le_bytes(b) as i64,
        }
    }
}

#[derive(Deserialize)]
pub struct Condition {
    /// Keys this version doesn't know (from a newer game file): such a
    /// condition never holds, rather than guessing what it meant.
    #[serde(flatten)]
    unknown: HashMap<String, toml::Value>,
    #[serde(default)]
    read: Option<Read>,
    #[serde(default)]
    address: Option<u64>,
    /// Engine query: a quest's current stage, by editor name.
    #[serde(default)]
    quest: Option<String>,
    /// Engine query: holds once this quest (by editor name) is completed;
    /// "A|B" for either of two. Needs no comparison.
    #[serde(default)]
    quest_done: Option<String>,
    /// Engine query: the player's rank in a faction, by form id (the low 24
    /// bits; the load-order byte is ignored).
    #[serde(default)]
    faction: Option<u32>,
    /// Engine query: a player value, "level" or "karma".
    #[serde(default)]
    player: Option<PlayerValue>,
    /// Engine query: a Pip-Boy general stat by name ("creatures_killed",
    /// "locks_picked", ... see MISC_STATS).
    #[serde(default)]
    stat: Option<String>,
    /// With `count`: check `count` values `stride` bytes apart; the
    /// condition holds if any of them matches (e.g. inventory slots).
    #[serde(default)]
    stride: u64,
    #[serde(default = "one")]
    count: u32,
    /// `address` is relative to where the game's exe is loaded (for exes
    /// that move between launches; record mode writes these).
    #[serde(default)]
    relative: bool,
    /// Only these bits of the value count (for flags packed into a byte).
    #[serde(default)]
    mask: Option<i64>,
    /// Compare the value from the previous check instead of the current one.
    #[serde(default)]
    previous: bool,
    eq: Option<i64>,
    ne: Option<i64>,
    lt: Option<i64>,
    le: Option<i64>,
    gt: Option<i64>,
    ge: Option<i64>,
    between: Option<[i64; 2]>,
}

fn one() -> u32 {
    1
}

impl Condition {
    fn what(&self) -> String {
        match (&self.quest, self.faction, self.address) {
            (Some(q), ..) => format!("quest {q}"),
            _ if self.quest_done.is_some() => format!("quest {}", self.quest_done.as_deref().unwrap_or_default()),
            (_, Some(f), _) => format!("faction {f:#x}"),
            (_, _, Some(a)) => format!("{a:#x}"),
            _ => "condition".into(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.unknown.is_empty() {
            return Ok(()); // from a newer game file: never holds (see `holds`)
        }
        let cmps = [self.eq, self.ne, self.lt, self.le, self.gt, self.ge].iter().filter(|c| c.is_some()).count()
            + self.between.is_some() as usize;
        if cmps == 0 && self.quest_done.is_none() {
            return Err(format!("{} has no comparison (eq/ne/lt/le/gt/ge/between)", self.what()));
        }
        let sources = (self.read.is_some() && self.address.is_some()) as u8 + self.quest.is_some() as u8 + self.faction.is_some() as u8 + self.quest_done.is_some() as u8 + self.player.is_some() as u8 + self.stat.is_some() as u8;
        if sources != 1 {
            return Err(format!("{}: give exactly one of read+address, quest, quest_done, faction, player or stat", self.what()));
        }
        if let Some(name) = &self.stat {
            if !MISC_STATS.contains(&name.as_str()) {
                return Err(format!("unknown stat {name} (one of: {})", MISC_STATS.join(", ")));
            }
        }
        if self.count == 0 || (self.count > 1 && self.stride == 0) {
            return Err(format!("{}: count needs a non-zero stride", self.what()));
        }
        Ok(())
    }

    fn cmp_count(&self) -> usize {
        [self.eq, self.ne, self.lt, self.le, self.gt, self.ge].iter().filter(|c| c.is_some()).count() + self.between.is_some() as usize
    }

    fn matches(&self, v: i64) -> bool {
        self.eq.is_none_or(|x| v == x)
            && self.ne.is_none_or(|x| v != x)
            && self.lt.is_none_or(|x| v < x)
            && self.le.is_none_or(|x| v <= x)
            && self.gt.is_none_or(|x| v > x)
            && self.ge.is_none_or(|x| v >= x)
            && self.between.is_none_or(|[lo, hi]| (lo..=hi).contains(&v))
    }

    pub fn holds(&self, mem: &mut Memory) -> bool {
        if !self.unknown.is_empty() {
            return false;
        }
        if let Some(q) = &self.quest {
            return mem.quest_stage(q).is_some_and(|v| self.matches(v));
        }
        if let Some(q) = &self.quest_done {
            // "A|B": either quest (games with alternative versions of one).
            return q.split('|').any(|q| mem.quest_completed(q).is_some_and(|done| done && (self.cmp_count() == 0 || self.matches(1))));
        }
        if let Some(f) = self.faction {
            return mem.faction_rank(f).is_some_and(|v| self.matches(v));
        }
        if let Some(p) = self.player {
            return mem.player_value(p).is_some_and(|v| self.matches(v));
        }
        if let Some(name) = &self.stat {
            return mem.misc_stat(name).is_some_and(|v| self.matches(v));
        }
        let (Some(read), Some(address)) = (self.read, self.address) else { return false };
        let address = if self.relative { address + crate::recorder::exe_base() as u64 } else { address };
        (0..self.count as u64).any(|i| {
            let addr = address + i * self.stride;
            let v = if self.previous { mem.previous(read, addr) } else { mem.current(read, addr) };
            v.is_some_and(|v| self.matches(self.mask.map_or(v, |m| v & m)))
        })
    }

    /// A plain memory condition (record mode's findings).
    pub fn memory(read: Read, address: u64, mask: Option<i64>, eq: Option<i64>, ge: Option<i64>) -> Condition {
        Condition {
            unknown: HashMap::new(),
            read: Some(read),
            address: Some(address),
            quest: None,
            quest_done: None,
            faction: None,
            player: None,
            stat: None,
            stride: 0,
            count: 1,
            relative: false,
            mask,
            previous: false,
            eq,
            ne: None,
            lt: None,
            le: None,
            gt: None,
            ge,
            between: None,
        }
    }

    pub fn quest_name(&self) -> Option<&str> {
        self.quest.as_deref().or(self.quest_done.as_deref())
    }

    /// Whether this asks the engine (needs the game's `[engine]` block).
    pub fn uses_engine(&self) -> bool {
        self.quest.is_some() || self.quest_done.is_some() || self.faction.is_some() || self.player.is_some() || self.stat.is_some()
    }
}

/// Where a supported engine keeps what engine conditions read (from the
/// definition's `[engine]` block).
#[derive(Deserialize, Clone, Copy, Debug)]
pub struct Engine {
    /// "gamebryo-oblivion" for now.
    #[allow(dead_code)]
    pub kind: EngineKind,
    /// Address of the pointer to the player character.
    #[serde(default)]
    pub player: u64,
    /// Address of the pointer to the data handler (lists of all forms).
    #[serde(default)]
    pub data_handler: u64,
    /// Fallout 3: where the pointer to the player sits, relative to the
    /// exe's base.
    #[serde(default)]
    pub player_rva: u64,
    /// Fallout 3: the table of pointers to the Pip-Boy's general stats,
    /// relative to the exe's base (each points 4 bytes before its value).
    #[serde(default)]
    pub misc_stats_rva: u64,
    /// Fallout 3: the quest vtable, relative to the exe's base. Quests are
    /// found by scanning memory for it (the exe moves between launches).
    #[serde(default)]
    pub quest_vtable: u64,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineKind {
    #[serde(rename = "gamebryo-oblivion")]
    Oblivion,
    #[serde(rename = "gamebryo-fallout3")]
    Fallout3,
}

impl EngineKind {
    /// (stage, editor name, flags) offsets in a quest.
    fn quest_layout(self) -> (u64, u64, u64) {
        match self {
            EngineKind::Oblivion => (0x5C, 0x60, 0x3C),
            EngineKind::Fallout3 => (0x60, 0x64, 0x3C),
        }
    }
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum PlayerValue {
    Level,
    /// Fallout 3: -1000..1000; good is 250 or more, evil -250 or less.
    Karma,
}

/// Fallout 3's Pip-Boy general stats, in the game's order.
pub const MISC_STATS: [&str; 26] = [
    "quests_completed",
    "locations_discovered",
    "people_killed",
    "creatures_killed",
    "locks_picked",
    "computers_hacked",
    "stimpaks_taken",
    "rad_x_taken",
    "radaway_taken",
    "chems_taken",
    "times_addicted",
    "mines_disarmed",
    "speech_successes",
    "pockets_picked",
    "pants_exploded",
    "books_read",
    "bobbleheads_found",
    "weapons_created",
    "people_mezzed",
    "captives_rescued",
    "sandman_kills",
    "paralyzing_punches",
    "robots_disabled",
    "contracts_completed",
    "corpses_eaten",
    "mysterious_stranger_visits",
];

/// Quest flag: completed.
const QUEST_COMPLETED: i64 = 0x02;

/// Fallout 3 quests found by a background scan: editor name -> address.
static FO3_QUESTS: std::sync::Mutex<Option<HashMap<String, u64>>> = std::sync::Mutex::new(None);
static FO3_SCANNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static FO3_LAST_SCAN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Finds every quest by its vtable in the game's private memory (in a
/// background thread; the result appears in FO3_QUESTS).
fn scan_fo3_quests(vtable: u32, editor_at: u64) {
    use std::sync::atomic::Ordering;
    use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_PRIVATE, MEMORY_BASIC_INFORMATION, PAGE_READWRITE, VirtualQuery};
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    if FO3_SCANNING.swap(true, Ordering::Relaxed) || now < FO3_LAST_SCAN.load(Ordering::Relaxed) + 20 {
        FO3_SCANNING.store(false, Ordering::Relaxed);
        return;
    }
    FO3_LAST_SCAN.store(now, Ordering::Relaxed);
    std::thread::spawn(move || {
        let mut found = HashMap::new();
        let mut addr = 0x10000usize;
        while addr < 0x7FFF_0000 {
            let mut mbi: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
            if unsafe { VirtualQuery(addr as *const _, &mut mbi, std::mem::size_of::<MEMORY_BASIC_INFORMATION>()) } == 0 {
                break;
            }
            let (start, size) = (mbi.BaseAddress as usize, mbi.RegionSize);
            if mbi.State == MEM_COMMIT && mbi.Type == MEM_PRIVATE && mbi.Protect == PAGE_READWRITE {
                let mut a = start;
                while a + 0x80 <= start + size {
                    // Page still there? (The game can free memory mid-scan.)
                    if a % 0x1000 == 0 && unsafe { windows_sys::Win32::System::Memory::IsBadReadPtr(a as *const _, 0x1000) } != 0 {
                        a += 0x1000;
                        continue;
                    }
                    // SAFETY: inside a committed read/write region.
                    if unsafe { *(a as *const u32) } == vtable && unsafe { *((a + 4) as *const u8) } == 0x47 {
                        if let Some(name) = read_string(a as u64 + editor_at) {
                            found.insert(name, a as u64);
                        }
                    }
                    a += 4;
                }
            }
            addr = start + size.max(0x1000);
        }
        if let Ok(mut q) = FO3_QUESTS.lock() {
            *q = Some(found);
        }
        FO3_SCANNING.store(false, Ordering::Relaxed);
    });
}

/// A game string { char*, u16 len } read directly.
fn read_string(at: u64) -> Option<String> {
    let data = read_own(Read::U32, at)? as u64;
    let len = read_own(Read::U16, at + 4)? as usize;
    if data < 0x10000 || len == 0 || len > 128 {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..len as u64).map(|i| read_own(Read::U8, data + i).map(|b| b as u8)).collect();
    String::from_utf8(bytes?).ok()
}

// Oblivion 1.2.0416 layouts (see xOBSE's GameForms.h / GameObjects.h).
const DH_QUESTS: u64 = 0x84; // tList<TESQuest>
const FORM_ID: u64 = 0x0C;
const REFR_BASE_FORM: u64 = 0x1C;
const NPC_FACTIONS: u64 = 0x24 + 0x18; // TESActorBase::actorBaseData.factionList
const FACTION_RANK: u64 = 0x04; // FactionListData { TESFaction*, i8 rank }

/// Reads game memory, caching each value for one poll and remembering the
/// previous poll's values for `previous = true` conditions.
#[derive(Default)]
pub struct Memory {
    current: HashMap<(Read, u64), Option<i64>>,
    last: HashMap<(Read, u64), Option<i64>>,
    pub engine: Option<Engine>,
    /// Quest editor name -> the quest's address (found once; forms stay
    /// put for the session).
    quests: HashMap<String, u64>,
    quests_scanned: bool,
}

impl Memory {
    pub fn with_engine(engine: Option<Engine>) -> Memory {
        Memory { engine, ..Memory::default() }
    }

    pub fn begin_poll(&mut self) {
        self.current.clear();
    }

    pub fn end_poll(&mut self) {
        // Values read this poll become "previous"; keep older ones for
        // addresses that weren't read this time.
        for (k, v) in self.current.drain() {
            self.last.insert(k, v);
        }
    }

    fn ptr(&mut self, addr: u64) -> Option<u64> {
        self.current(Read::U32, addr).map(|v| v as u64).filter(|&p| p >= 0x10000)
    }

    /// A singly linked tList's items, safely (bounded, null-terminated).
    fn list(&mut self, head: u64) -> Vec<u64> {
        let mut out = Vec::new();
        let mut node = head;
        for _ in 0..20_000 {
            if let Some(item) = self.ptr(node) {
                out.push(item);
            }
            match self.ptr(node + 4) {
                Some(next) => node = next,
                None => break,
            }
        }
        out
    }

    fn string(&mut self, at: u64) -> Option<String> {
        let data = self.ptr(at)?;
        let len = self.current(Read::U16, at + 4)? as usize;
        if len == 0 || len > 128 {
            return None;
        }
        let mut s = Vec::with_capacity(len);
        for i in 0..len as u64 {
            s.push(read_own(Read::U8, data + i)? as u8);
        }
        String::from_utf8(s).ok()
    }

    /// Current stage of the quest with this editor name.
    pub fn quest_stage(&mut self, name: &str) -> Option<i64> {
        let q = self.quest(name)?;
        let (stage, _, _) = self.engine?.kind.quest_layout();
        self.current(Read::U8, q + stage)
    }

    /// Whether the quest with this editor name is completed.
    pub fn quest_completed(&mut self, name: &str) -> Option<bool> {
        let q = self.quest(name)?;
        let (_, _, flags) = self.engine?.kind.quest_layout();
        self.current(Read::U8, q + flags).map(|f| f & QUEST_COMPLETED != 0)
    }

    /// The quest with this editor name.
    fn quest(&mut self, name: &str) -> Option<u64> {
        let engine = self.engine?;
        let (_, editor_at, _) = engine.kind.quest_layout();
        if engine.kind == EngineKind::Fallout3 {
            let vtable = crate::recorder::exe_base() as u64 + engine.quest_vtable;
            if self.quests.is_empty() {
                // One scan per session, once the game has loaded its data
                // (an empty result before that is retried).
                match FO3_QUESTS.lock().ok().and_then(|q| q.clone()) {
                    Some(map) if !map.is_empty() => self.quests = map,
                    _ => {
                        scan_fo3_quests(vtable as u32, editor_at);
                        return None;
                    }
                }
            }
            let q = *self.quests.get(name)?;
            // Still that quest? If not, the forms moved: look again.
            if read_own(Read::U32, q)? as u64 != vtable {
                self.quests.clear();
                if let Ok(mut m) = FO3_QUESTS.lock() {
                    *m = None;
                }
                return None;
            }
            return Some(q);
        }
        if !self.quests_scanned || !self.quests.contains_key(name) {
            // (Re)build the name -> address map; a miss before the game has
            // loaded its data just tries again next poll.
            let dh = self.ptr(engine.data_handler)?;
            let quests = self.list(dh + DH_QUESTS);
            if quests.is_empty() {
                return None;
            }
            let mut map = HashMap::new();
            for q in quests {
                if let Some(n) = self.string(q + editor_at) {
                    map.insert(n, q);
                }
            }
            self.quests = map;
            self.quests_scanned = true;
        }
        self.quests.get(name).copied()
    }

    /// The player's level or karma (Fallout 3), or None while no game is
    /// loaded.
    pub fn player_value(&mut self, which: PlayerValue) -> Option<i64> {
        let engine = self.engine?;
        if engine.kind != EngineKind::Fallout3 || engine.player_rva == 0 {
            return None;
        }
        let player = self.ptr(crate::recorder::exe_base() as u64 + engine.player_rva)?;
        match which {
            // TESActorBaseData::level in the player's base form.
            PlayerValue::Level => {
                let base = self.ptr(player + REFR_BASE_FORM)?;
                self.current(Read::U16, base + 0x3C).filter(|&l| (1..=100).contains(&l))
            }
            // The karma actor value, a float on the player.
            PlayerValue::Karma => {
                let bits = self.current(Read::U32, player + 0x380)? as u32;
                let k = f32::from_bits(bits);
                (k.is_finite() && (-1000.0..=1000.0).contains(&k)).then_some(k.round() as i64)
            }
        }
    }

    /// A Pip-Boy general stat (Fallout 3).
    pub fn misc_stat(&mut self, name: &str) -> Option<i64> {
        let engine = self.engine?;
        if engine.misc_stats_rva == 0 {
            return None;
        }
        let i = MISC_STATS.iter().position(|s| *s == name)? as u64;
        let entry = self.ptr(crate::recorder::exe_base() as u64 + engine.misc_stats_rva + 4 * i)?;
        self.current(Read::U32, entry + 4).filter(|&v| v < 1_000_000)
    }

    /// The player's rank in a faction (-1 if not a member), or None while
    /// no game is loaded.
    pub fn faction_rank(&mut self, faction: u32) -> Option<i64> {
        let engine = self.engine?;
        let player = self.ptr(engine.player)?;
        let base = self.ptr(player + REFR_BASE_FORM)?;
        for data in self.list(base + NPC_FACTIONS) {
            let Some(f) = self.ptr(data) else { continue };
            if self.current(Read::U32, f + FORM_ID).is_some_and(|id| id as u32 & 0x00FF_FFFF == faction & 0x00FF_FFFF) {
                return self.current(Read::I8, data + FACTION_RANK);
            }
        }
        Some(-1)
    }

    /// Every faction the player is in, as (form id, rank) (for logging).
    pub fn player_factions(&mut self) -> Vec<(u32, i64)> {
        let Some(engine) = self.engine else { return Vec::new() };
        let Some(player) = self.ptr(engine.player) else { return Vec::new() };
        let Some(base) = self.ptr(player + REFR_BASE_FORM) else { return Vec::new() };
        let mut out = Vec::new();
        for data in self.list(base + NPC_FACTIONS) {
            if let (Some(f), Some(rank)) = (self.ptr(data), self.current(Read::I8, data + FACTION_RANK)) {
                if let Some(id) = self.current(Read::U32, f + FORM_ID) {
                    out.push((id as u32, rank));
                }
            }
        }
        out
    }

    pub fn current(&mut self, read: Read, addr: u64) -> Option<i64> {
        *self.current.entry((read, addr)).or_insert_with(|| read_own(read, addr))
    }

    /// Value at the previous poll. Also reads the current value so it is
    /// available as "previous" next time.
    pub fn previous(&mut self, read: Read, addr: u64) -> Option<i64> {
        self.current(read, addr);
        self.last.get(&(read, addr)).copied().flatten()
    }
}

/// Reads from our own process without risking an access violation: a bad
/// address in a definition file just makes the condition false.
/// (IsBadReadPtr checks in-process; ReadProcessMemory would be a wineserver
/// round trip per value under Wine.)
pub fn read_own(read: Read, addr: u64) -> Option<i64> {
    use windows_sys::Win32::System::Memory::IsBadReadPtr;
    let p = addr as usize as *const u8;
    if addr < 0x10000 || addr > 0xFFFF_FFFF || unsafe { IsBadReadPtr(p.cast(), read.size()) } != 0 {
        return None;
    }
    let mut b = [0u8; 4];
    // SAFETY: just checked readable.
    unsafe { std::ptr::copy_nonoverlapping(p, b.as_mut_ptr(), read.size()) };
    Some(read.decode(b))
}
