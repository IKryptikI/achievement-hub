//! Overlay hotkeys: a keyboard shortcut and a controller button combo, both
//! configurable in achievement-hub.ini:
//!
//!   hotkey=shift+f1          modifiers (shift/ctrl/alt) + one key
//!   pad_hotkey=guide         XInput buttons that must be held together
//!                            ("guide" = the Xbox button)
//!
//! Polled once per frame from the render thread.

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

/// Controller state for the UI (buttons from all pads, strongest left stick).
pub static PAD_BUTTONS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
pub static PAD_LX: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
pub static PAD_LY: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
/// Left/right presses (stick, D-pad, arrow keys) waiting for the panel:
/// they switch blades.
pub static HNAV: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
/// Bumped whenever the controller or arrow keys are used (the overlay hides
/// the mouse pointer until the mouse moves again).
pub static PAD_ACTIVITY: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// Last XInput result code per controller slot (diagnostics; 0 = connected,
/// 1167 = not connected).
pub static PAD_CODES: [std::sync::atomic::AtomicU32; 4] = [
    std::sync::atomic::AtomicU32::new(u32::MAX),
    std::sync::atomic::AtomicU32::new(u32::MAX),
    std::sync::atomic::AtomicU32::new(u32::MAX),
    std::sync::atomic::AtomicU32::new(u32::MAX),
];

/// Set while the panel handles B/Esc itself (e.g. "back" from a game's
/// achievement list) instead of closing.
pub static MODAL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub const DEFAULT_KEY: &str = "shift+f2";
/// Alternatives separated by `|`; any one opens the panel.
pub const DEFAULT_PAD: &str = "guide|back+start";

/// Used instead when the Xbox button can't be read on this system.
const FALLBACK_PAD: &str = "back+start";
const GUIDE: u16 = 0x0400;

// XInput button bits.
const BUTTON_NAMES: &[(&str, u16)] = &[
    ("up", 0x0001), ("down", 0x0002), ("left", 0x0004), ("right", 0x0008),
    ("start", 0x0010), ("back", 0x0020), ("select", 0x0020),
    ("ls", 0x0040), ("rs", 0x0080), ("lb", 0x0100), ("rb", 0x0200),
    ("guide", GUIDE), ("xbox", GUIDE), ("home", GUIDE),
    ("a", 0x1000), ("b", 0x2000), ("x", 0x4000), ("y", 0x8000),
];

#[repr(C)]
#[derive(Default)]
struct XInputState {
    packet: u32,
    buttons: u16,
    left_trigger: u8,
    right_trigger: u8,
    thumbs: [i16; 4],
}

type XInputGetState = unsafe extern "system" fn(u32, *mut XInputState) -> u32;

pub struct Hotkeys {
    key_mods: Vec<VIRTUAL_KEY>,
    key: Option<VIRTUAL_KEY>,
    /// Button combos (any one opens the panel).
    pad_masks: Vec<u16>,
    /// Also accept the Home key (the Xbox button over Bluetooth).
    guide_key: bool,
    xinput: Option<XInputGetState>,
    key_was_down: bool,
    pad_was_down: bool,
    esc_was_down: bool,
    b_was_down: bool,
    /// Up/down navigation: direction held and when it next repeats.
    nav_held: i32,
    nav_next: Option<std::time::Instant>,
    /// Left/right, the same way (slower repeat).
    hnav_held: i32,
    hnav_next: Option<std::time::Instant>,
    /// Controller slots known to be connected; empty slots are re-checked
    /// only once a second because XInput is slow on disconnected pads.
    pads_connected: [bool; 4],
    last_pad_scan: Option<std::time::Instant>,
    /// Human-readable form for the panel footer, e.g. "Shift+F1 or Back+Start".
    pub label: String,
}

fn parse_key(name: &str) -> Option<VIRTUAL_KEY> {
    let n = name.to_ascii_lowercase();
    if let Some(f) = n.strip_prefix('f').and_then(|d| d.parse::<u16>().ok()) {
        return (1..=12).contains(&f).then(|| VK_F1 + f - 1);
    }
    if n.len() == 1 {
        let c = n.as_bytes()[0].to_ascii_uppercase();
        if c.is_ascii_alphanumeric() {
            return Some(c as VIRTUAL_KEY);
        }
    }
    Some(match n.as_str() {
        "tab" => VK_TAB,
        "home" => VK_HOME,
        "end" => VK_END,
        "insert" => VK_INSERT,
        "delete" => VK_DELETE,
        "pageup" => VK_PRIOR,
        "pagedown" => VK_NEXT,
        "backquote" | "`" => VK_OEM_3,
        _ => return None,
    })
}

fn title_case(s: &str) -> String {
    s.split('+')
        .map(|p| {
            let mut c = p.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
        })
        .collect::<Vec<String>>()
        .join("+")
}

