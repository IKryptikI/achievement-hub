//! Achievement icons, unlock screenshots and game icons, as imgui textures.
//!
//! Images are keyed by (kind, game id, item id), so any game's media can be
//! shown. They're decoded off the render thread; textures are created on it
//! (in `before_render`) and recreated whenever hudhook rebuilds its renderer
//! (`reset`), since textures belong to the old device.

use hudhook::imgui::TextureId;
use hudhook::RenderContext;
use image::RgbaImage;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;

pub const ICON_SIZE: u32 = 128;
const THUMB_WIDTH: u32 = 480;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Icon,
    /// Greyscale, darkened icon for locked achievements.
    IconLocked,
    Shot,
    /// A game's own icon (item id is empty).
    GameIcon,
    /// Greyscale, darkened game icon: placeholder for locked achievements.
    GameIconLocked,
    /// The Blades background swirl (generated, not loaded).
    Swirl,
    /// A game's store artwork, cropped to 16:9 around its subject (item id
    /// is empty).
    Art,
    /// A game's box art (item id is empty).
    Cover,
    /// A piece of an installed theme pack (item id = file name stem;
    /// background frames are "swirl-00"...).
    Theme,
    /// A player's uploaded gamer picture (item id = gamertag).
    Gamerpic,
}

type Key = (Kind, String, String);

pub struct Media {
    /// Decoded images, kept so textures can be recreated after a reset.
    images: HashMap<Key, RgbaImage>,
    textures: HashMap<Key, TextureId>,
    tx: Sender<(Key, RgbaImage)>,
    /// Mutex only so `Media` is Sync (hudhook requires it); used from one thread.
    rx: Mutex<Receiver<(Key, RgbaImage)>>,
    /// Games whose media has been requested already.
    loaded: Mutex<HashSet<String>>,
}

fn key(kind: Kind, game: &str, id: &str) -> Key {
    (kind, game.to_string(), id.to_string())
}

impl Default for Media {
    fn default() -> Self {
        let (tx, rx) = channel();
        let mut images = HashMap::new();
        images.insert(key(Kind::Swirl, "", ""), swirl(512));
        Media { images, textures: HashMap::new(), tx, rx: Mutex::new(rx), loaded: Mutex::new(HashSet::new()) }
    }
}

impl Media {
    /// Loads a game's icons and screenshot thumbnails in the background, once.
    /// `icons` are hand-made icons from the definitions; unlocked achievements
    /// without one use the icon cropped automatically at unlock.
    pub fn load_game(&self, game: &str, shots: PathBuf, icons: Vec<(String, PathBuf)>, unlocked: Vec<String>) {
        if !self.loaded.lock().map(|mut l| l.insert(game.to_string())).unwrap_or(false) {
            return;
        }
        let tx = self.tx.clone();
        let game = game.to_string();
        std::thread::spawn(move || {
            let mut have = HashSet::new();
            for (id, path) in icons {
                if let Some(img) = load(&path) {
                    send_icon(&tx, &game, &id, img);
                    have.insert(id);
                }
            }
            for id in unlocked.iter().filter(|id| !have.contains(*id)) {
                if let Some(img) = load(&auto_icon_path(&shots, id)) {
                    send_icon(&tx, &game, id, img);
                }
            }
            for id in unlocked {
                if let Some(img) = load(&thumb_path(&shots, &id)) {
                    let _ = tx.send((key(Kind::Shot, &game, &id), img));
                }
            }
        });
    }

    /// Loads a player's gamer picture (downloaded by `social`) in the background.
    pub fn load_picture(&self, gamertag: &str, path: PathBuf) {
        let tx = self.tx.clone();
        let tag = gamertag.to_string();
        std::thread::spawn(move || {
            if let Some(img) = load(&path) {
                let _ = tx.send((key(Kind::Gamerpic, "", &tag), img));
            }
        });
    }

    pub fn set_game_icon(&self, game: &str, img: RgbaImage) {
        send_game_icon(&self.tx, game, img);
    }

