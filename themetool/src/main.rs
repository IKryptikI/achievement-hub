//! ah-theme: builds an Achievement Hub theme pack from files you already
//! have. Nothing here ships any third-party artwork; it converts your own
//! copy into the overlay's theme folder.
//!
//!   ah-theme import-mc360 <path to the "MC360 2.1" skin folder> [out dir]
//!
//! The default output is the profile's themes/blades folder (Documents/My
//! Games/Achievement Hub/themes/blades), which the overlay picks up.

mod lzo;
mod xpr;

use std::fs;
use std::path::{Path, PathBuf};

/// Theme file name <- the skin's texture name.
const TEXTURES: &[(&str, &str)] = &[
    ("body-green.png", "background-green-alpha.png"),
    ("body-orange.png", "background-orange-alpha.png"),
    ("body-blue.png", "background-blue-alpha.png"),
    ("body-purple.png", "background-purple-alpha.png"),
    ("glass-top-left.png", "bkgd-whitewash-glass-top-left.png"),
    ("glass-top-middle.png", "bkgd-whitewash-glass-top-middle.png"),
    ("glass-top-right.png", "bkgd-whitewash-glass-top-right.png"),
    ("glass-bottom-left.png", "bkgd-whitewash-glass-bottom-left.png"),
    ("glass-bottom-middle.png", "bkgd-whitewash-glass-bottom-middle.png"),
    ("glass-bottom-right.png", "bkgd-whitewash-glass-bottom-right.png"),
    ("sheen-left.png", "background-overlay-whitewash-left.png"),
    ("sheen-middle.png", "background-overlay-whitewash-centertile.png"),
    ("sheen-right.png", "background-overlay-whitewash-right.png"),
    ("blade-left-1.png", "blades-size1-left-nf.png"),
    ("blade-left-2.png", "blades-size2-left-nf.png"),
    ("blade-left-3.png", "blades-size3-left-nf.png"),
    ("blade-right-1.png", "blades-size1-right-nf.png"),
    ("blade-right-2.png", "blades-size2-right-nf.png"),
    ("blade-right-3.png", "blades-size3-right-nf.png"),
    ("blade-open.png", "blades-size4-header.png"),
    ("runner-left.png", "blades-runner-left.png"),
    ("runner-right.png", "blades-runner-right.png"),
    ("focus.png", "button-focus.png"),
    ("nofocus.png", "button-nofocus.png"),
    ("card.png", "gamecard_home.png"),
    ("card-focus.png", "gamecard_home-focus.png"),
    ("glyph-a.png", "button-a.png"),
    ("glyph-b.png", "button-b.png"),
    ("glyph-x.png", "button-x.png"),
    ("glyph-y.png", "button-y.png"),
];

/// Theme sound <- the skin's sound.
const SOUNDS: &[(&str, &str)] = &[
    ("move.wav", "cursor.wav"),
    ("select.wav", "select.wav"),
    ("back.wav", "back.wav"),
    ("switch.wav", "bladeslide_1.wav"),
    ("open.wav", "guide_open.wav"),
    ("close.wav", "guide_close.wav"),
    ("notify.wav", "notification.wav"),
];

fn profile_dir() -> PathBuf {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
    home.join("Documents").join("My Games").join("Achievement Hub")
}

/// A child of `dir` matching `name` case-insensitively (the skin mixes cases).
fn find(dir: &Path, name: &str) -> Option<PathBuf> {
    fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name)))
}

fn import_mc360(skin: &Path, out: &Path) -> Result<(), String> {
    let media = find(skin, "media").ok_or_else(|| format!("no media folder in {}", skin.display()))?;
    let bundle_path = find(&media, "textures.xpr").ok_or("no media/textures.xpr (is this the MC360 skin folder?)")?;
    let bundle = fs::read(&bundle_path).map_err(|e| format!("{}: {e}", bundle_path.display()))?;
    let entries = xpr::entries(&bundle)?;
    fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;

    let mut made = 0;
    for (ours, theirs) in TEXTURES {
        let Some(entry) = entries.iter().find(|e| e.name.eq_ignore_ascii_case(theirs)) else {
            eprintln!("  missing in the skin: {theirs}");
            continue;
        };
        match xpr::decode(&entry.data) {
            Some(img) => {
                img.save(out.join(ours)).map_err(|e| format!("{ours}: {e}"))?;
                made += 1;
            }
            None => eprintln!("  couldn't decode {theirs}"),
        }
    }
    println!("textures: {made} of {}", TEXTURES.len());

    // The animated background: numbered frames.
    let frames_dir = find(&media, "bkgd_frames");
    let mut frames: Vec<PathBuf> = frames_dir
        .iter()
        .flat_map(|d| fs::read_dir(d).into_iter().flatten().flatten())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("jpg") || x.eq_ignore_ascii_case("png")))
        .collect();
    frames.sort();
    let swirl = out.join("swirl");
    let _ = fs::remove_dir_all(&swirl);
    fs::create_dir_all(&swirl).map_err(|e| e.to_string())?;
    for (i, f) in frames.iter().enumerate() {
        // Some are PNGs named .jpg: go by the contents.
        let img = image::ImageReader::open(f)
            .and_then(|r| r.with_guessed_format())
            .map_err(|e| format!("{}: {e}", f.display()))?
            .decode()
            .map_err(|e| format!("{}: {e}", f.display()))?
            .to_rgba8();
        img.save(swirl.join(format!("{i:02}.png"))).map_err(|e| e.to_string())?;
    }
    println!("background frames: {}", frames.len());

    let mut sounds = 0;
    if let Some(sdir) = find(skin, "sounds") {
        let out_sounds = out.join("sounds");
        fs::create_dir_all(&out_sounds).map_err(|e| e.to_string())?;
        for (ours, theirs) in SOUNDS {
            if let Some(src) = find(&sdir, theirs) {
                fs::copy(&src, out_sounds.join(ours)).map_err(|e| e.to_string())?;
                sounds += 1;
            }
        }
    }
    println!("sounds: {sounds} of {}", SOUNDS.len());

    // The skin's font, if it has one.
    if let Some(font) = find(skin, "fonts").and_then(|d| find(&d, "MC360.ttf")) {
        fs::copy(&font, out.join("font.ttf")).map_err(|e| e.to_string())?;
        println!("font: yes");
    }

    fs::write(out.join("theme.toml"), format!("name = \"Blades\"\nsource = \"MC360 skin ({})\"\n", skin.display())).map_err(|e| e.to_string())?;
    println!("theme written to {}", out.display());
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let result = match args.get(1).map(String::as_str) {
        Some("import-mc360") if args.len() >= 3 => {
            let out = args.get(3).map(PathBuf::from).unwrap_or_else(|| profile_dir().join("themes").join("blades"));
            import_mc360(Path::new(&args[2]), &out)
        }
        _ => Err("usage: ah-theme import-mc360 <MC360 skin folder> [out dir]".into()),
    };
    if let Err(e) = result {
        eprintln!("ah-theme: {e}");
        std::process::exit(1);
    }
}
