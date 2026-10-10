//! Anti-cheat detection, shared by the launcher and the overlay.
//!
//! Injecting into a game protected by anti-cheat can get a player banned, so
//! Achievement Hub must never do it. Both the launcher (before injecting) and
//! the overlay (before hooking anything) check for anti-cheat and stand down.

use std::path::Path;

/// File or folder names (lower-case) that indicate an anti-cheat system.
/// A name ending in '*' matches as a prefix.
const SIGNATURES: &[(&str, &str)] = &[
    ("easyanticheat", "Easy Anti-Cheat"),
    ("easyanticheat*", "Easy Anti-Cheat"),
    ("start_protected_game.exe", "Easy Anti-Cheat"),
    ("battleye", "BattlEye"),
    ("beservice*", "BattlEye"),
    ("beclient*", "BattlEye"),
    ("gameguard", "nProtect GameGuard"),
    ("gamemon.des", "nProtect GameGuard"),
    ("npgg*", "nProtect GameGuard"),
    ("xigncode", "XIGNCODE3"),
    ("x3.xem", "XIGNCODE3"),
    ("pbsvc.exe", "PunkBuster"),
    ("pbcl.dll", "PunkBuster"),
    ("hshield", "AhnLab HackShield"),
    ("ehsvc.dll", "AhnLab HackShield"),
    ("xtrap", "X-Trap"),
    ("blackcipher", "Nexon Game Security"),
    ("eaanticheat*", "EA Anti-Cheat"),
    ("anticheatexpert", "Anti-Cheat Expert"),
    ("ace-base*", "Anti-Cheat Expert"),
    ("denuvo-anti-cheat*", "Denuvo Anti-Cheat"),
    ("mhyprot*", "miHoYo anti-cheat"),
    ("vgk.sys", "Riot Vanguard"),
    ("vgc.exe", "Riot Vanguard"),
    ("faceit*", "FACEIT"),
    ("randgrid.sys", "RICOCHET"),
];

fn matches(name: &str) -> Option<&'static str> {
    let name = name.to_ascii_lowercase();
    SIGNATURES.iter().find_map(|(sig, which)| {
        let hit = match sig.strip_suffix('*') {
            Some(prefix) => name.starts_with(prefix),
            None => name == *sig,
        };
        hit.then_some(*which)
    })
}

/// Looks for anti-cheat files in a game folder (up to 3 levels deep).
pub fn scan_dir(dir: &Path) -> Option<&'static str> {
    fn walk(dir: &Path, depth: u32) -> Option<&'static str> {
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(which) = matches(&name) {
                return Some(which);
            }
            if depth > 0 && entry.file_type().is_ok_and(|t| t.is_dir()) {
                if let Some(which) = walk(&entry.path(), depth - 1) {
                    return Some(which);
                }
            }
        }
        None
    }
    walk(dir, 3)
}

/// Checks a list of loaded module names (e.g. from the game process).
pub fn scan_modules<'a>(names: impl IntoIterator<Item = &'a str>) -> Option<&'static str> {
    names.into_iter().find_map(|n| matches(n.rsplit(['\\', '/']).next().unwrap_or(n)))
}
