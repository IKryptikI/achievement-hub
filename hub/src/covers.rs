//! Cover art for the game lists, off the UI thread: decodes the local
//! file (or downloads it from the store into a cache when there's none)
//! and shrinks it to a small thumbnail.

use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

const THUMB: (u32, u32) = (96, 144);

pub type Request = (PathBuf, Option<PathBuf>, Option<String>);

pub fn spawn_loader() -> (Sender<Request>, Receiver<(PathBuf, Option<egui::ColorImage>)>) {
    let (tx, rx) = channel::<Request>();
    let (out_tx, out_rx) = channel();
    let rx = std::sync::Arc::new(std::sync::Mutex::new(rx));
    for _ in 0..3 {
        let rx = rx.clone();
        let out = out_tx.clone();
        std::thread::spawn(move || {
            loop {
                let Ok((key, path, url)) = rx.lock().map_err(|_| ()).and_then(|r| r.recv().map_err(|_| ())) else { return };
                let _ = out.send((key.clone(), load(&key, path.as_deref(), url.as_deref())));
            }
        });
    }
    (tx, out_rx)
}

fn cache_dir() -> PathBuf {
    crate::library::profile_dir().join("covers")
}

fn load(key: &Path, path: Option<&Path>, url: Option<&str>) -> Option<egui::ColorImage> {
    let img = path.and_then(|p| image::open(p).ok()).or_else(|| {
        let url = url?;
        let name = key.file_name()?.to_string_lossy().replace(|c: char| !c.is_alphanumeric(), "_");
        let cached = cache_dir().join(format!("{name}.jpg"));
        if !cached.exists() {
            let bytes = download(url)?;
            let _ = std::fs::create_dir_all(cache_dir());
            std::fs::write(&cached, bytes).ok()?;
        }
        image::open(&cached).ok()
    })?;
    let thumb = img.resize_to_fill(THUMB.0, THUMB.1, image::imageops::FilterType::Triangle).into_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied([thumb.width() as usize, thumb.height() as usize], thumb.as_raw()))
}

fn get(url: &str) -> Option<Vec<u8>> {
    let mut resp = ureq::get(url).call().ok()?;
    resp.body_mut().with_config().limit(20 << 20).read_to_vec().ok()
}

/// "gog:<id>" means GOG's box art (looked up via its API); anything else is
/// a direct image URL.
fn download(url: &str) -> Option<Vec<u8>> {
    if let Some(id) = url.strip_prefix("gog:") {
        let json: serde_json::Value = serde_json::from_slice(&get(&format!("https://api.gog.com/v2/games/{id}?locale=en-US"))?).ok()?;
        return get(json["_links"]["boxArtImage"]["href"].as_str()?);
    }
    get(url)
}
