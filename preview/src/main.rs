//! ah-preview.exe: opens a Direct3D 9 window like a game would, loads
//! overlay.dll from its own folder (which then runs in its preview mode:
//! panel open, profile read-only, nothing hooked but drawing), lets the
//! animations settle and saves the frame as a BMP. For checking the look
//! without launching a game.
//!
//! Settings (environment): AH_OUT (output path, default preview.bmp),
//! AH_SIZE (e.g. 1600x900), AH_SECS (seconds before capture, default 2.5),
//! plus the overlay's AH_SKIN and AH_BLADE.

#![windows_subsystem = "windows"]

use std::time::{Duration, Instant};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D9::*;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_DESTROY {
        // SAFETY: plain Win32 call.
        unsafe { PostQuitMessage(0) };
        return LRESULT(0);
    }
    // SAFETY: default handling for everything else.
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

fn note(msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("preview.log") {
        let _ = writeln!(f, "{msg}");
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn main() {
    let (w, h) = env("AH_SIZE")
        .and_then(|s| {
            let (a, b) = s.split_once('x')?;
            Some((a.parse().ok()?, b.parse().ok()?))
        })
        .unwrap_or((1600u32, 900u32));
    let secs: f32 = env("AH_SECS").and_then(|s| s.parse().ok()).unwrap_or(2.5);
    let out = env("AH_OUT").unwrap_or_else(|| "preview.bmp".into());
    if let Err(e) = run(w, h, secs, &out) {
        let _ = std::fs::write("preview-error.txt", format!("{e:?}"));
        std::process::exit(1);
    }
}

fn run(w: u32, h: u32, secs: f32, out: &str) -> windows::core::Result<()> {
    // SAFETY: standard window + D3D9 setup; every pointer passed is valid
    // for the call.
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = w!("AhPreview");
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance.into(), lpszClassName: class, ..Default::default() };
        RegisterClassW(&wc);
        let mut rect = RECT { left: 0, top: 0, right: w as i32, bottom: h as i32 };
        AdjustWindowRect(&mut rect, WS_OVERLAPPEDWINDOW, false)?;
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("Achievement Hub preview"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            // Off screen: the capture reads the back buffer, not the screen.
            -4000,
            -4000,
            rect.right - rect.left,
            rect.bottom - rect.top,
            None,
            None,
            Some(instance.into()),
            None,
        )?;

        let d3d = Direct3DCreate9(D3D_SDK_VERSION).ok_or_else(|| windows::core::Error::from(E_FAIL))?;
        let mut pp = D3DPRESENT_PARAMETERS {
            BackBufferWidth: w,
            BackBufferHeight: h,
            BackBufferFormat: D3DFMT_X8R8G8B8,
            BackBufferCount: 1,
            SwapEffect: D3DSWAPEFFECT_COPY,
            hDeviceWindow: hwnd,
            Windowed: true.into(),
            PresentationInterval: D3DPRESENT_INTERVAL_IMMEDIATE as u32,
            ..Default::default()
        };
        let mut device = None;
        d3d.CreateDevice(D3DADAPTER_DEFAULT, D3DDEVTYPE_HAL, hwnd, D3DCREATE_HARDWARE_VERTEXPROCESSING as u32, &mut pp, &mut device)?;
        let device = device.ok_or_else(|| windows::core::Error::from(E_FAIL))?;
        note("device created");

        // The overlay, from next to this exe.
        // AH_VIA=version.dll: load the overlay the way games do, through
        // the stand-in DLL, and check one forwarded call works.
        if let Some(via) = env("AH_VIA") {
            let proxy = std::env::current_exe().unwrap_or_default().with_file_name(&via);
            let m = LoadLibraryW(PCWSTR(HSTRING::from(proxy.as_os_str()).as_ptr()))?;
            let exe = HSTRING::from(std::env::current_exe().unwrap_or_default().as_os_str());
            if let Some(f) = windows::Win32::System::LibraryLoader::GetProcAddress(m, windows::core::s!("GetFileVersionInfoSizeW")) {
                let f: extern "system" fn(PCWSTR, *mut u32) -> u32 = std::mem::transmute(f);
                note(&format!("forwarded GetFileVersionInfoSizeW -> {}", f(PCWSTR(exe.as_ptr()), std::ptr::null_mut())));
            }
            if let Some(f) = windows::Win32::System::LibraryLoader::GetProcAddress(m, windows::core::s!("timeGetTime")) {
                let f: extern "system" fn() -> u32 = std::mem::transmute(f);
                note(&format!("forwarded timeGetTime -> {}", f()));
            }
            std::thread::sleep(Duration::from_millis(500));
        } else {
            let dll = std::env::current_exe().unwrap_or_default().with_file_name("overlay.dll");
            LoadLibraryW(PCWSTR(HSTRING::from(dll.as_os_str()).as_ptr()))?;
        }

        note("overlay loaded");
        let start = Instant::now();
        let mut frames = 0u32;
        let mut msg = MSG::default();
        loop {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    return Ok(());
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            // A dark, slightly blue "game" behind the panel.
            device.Clear(0, std::ptr::null(), D3DCLEAR_TARGET as u32, 0xff1a1e24, 1.0, 0)?;
            device.BeginScene()?;
            device.EndScene()?;
            device.Present(std::ptr::null(), std::ptr::null(), HWND::default(), std::ptr::null())?;
            frames += 1;
            if frames % 30 == 1 {
                note(&format!("frame {frames} at {:.2}s", start.elapsed().as_secs_f32()));
            }
            if start.elapsed().as_secs_f32() >= secs {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        // The back buffer still holds the last frame, overlay included
        // (copy swap effect).
        let back = device.GetBackBuffer(0, 0, D3DBACKBUFFER_TYPE_MONO)?;
        let mut sys = None;
        device.CreateOffscreenPlainSurface(w, h, D3DFMT_X8R8G8B8, D3DPOOL_SYSTEMMEM, &mut sys, std::ptr::null_mut())?;
        let sys = sys.ok_or_else(|| windows::core::Error::from(E_FAIL))?;
        device.GetRenderTargetData(&back, &sys)?;
        let mut lr = D3DLOCKED_RECT::default();
        sys.LockRect(&mut lr, std::ptr::null(), D3DLOCK_READONLY as u32)?;
        let mut pixels = Vec::with_capacity((w * h * 3) as usize);
        // BMP rows go bottom-up, BGR.
        for y in (0..h).rev() {
            let row = (lr.pBits as *const u8).add((y as i32 * lr.Pitch) as usize);
            for x in 0..w as usize {
                pixels.extend_from_slice(std::slice::from_raw_parts(row.add(x * 4), 3));
            }
        }
        sys.UnlockRect()?;
        write_bmp(out, w, h, &pixels);
        note("saved");
        let _ = DestroyWindow(hwnd);
    }
    Ok(())
}

fn write_bmp(path: &str, w: u32, h: u32, bgr: &[u8]) {
    let row = (w * 3).div_ceil(4) * 4;
    let size = 54 + row * h;
    let mut f = Vec::with_capacity(size as usize);
    f.extend_from_slice(b"BM");
    for v in [size, 0, 54, 40] {
        f.extend_from_slice(&v.to_le_bytes());
    }
    f.extend_from_slice(&(w as i32).to_le_bytes());
    f.extend_from_slice(&(h as i32).to_le_bytes());
    f.extend_from_slice(&1u16.to_le_bytes());
    f.extend_from_slice(&24u16.to_le_bytes());
    for v in [0u32, row * h, 2835, 2835, 0, 0] {
        f.extend_from_slice(&v.to_le_bytes());
    }
    for y in 0..h as usize {
        let src = &bgr[y * w as usize * 3..(y + 1) * w as usize * 3];
        f.extend_from_slice(src);
        f.resize(f.len() + (row as usize - src.len()), 0);
    }
    let _ = std::fs::write(path, f);
}
