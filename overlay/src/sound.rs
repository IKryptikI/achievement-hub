//! Interface sounds. Built-in ones are synthesised here; a player can swap
//! any of them for their own by putting `<name>.wav` in the profile's
//! `sounds` folder (move, select, back, switch, open, close, unlock). An
//! installed Blades theme pack's sounds play while the Blades design is on
//! (its notification sound for unlocks). Played
//! with Windows' PlaySound, asynchronously, one at a time.

use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};

#[derive(Clone, Copy)]
pub enum Sfx {
    /// Moving the selection.
    Move,
    /// A / Enter on something.
    Select,
    /// B / Esc out of something.
    Back,
    /// Changing blade or design.
    Switch,
    /// The panel opening and closing.
    Open,
    Close,
    /// An achievement unlocking (its toast appearing).
    Unlock,
}

const ALL: [(Sfx, &str); 7] = [
    (Sfx::Move, "move"),
    (Sfx::Select, "select"),
    (Sfx::Back, "back"),
    (Sfx::Switch, "switch"),
    (Sfx::Open, "open"),
    (Sfx::Close, "close"),
    (Sfx::Unlock, "unlock"),
];

/// A theme pack's file for a sound (themetool names the skin's notification
/// sound notify.wav).
fn theme_file(name: &str) -> &str {
    if name == "unlock" { "notify" } else { name }
}

static SOUNDS: OnceLock<Vec<Vec<u8>>> = OnceLock::new();
/// The Blades theme pack's sounds (None where it has none).
static THEME: OnceLock<Vec<Option<Vec<u8>>>> = OnceLock::new();
static ENABLED: AtomicBool = AtomicBool::new(true);
/// Volume in percent, and the sounds scaled to it (kept alive for good:
/// PlaySound reads them while playing; volume changes are rare).
static VOLUME: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(100);
static SCALED: std::sync::Mutex<Option<(u8, &'static [Vec<u8>], &'static [Option<Vec<u8>>])>> = std::sync::Mutex::new(None);

/// Sounds on or off, from the Settings screen.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// Volume (percent), from the Settings screen.
pub fn set_volume(percent: u8) {
    VOLUME.store(percent.clamp(5, 100), Ordering::Relaxed);
}

/// A 16-bit PCM WAV at `percent` volume (other formats unchanged).
fn scaled(wav: &[u8], percent: u8) -> Vec<u8> {
    scale_pcm(wav.to_vec(), percent as f32 / 100.0)
}

/// Loads the player's own sounds where present, synthesising the rest.
/// Returns the names of the ones replaced.
pub fn init(profile_dir: &Path, theme_dir: Option<&Path>, enabled: bool) -> Vec<&'static str> {
    ENABLED.store(enabled, Ordering::Relaxed);
    let read = |dir: &Path, name: &str| std::fs::read(dir.join(format!("{name}.wav"))).ok().filter(|b| b.starts_with(b"RIFF")).map(quieter);
    let mut custom = Vec::new();
    let sounds = ALL
        .iter()
        .map(|&(sfx, name)| match read(&profile_dir.join("sounds"), name) {
            Some(bytes) => {
                custom.push(name);
                bytes
            }
            None => wav(&synth(sfx)),
        })
        .collect();
    let _ = SOUNDS.set(sounds);
    if let Some(dir) = theme_dir {
        // The player's own sounds still win over the theme's.
        let theme = ALL.iter().map(|&(_, name)| if custom.contains(&name) { None } else { read(&dir.join("sounds"), theme_file(name)) }).collect();
        let _ = THEME.set(theme);
    }
    custom
}

/// Last time the move sound started (ms since start), to keep holding a
/// direction from restarting it every repeat (loud, crackly).
static LAST_MOVE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn now_ms() -> u64 {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64
}