    /// Loads saved game icons (from the profile) in the background.
    pub fn preload_game_icons(&self, icons: Vec<(String, PathBuf)>) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            for (game, path) in icons {
                if let Some(img) = load(&path) {
                    send_game_icon(&tx, &game, img);
                }
            }
        });
    }

    /// Loads achievement icons (e.g. just fetched) in the background.
    pub fn add_icons(&self, game: &str, icons: Vec<(String, PathBuf)>) {
        let tx = self.tx.clone();
        let game = game.to_string();
        std::thread::spawn(move || {
            for (id, path) in icons {
                if let Some(img) = load(&path) {
                    send_icon(&tx, &game, &id, img);
                }
            }
        });
    }

    /// Loads a game's store artwork in the background, if it's saved.
    /// (Kind::Art "0", "1", ... are its screenshots, in order.)
    pub fn load_art(&self, game: &str, profile: &std::path::Path) {
        let tx = self.tx.clone();
        let game = game.to_string();
        let shots: Vec<PathBuf> = (0..crate::storeinfo::MAX_SHOTS).map(|n| crate::storeinfo::shot_path(profile, &game, n)).take_while(|p| p.exists()).collect();
        let cover = crate::storeinfo::cover_path(profile, &game);
        std::thread::spawn(move || {
            for (n, path) in shots.iter().enumerate() {
                if let Some(img) = load(path) {
                    let _ = tx.send((key(Kind::Art, &game, &n.to_string()), crop_to_subject(&img, 16.0 / 9.0, 960)));
                }
            }
            if let Some(img) = load(&cover) {
                let _ = tx.send((key(Kind::Cover, &game, ""), img));
            }
        });
    }

    /// Loads a theme pack's images (PNG files, plus numbered frames in
    /// `swirl/`) in the background.
    pub fn load_theme(&self, dir: PathBuf) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let pngs = |d: &std::path::Path| -> Vec<PathBuf> {
                let mut v: Vec<PathBuf> = std::fs::read_dir(d)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("png")))
                    .collect();
                v.sort();
                v
            };
            for p in pngs(&dir) {
                let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                if let Some(img) = load(&p) {
                    if stem == "body-orange" {
                        // A fifth blade colour: the orange pushed to red.
                        let mut red = img.clone();
                        for px in red.pixels_mut() {
                            px[1] = (px[1] as f32 * 0.32) as u8;
                            px[2] = (px[2] as f32 * 0.55) as u8;
                        }
                        let _ = tx.send((key(Kind::Theme, "", "body-red"), red));
                    }
                    let _ = tx.send((key(Kind::Theme, "", &stem), img));
                }
            }
            for (i, p) in pngs(&dir.join("swirl")).into_iter().enumerate() {
                if let Some(img) = load(&p) {
                    let _ = tx.send((key(Kind::Theme, "", &format!("swirl-{i:02}")), img));
                }
            }
        });
    }

    /// Saves a captured frame as the screenshot for each achievement in `ids`
    /// (full size + thumbnail) in `dir`, in the background. Achievements in
    /// `needs_icon` also get an icon cropped from the middle of the frame.
    #[allow(clippy::too_many_arguments)]
    pub fn save_shot(&self, game: &str, dir: PathBuf, ids: Vec<String>, needs_icon: Vec<String>, rgba: Vec<u8>, width: u32, height: u32) {
        let tx = self.tx.clone();
        let game = game.to_string();
        std::thread::spawn(move || {
            let Some(full) = RgbaImage::from_raw(width, height, rgba) else { return };
            let _ = std::fs::create_dir_all(&dir);
            let thumb_h = (height as u64 * THUMB_WIDTH as u64 / width.max(1) as u64) as u32;
            let thumb = image::imageops::resize(&full, THUMB_WIDTH, thumb_h.max(1), image::imageops::FilterType::Triangle);
            // Icon: a centred square, 80% of the frame height, so it frames
            // whatever was in the middle of the screen at that moment.
            let side = ((height as f32 * 0.8) as u32).min(width).max(1);
            let crop = image::imageops::crop_imm(&full, (width - side) / 2, (height - side) / 2, side, side).to_image();
            let icon = image::imageops::resize(&crop, ICON_SIZE, ICON_SIZE, image::imageops::FilterType::Lanczos3);
            for id in ids {
                let _ = full.save(dir.join(format!("{id}.png")));
                let _ = thumb.save(thumb_path(&dir, &id));
                let _ = tx.send((key(Kind::Shot, &game, &id), thumb.clone()));
                if needs_icon.contains(&id) {
                    let _ = icon.save(auto_icon_path(&dir, &id));
                    send_icon(&tx, &game, &id, icon.clone());
                }
            }
        });
    }

    /// Call when the renderer was (re)built: old textures are invalid.
    pub fn reset(&mut self) {
        self.textures.clear();
    }

    /// Uploads newly decoded images and recreates textures after a reset.
    pub fn upload(&mut self, ctx: &mut dyn RenderContext) {
        let received: Vec<_> = self.rx.lock().map(|rx| rx.try_iter().collect()).unwrap_or_default();
        for (k, img) in received {
            // Same size: update the existing texture in place.
            if let (Some(&t), Some(old)) = (self.textures.get(&k), self.images.get(&k)) {
                if old.dimensions() == img.dimensions() && ctx.replace_texture(t, img.as_raw(), img.width(), img.height()).is_ok() {
                    self.images.insert(k, img);
                    continue;
                }
            }
            self.textures.remove(&k);
            self.images.insert(k, img);
        }
        for (k, img) in &self.images {
            if !self.textures.contains_key(k) {
                if let Ok(t) = ctx.load_texture(img.as_raw(), img.width(), img.height()) {
                    self.textures.insert(k.clone(), t);
                }
            }
        }
    }

    /// Texture and pixel size.
    pub fn get(&self, kind: Kind, game: &str, id: &str) -> Option<(TextureId, [f32; 2])> {
        let k = key(kind, game, id);
        let t = *self.textures.get(&k)?;
        let img = self.images.get(&k)?;
        Some((t, [img.width() as f32, img.height() as f32]))
    }
}

