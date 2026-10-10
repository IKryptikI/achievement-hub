//! Finds the running game's icon: GOG's `goggame-*.ico` next to the exe if
//! there is one (it includes a 256px image), otherwise the icon embedded in
//! the exe itself.

use image::RgbaImage;
use std::path::PathBuf;
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, PrivateExtractIconsW, HICON, ICONINFO};

const SIZE: u32 = 128;

fn exe_path() -> Option<PathBuf> {
    let mut buf = [0u16; 1024];
    // SAFETY: buffer is valid for its length; NULL module = the exe.
    let n = unsafe { GetModuleFileNameW(std::ptr::null_mut(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
    (n > 0).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..n])))
}

/// GOG ships the game's icon as goggame-<product id>.ico in the game folder.
fn gog_icon(exe: &std::path::Path) -> Option<RgbaImage> {
    let dir = exe.parent()?;
    let path = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.to_ascii_lowercase().starts_with("goggame-") && n.to_ascii_lowercase().ends_with(".ico"))
    })?;
    // The ICO decoder picks the largest image in the file.
    Some(image::open(path).ok()?.into_rgba8())
}

/// The first icon embedded in the exe, rendered at SIZE.
fn exe_icon(exe: &std::path::Path) -> Option<RgbaImage> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = exe.as_os_str().encode_wide().chain([0]).collect();
    let mut wide_path = [0u16; 260];
    let n = wide.len().min(wide_path.len());
    wide_path[..n].copy_from_slice(&wide[..n]);
    // SAFETY: Win32 icon extraction; every handle is released below.
    unsafe {
        let mut icon: HICON = std::ptr::null_mut();
        let got = PrivateExtractIconsW(wide_path.as_ptr(), 0, SIZE as i32, SIZE as i32, &mut icon, std::ptr::null_mut(), 1, 0);
        if got == 0 || got == u32::MAX || icon.is_null() {
            return None;
        }
        let mut info: ICONINFO = std::mem::zeroed();
        let ok = GetIconInfo(icon, &mut info) != 0;
        let result = if ok && !info.hbmColor.is_null() {
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: SIZE as i32,
                biHeight: -(SIZE as i32), // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut bgra = vec![0u8; (SIZE * SIZE * 4) as usize];
            let rows = GetDIBits(dc, info.hbmColor, 0, SIZE, bgra.as_mut_ptr().cast(), &mut bmi, DIB_RGB_COLORS);
            DeleteDC(dc);
            (rows > 0).then(|| {
                // Old icons have no alpha channel; treat them as opaque.
                let has_alpha = bgra.chunks_exact(4).any(|p| p[3] != 0);
                for p in bgra.chunks_exact_mut(4) {
                    p.swap(0, 2);
                    if !has_alpha {
                        p[3] = 255;
                    }
                }
                RgbaImage::from_raw(SIZE, SIZE, bgra)
            })
            .flatten()
        } else {
            None
        };
        if ok {
            if !info.hbmColor.is_null() {
                DeleteObject(info.hbmColor);
            }
            if !info.hbmMask.is_null() {
                DeleteObject(info.hbmMask);
            }
        }
        DestroyIcon(icon);
        result
    }
}

/// The running game's icon, resized to SIZE.
pub fn find() -> Option<RgbaImage> {
    let exe = exe_path()?;
    let img = gog_icon(&exe).or_else(|| exe_icon(&exe))?;
    Some(if img.dimensions() == (SIZE, SIZE) {
        img
    } else {
        image::imageops::resize(&img, SIZE, SIZE, image::imageops::FilterType::Lanczos3)
    })
}