pub fn play(sfx: Sfx) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    if matches!(sfx, Sfx::Move) {
        let t = now_ms();
        if t.saturating_sub(LAST_MOVE.load(Ordering::Relaxed)) < 160 {
            return;
        }
        LAST_MOVE.store(t, Ordering::Relaxed);
    }
    let Some(sounds) = SOUNDS.get() else { return };
    let i = ALL.iter().position(|(s, _)| std::mem::discriminant(s) == std::mem::discriminant(&sfx)).unwrap_or(0);
    let themed = (crate::ui::theme::skin() == crate::ui::theme::Skin::Blades)
        .then(|| THEME.get().and_then(|t| t[i].as_ref()))
        .flatten();
    let mut bytes: &[u8] = themed.unwrap_or(&sounds[i]);
    let vol = VOLUME.load(Ordering::Relaxed);
    if vol < 100 {
        if let Ok(mut cache) = SCALED.lock() {
            if cache.as_ref().is_none_or(|(v, ..)| *v != vol) {
                let plain: &'static [Vec<u8>] = Box::leak(sounds.iter().map(|w| scaled(w, vol)).collect::<Vec<_>>().into_boxed_slice());
                let theme: &'static [Option<Vec<u8>>] =
                    Box::leak(THEME.get().map(|t| t.iter().map(|w| w.as_ref().map(|w| scaled(w, vol))).collect::<Vec<_>>()).unwrap_or_default().into_boxed_slice());
                *cache = Some((vol, plain, theme));
            }
            if let Some((_, plain, theme)) = *cache {
                bytes = match themed {
                    Some(_) => theme.get(i).and_then(|t| t.as_deref()).unwrap_or(&plain[i]),
                    None => &plain[i],
                };
            }
        }
    }
    // SAFETY: the WAV bytes live in a static for the life of the process,
    // as SND_MEMORY | SND_ASYNC requires.
    unsafe {
        PlaySoundW(bytes.as_ptr().cast(), std::ptr::null_mut(), SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
    }
}

/// Turns a 16-bit PCM WAV down to about half volume (console UI sounds are
/// mastered loud for TV speakers); other formats are left as they are.
fn quieter(wav: Vec<u8>) -> Vec<u8> {
    scale_pcm(wav, 0.5)
}

/// A 16-bit PCM WAV with its samples scaled by `gain`.
fn scale_pcm(mut wav: Vec<u8>, gain: f32) -> Vec<u8> {
    let find = |id: &[u8], wav: &[u8]| -> Option<(usize, usize)> {
        let mut p = 12;
        while p + 8 <= wav.len() {
            let len = u32::from_le_bytes(wav[p + 4..p + 8].try_into().ok()?) as usize;
            if &wav[p..p + 4] == id {
                return Some((p + 8, len.min(wav.len() - p - 8)));
            }
            p += 8 + len + (len & 1);
        }
        None
    };
    let Some((fmt, _)) = find(b"fmt ", &wav) else { return wav };
    let format = u16::from_le_bytes([wav[fmt], wav[fmt + 1]]);
    let bits = u16::from_le_bytes([wav[fmt + 14], wav[fmt + 15]]);
    if format != 1 || bits != 16 {
        return wav;
    }
    if let Some((data, len)) = find(b"data", &wav) {
        for c in wav[data..data + len].chunks_exact_mut(2) {
            let v = (i16::from_le_bytes([c[0], c[1]]) as f32 * gain) as i16;
            c.copy_from_slice(&v.to_le_bytes());
        }
    }
    wav
}

// ── Synthesis ───────────────────────────────────────────────────────────

const RATE: f32 = 44_100.0;

/// A tone: frequency glides from `f0` to `f1`, with a quick attack and an
/// exponential decay; a touch of the octave gives it a glassy edge.
fn tone(out: &mut Vec<f32>, start: f32, len: f32, f0: f32, f1: f32, gain: f32, decay: f32) {
    let (s0, n) = ((start * RATE) as usize, (len * RATE) as usize);
    if out.len() < s0 + n {
        out.resize(s0 + n, 0.0);
    }
    let mut phase = 0.0f32;
    for i in 0..n {
        let t = i as f32 / RATE;
        let f = f0 + (f1 - f0) * (i as f32 / n as f32);
        phase += f / RATE * std::f32::consts::TAU;
        let env = (t / 0.004).min(1.0) * (-t * decay).exp();
        out[s0 + i] += gain * env * (phase.sin() + 0.25 * (2.0 * phase).sin());
    }
}