/// The documented XInputGetState never reports the Xbox button; the
/// undocumented XInputGetStateEx (ordinal 100 in xinput1_3/1_4, also
/// implemented by Wine) does. Use it when the combo needs that button.
fn load_xinput(need_guide: bool) -> Option<XInputGetState> {
    let dlls: &[&std::ffi::CStr] = if need_guide {
        &[c"xinput1_4.dll", c"xinput1_3.dll"]
    } else {
        &[c"xinput1_4.dll", c"xinput1_3.dll", c"xinput9_1_0.dll"]
    };
    for dll in dlls {
        // SAFETY: loading a system DLL and looking up a known export.
        unsafe {
            let h = LoadLibraryA(dll.as_ptr().cast());
            if h.is_null() {
                continue;
            }
            let f = if need_guide {
                // Lookup by ordinal: the "name" pointer carries the number.
                GetProcAddress(h, 100usize as *const u8)
            } else {
                GetProcAddress(h, c"XInputGetState".as_ptr().cast())
            };
            if let Some(f) = f {
                return Some(std::mem::transmute::<_, XInputGetState>(f));
            }
        }
    }
    None
}

fn parse_pad(spec: &str) -> u16 {
    spec.split('+')
        .filter_map(|b| BUTTON_NAMES.iter().find(|(n, _)| n.eq_ignore_ascii_case(b.trim())))
        .fold(0, |m, (_, bit)| m | bit)
}