/// A soft one-armed spiral, white with the brightness in the alpha: turned
/// slowly behind the Blades body it gives the dashboard's drifting light.
fn swirl(size: u32) -> RgbaImage {
    RgbaImage::from_fn(size, size, |x, y| {
        let u = x as f32 / size as f32 * 2.0 - 1.0;
        let v = y as f32 / size as f32 * 2.0 - 1.0;
        let r = (u * u + v * v).sqrt();
        let theta = v.atan2(u);
        let band = 0.5 + 0.5 * (theta - r * std::f32::consts::TAU * 3.2).cos();
        // Fade out at the rim and soften the very centre.
        let rim = 1.0 - ((r - 0.80) / 0.20).clamp(0.0, 1.0);
        let core = 0.55 + 0.45 * (r / 0.15).min(1.0);
        let a = band.powf(1.6) * rim * rim * core;
        image::Rgba([255, 255, 255, (a * 255.0) as u8])
    })
}

/// Crops `img` to `aspect` around where the picture is (store backgrounds
/// often put the subject to one side of a plain backdrop), at most `max_w`
/// wide.
fn crop_to_subject(img: &RgbaImage, aspect: f32, max_w: u32) -> RgbaImage {
    let (w, h) = img.dimensions();
    let (cw, ch) = if (w as f32 / h as f32) > aspect { (((h as f32) * aspect) as u32, h) } else { (w, ((w as f32) / aspect) as u32) };
    // The subject: the luminance-weighted centre of the columns/rows that
    // differ most from the image's average (sampled coarsely).
    let step = (w.max(h) / 256).max(1);
    let lum = |x: u32, y: u32| {
        let p = img.get_pixel(x, y);
        (p[0] as f32 * 0.3 + p[1] as f32 * 0.59 + p[2] as f32 * 0.11) / 255.0
    };
    let mut mean = 0.0f32;
    let mut n = 0.0f32;
    for y in (0..h).step_by(step as usize) {
        for x in (0..w).step_by(step as usize) {
            mean += lum(x, y);
            n += 1.0;
        }
    }
    mean /= n.max(1.0);
    let (mut sx, mut sy, mut sw) = (0.0f32, 0.0f32, 0.0f32);
    for y in (0..h).step_by(step as usize) {
        for x in (0..w).step_by(step as usize) {
            let d = (lum(x, y) - mean).abs();
            sx += x as f32 * d;
            sy += y as f32 * d;
            sw += d;
        }
    }
    let (fx, fy) = if sw > 0.0 { (sx / sw, sy / sw) } else { (w as f32 / 2.0, h as f32 / 2.0) };
    let x0 = (fx - cw as f32 / 2.0).clamp(0.0, (w - cw) as f32) as u32;
    let y0 = (fy - ch as f32 / 2.0).clamp(0.0, (h - ch) as f32) as u32;
    let cropped = image::imageops::crop_imm(img, x0, y0, cw, ch).to_image();
    if cw > max_w {
        image::imageops::resize(&cropped, max_w, ch * max_w / cw, image::imageops::FilterType::Triangle)
    } else {
        cropped
    }
}

/// A copy in darkened greyscale, for locked states.
fn greyed(img: &RgbaImage) -> RgbaImage {
    let mut out = img.clone();
    for p in out.pixels_mut() {
        let l = (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32) * 0.55;
        p[0] = l as u8;
        p[1] = l as u8;
        p[2] = l as u8;
    }
    out
}

fn send_game_icon(tx: &Sender<(Key, RgbaImage)>, game: &str, img: RgbaImage) {
    let _ = tx.send((key(Kind::GameIconLocked, game, ""), greyed(&img)));
    let _ = tx.send((key(Kind::GameIcon, game, ""), img));
}

fn send_icon(tx: &Sender<(Key, RgbaImage)>, game: &str, id: &str, img: RgbaImage) {
    let img = if img.dimensions() != (ICON_SIZE, ICON_SIZE) {
        image::imageops::resize(&img, ICON_SIZE, ICON_SIZE, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    let locked = greyed(&img);
    let _ = tx.send((key(Kind::Icon, game, id), img));
    let _ = tx.send((key(Kind::IconLocked, game, id), locked));
}

fn auto_icon_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.icon.png"))
}

fn thumb_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.thumb.png"))
}

fn load(path: &Path) -> Option<RgbaImage> {
    // By content, not extension (downloaded art can be PNG named .jpg).
    Some(image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?.into_rgba8())
}