/// A whoosh: noise through a resonant band-pass whose centre sweeps from
/// `f0` to `f1`, swelling and fading.
fn whoosh(out: &mut Vec<f32>, len: f32, f0: f32, f1: f32, gain: f32) {
    let n = (len * RATE) as usize;
    if out.len() < n {
        out.resize(n, 0.0);
    }
    let mut seed = 0x2545_f491u32;
    let (mut low, mut band) = (0.0f32, 0.0f32);
    for (i, sample) in out.iter_mut().enumerate().take(n) {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let noise = seed as f32 / u32::MAX as f32 * 2.0 - 1.0;
        let x = i as f32 / n as f32;
        // Exponential sweep sounds even.
        let f = f0 * (f1 / f0).powf(x);
        let k = 2.0 * (std::f32::consts::PI * f / RATE).sin();
        let q = 0.35;
        low += k * band;
        let high = noise - low - q * band;
        band += k * high;
        let env = (x * std::f32::consts::PI).sin().powf(1.5);
        *sample += gain * env * band;
    }
}

fn synth(sfx: Sfx) -> Vec<f32> {
    let mut s = Vec::new();
    match sfx {
        Sfx::Move => tone(&mut s, 0.0, 0.06, 1500.0, 1400.0, 0.22, 70.0),
        Sfx::Select => {
            tone(&mut s, 0.0, 0.18, 880.0, 880.0, 0.25, 22.0);
            tone(&mut s, 0.055, 0.25, 1320.0, 1320.0, 0.22, 16.0);
        }
        Sfx::Back => {
            tone(&mut s, 0.0, 0.16, 1046.0, 1000.0, 0.22, 24.0);
            tone(&mut s, 0.05, 0.22, 698.0, 660.0, 0.22, 18.0);
        }
        Sfx::Switch => {
            whoosh(&mut s, 0.22, 700.0, 2600.0, 0.38);
            tone(&mut s, 0.02, 0.10, 1900.0, 1700.0, 0.06, 40.0);
        }
        Sfx::Open => {
            whoosh(&mut s, 0.30, 500.0, 3000.0, 0.40);
            tone(&mut s, 0.12, 0.30, 1175.0, 1175.0, 0.16, 12.0);
            tone(&mut s, 0.17, 0.35, 1760.0, 1760.0, 0.12, 10.0);
        }
        Sfx::Close => {
            whoosh(&mut s, 0.26, 2600.0, 450.0, 0.40);
            tone(&mut s, 0.02, 0.20, 1175.0, 1100.0, 0.12, 18.0);
        }
        Sfx::Unlock => {
            // A bright rising chord with a shimmer: G, D, G an octave up, B.
            whoosh(&mut s, 0.45, 900.0, 4200.0, 0.18);
            tone(&mut s, 0.00, 0.90, 784.0, 784.0, 0.20, 4.5);
            tone(&mut s, 0.08, 0.90, 1175.0, 1175.0, 0.18, 4.5);
            tone(&mut s, 0.16, 1.00, 1568.0, 1568.0, 0.17, 4.0);
            tone(&mut s, 0.24, 1.10, 1976.0, 1976.0, 0.12, 3.5);
        }
    }
    s
}

/// 16-bit mono PCM WAV.
fn wav(samples: &[f32]) -> Vec<u8> {
    let data: Vec<u8> = samples.iter().flat_map(|&x| ((x.clamp(-1.0, 1.0) * 32_000.0) as i16).to_le_bytes()).collect();
    let mut out = Vec::with_capacity(44 + data.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&(RATE as u32).to_le_bytes());
    out.extend_from_slice(&(RATE as u32 * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    out
}

/// The built-in sounds as WAV files (for listening to them outside a game).
pub fn export(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    for (sfx, name) in ALL {
        let _ = std::fs::write(dir.join(format!("{name}.wav")), wav(&synth(sfx)));
    }
}