impl Hotkeys {
    pub fn new(key_spec: &str, pad_spec: &str) -> Hotkeys {
        let mut key_mods = Vec::new();
        let mut key = None;
        for part in key_spec.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "shift" => key_mods.push(VK_SHIFT),
                "ctrl" | "control" => key_mods.push(VK_CONTROL),
                "alt" => key_mods.push(VK_MENU),
                other => key = parse_key(other),
            }
        }
        let mut pad_masks: Vec<u16> = pad_spec.split('|').map(parse_pad).filter(|&m| m != 0).collect();
        let wants_guide = pad_masks.iter().any(|m| m & GUIDE != 0);
        let mut xinput = if pad_masks.is_empty() { None } else { load_xinput(wants_guide) };
        if wants_guide && xinput.is_none() {
            // The Xbox button can't be read here: use the other combos.
            pad_masks.retain(|m| m & GUIDE == 0);
            if pad_masks.is_empty() {
                pad_masks.push(parse_pad(FALLBACK_PAD));
            }
            xinput = load_xinput(false);
        }
        let pad_label = pad_masks
            .iter()
            .map(|&m| {
                if m == GUIDE {
                    "Xbox button".to_string()
                } else {
                    let names: Vec<&str> = BUTTON_NAMES.iter().filter(|(n, b)| m & b != 0 && !["select", "xbox", "home"].contains(n)).map(|(n, _)| *n).collect();
                    title_case(&names.join("+"))
                }
            })
            .collect::<Vec<_>>()
            .join(" or ");
        // Over Bluetooth the Xbox button arrives as the keyboard's Home key
        // (browser home), not as a controller button.
        let guide_key = pad_masks.contains(&GUIDE);
        // Hide the hotkey from the game so it doesn't act on it too.
        crate::gate::HOTKEY_VK.store(key.unwrap_or(0), std::sync::atomic::Ordering::Relaxed);
        for (slot, m) in crate::gate::HOTKEY_MODS.iter().zip(key_mods.iter().copied().chain(std::iter::repeat(0))) {
            slot.store(m, std::sync::atomic::Ordering::Relaxed);
        }
        let label = match (key.is_some(), !pad_masks.is_empty()) {
            (true, true) => format!("{} or {pad_label}", title_case(key_spec)),
            (true, false) => title_case(key_spec),
            (false, true) => pad_label,
            (false, false) => "no hotkey configured".into(),
        };
        Hotkeys {
            key_mods,
            key,
            pad_masks,
            guide_key,
            xinput,
            key_was_down: false,
            pad_was_down: false,
            esc_was_down: false,
            b_was_down: false,
            nav_held: 0,
            nav_next: None,
            hnav_held: 0,
            hnav_next: None,
            pads_connected: [false; 4],
            last_pad_scan: None,
            label,
        }
    }

    fn game_has_focus() -> bool {
        // SAFETY: plain Win32 queries.
        unsafe {
            let hwnd: HWND = GetForegroundWindow();
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, &mut pid);
            pid == GetCurrentProcessId()
        }
    }

    /// Reads the keyboard and controllers; call ~60 times a second.
    /// `panel_open` enables the close-only keys (Esc, controller B).
    pub fn poll(&mut self, panel_open: bool) -> Poll {
        let focused = Self::game_has_focus();
        // SAFETY: GetAsyncKeyState has no preconditions.
        let down = |vk: VIRTUAL_KEY| unsafe { GetAsyncKeyState(vk as i32) } < 0;
        let key_down = focused && self.key.is_some_and(|k| down(k) && self.key_mods.iter().all(|&m| down(m)));
        let esc_down = focused && down(VK_ESCAPE);

        let rescan = self.last_pad_scan.is_none_or(|t| t.elapsed().as_secs() >= 1);
        if rescan {
            self.last_pad_scan = Some(std::time::Instant::now());
        }
        let mut buttons = 0u16;
        let mut stick_y = 0i32;
        let mut stick_x = 0i32;
        if let Some(get) = self.xinput {
            for i in 0..4 {
                if !self.pads_connected[i] && !rescan {
                    continue;
                }
                let mut s = XInputState::default();
                // SAFETY: valid out pointer; 0 = ERROR_SUCCESS.
                let code = unsafe { get(i as u32, &mut s) };
                PAD_CODES[i].store(code, std::sync::atomic::Ordering::Relaxed);
                let ok = code == 0;
                self.pads_connected[i] = ok;
                if ok {
                    buttons |= s.buttons;
                    if (s.thumbs[1] as i32).abs() > stick_y.abs() {
                        stick_y = s.thumbs[1] as i32;
                    }
                    if (s.thumbs[0] as i32).abs() > stick_x.abs() {
                        stick_x = s.thumbs[0] as i32;
                    }
                }
            }
        }
        use std::sync::atomic::Ordering::Relaxed;
        PAD_BUTTONS.store(buttons as u32, Relaxed);
        PAD_LX.store(stick_x, Relaxed);
        PAD_LY.store(stick_y, Relaxed);
        let pad_down = self.pad_masks.iter().any(|&m| buttons & m == m) || (self.guide_key && focused && down(VK_BROWSER_HOME));
        let b_down = buttons & 0x2000 != 0;

        let toggle = (key_down && !self.key_was_down) || (pad_down && !self.pad_was_down);
        let modal = MODAL.load(Relaxed);
        let close = panel_open && !modal && ((esc_down && !self.esc_was_down) || (b_down && !self.b_was_down));
        self.key_was_down = key_down;
        self.pad_was_down = pad_down;
        self.esc_was_down = esc_down;
        self.b_was_down = b_down;
        // Up/down: D-pad, left stick or arrow keys, repeating while held.
        let up = buttons & 0x0001 != 0 || stick_y > 16_000 || (focused && down(VK_UP));
        let dn = buttons & 0x0002 != 0 || stick_y < -16_000 || (focused && down(VK_DOWN));
        let dir = match (up, dn) {
            (true, false) => -1,
            (false, true) => 1,
            _ => 0,
        };
        let now = std::time::Instant::now();
        let mut nav = 0;
        if dir != self.nav_held {
            self.nav_held = dir;
            nav = dir;
            self.nav_next = Some(now + std::time::Duration::from_millis(400));
        } else if dir != 0 && self.nav_next.is_some_and(|t| now >= t) {
            nav = dir;
            self.nav_next = Some(now + std::time::Duration::from_millis(90));
        }
        // Left/right: D-pad, left stick or arrow keys switch blades.
        let left = buttons & 0x0004 != 0 || stick_x < -20_000 || (focused && down(VK_LEFT));
        let right = buttons & 0x0008 != 0 || stick_x > 20_000 || (focused && down(VK_RIGHT));
        let hdir = match (left, right) {
            (true, false) => -1,
            (false, true) => 1,
            _ => 0,
        };
        let mut hnav = 0;
        if hdir != self.hnav_held {
            self.hnav_held = hdir;
            hnav = hdir;
            self.hnav_next = Some(now + std::time::Duration::from_millis(500));
        } else if hdir != 0 && self.hnav_next.is_some_and(|t| now >= t) {
            hnav = hdir;
            self.hnav_next = Some(now + std::time::Duration::from_millis(250));
        }
        if panel_open && hnav != 0 {
            HNAV.fetch_add(hnav, Relaxed);
        }
        if buttons != 0 || stick_x.abs() > 12_000 || stick_y.abs() > 12_000 || (focused && (down(VK_UP) || down(VK_DOWN) || down(VK_LEFT) || down(VK_RIGHT))) {
            PAD_ACTIVITY.fetch_add(1, Relaxed);
        }
        let held = buttons != 0 || key_down || esc_down || self.key_mods.iter().any(|&m| focused && down(m));
        Poll { toggle, close, nav: if panel_open { nav } else { 0 }, held }
    }
}

/// One line about the controller setup, for the log.
pub fn describe(h: &Hotkeys) -> String {
    format!(
        "xinput {}, combos {:x?}, bluetooth home key {}",
        if h.xinput.is_some() { "loaded" } else { "MISSING" },
        h.pad_masks,
        if h.guide_key { "on" } else { "off" }
    )
}

pub struct Poll {
    pub toggle: bool,
    pub close: bool,
    /// -1 / +1 to move the panel selection up / down.
    pub nav: i32,
    /// Any controller button, the hotkey or Esc is currently held.
    pub held: bool,
}
