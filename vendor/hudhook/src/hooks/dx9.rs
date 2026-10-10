//! Hooks for DirectX 9.
//!
//! Achievement Hub patch: also hooks `IDirect3DSwapChain9::Present`, which
//! some games and wrappers (e.g. GOG's DirectDraw-to-D3D9 wrapper) use instead
//! of `IDirect3DDevice9::Present`. A thread-local guard keeps the overlay from
//! being drawn twice when a device Present calls into the swap chain.

use std::cell::Cell;
use std::ffi::c_void;
use std::mem;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

use imgui::Context;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use tracing::{error, trace};
use windows::core::{Error, Interface, Result, BOOL, HRESULT};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct3D9::{
    Direct3DCreate9, IDirect3DDevice9, IDirect3DSurface9, IDirect3DSwapChain9, D3DADAPTER_DEFAULT,
    D3DFMT_A8R8G8B8, D3DFMT_X8R8G8B8, D3DLOCKED_RECT, D3DLOCK_READONLY, D3DMULTISAMPLE_NONE,
    D3DPOOL_SYSTEMMEM, D3DSURFACE_DESC, D3DTEXF_NONE, D3DBACKBUFFER_TYPE_MONO,
    D3DCREATE_SOFTWARE_VERTEXPROCESSING, D3DDEVTYPE_NULLREF, D3DDISPLAYMODE, D3DFORMAT,
    D3DPRESENT_PARAMETERS, D3DSWAPEFFECT_DISCARD, D3D_SDK_VERSION,
};
use windows::Win32::Graphics::Gdi::RGNDATA;

use super::DummyHwnd;
use crate::mh::MhHook;
use crate::renderer::{D3D9RenderEngine, Pipeline};
use crate::{perform_eject, util, Hooks, ImguiRenderLoop, EJECT_REQUESTED, HOOK_EJECTION_BARRIER};

type Dx9PresentType = unsafe extern "system" fn(
    this: IDirect3DDevice9,
    psourcerect: *const RECT,
    pdestrect: *const RECT,
    hdestwindowoverride: HWND,
    pdirtyregion: *const RGNDATA,
) -> HRESULT;

type Dx9SwapPresentType = unsafe extern "system" fn(
    this: IDirect3DSwapChain9,
    psourcerect: *const RECT,
    pdestrect: *const RECT,
    hdestwindowoverride: HWND,
    pdirtyregion: *const RGNDATA,
    dwflags: u32,
) -> HRESULT;

type Dx9ResetType =
    unsafe extern "system" fn(this: IDirect3DDevice9, *const D3DPRESENT_PARAMETERS) -> HRESULT;

struct Trampolines {
    dx9_present: Dx9PresentType,
    dx9_reset: Dx9ResetType,
    dx9_swap_present: Dx9SwapPresentType,
}

thread_local! {
    /// True while inside our device Present detour, so the swap chain
    /// detour it may call into doesn't render a second time.
    static IN_DEVICE_PRESENT: Cell<bool> = const { Cell::new(false) };
}

static mut TRAMPOLINES: OnceLock<Trampolines> = OnceLock::new();
static mut PIPELINE: OnceCell<Mutex<Pipeline<D3D9RenderEngine>>> = OnceCell::new();
static mut RENDER_LOOP: OnceCell<Box<dyn ImguiRenderLoop + Send + Sync>> = OnceCell::new();

/// Device the pipeline was built for. Some wrappers destroy and recreate the
/// device (e.g. on pause or mode change) instead of calling Reset; when a
/// different device shows up, the old pipeline's resources are stale.
static PIPELINE_DEVICE: AtomicUsize = AtomicUsize::new(0);

/// Drops the pipeline, keeping the render loop for the next one.
unsafe fn drop_pipeline() {
    if let Some(pipeline) = PIPELINE.take() {
        let render_loop = pipeline.into_inner().take();
        RENDER_LOOP.set(render_loop).map_err(|_| ()).expect("Render loop cell should be empty");
    }
    PIPELINE_DEVICE.store(0, Ordering::SeqCst);
}

unsafe fn init_pipeline(device: &IDirect3DDevice9) -> Result<Mutex<Pipeline<D3D9RenderEngine>>> {
    trace!("initializing pipeline");
    let mut creation_parameters = Default::default();
    device.GetCreationParameters(&mut creation_parameters)?;

    let hwnd = creation_parameters.hFocusWindow;

    let mut ctx = Context::create();
    trace!("creating engine");
    let engine = D3D9RenderEngine::new(device, &mut ctx)?;

    let Some(render_loop) = RENDER_LOOP.take() else {
        error!("Render loop not yet initialized");
        return Err(Error::from_hresult(HRESULT(-1)));
    };

    trace!("creating pipeline");
    let pipeline = Pipeline::new(hwnd, ctx, engine, render_loop).map_err(|(e, render_loop)| {
        RENDER_LOOP.get_or_init(move || render_loop);
        e
    })?;
    Ok(Mutex::new(pipeline))
}

fn render(device: &IDirect3DDevice9, surface: IDirect3DSurface9) -> Result<()> {
    let device_addr = device.as_raw() as usize;
    let known = PIPELINE_DEVICE.load(Ordering::SeqCst);
    if known != 0 && known != device_addr {
        trace!("Device changed; rebuilding pipeline");
        unsafe { drop_pipeline() };
    }
    let pipeline = unsafe { PIPELINE.get_or_try_init(|| init_pipeline(device)) }?;
    PIPELINE_DEVICE.store(device_addr, Ordering::SeqCst);

    let Some(mut pipeline) = pipeline.try_lock() else {
        error!("Could not lock pipeline");
        return Err(Error::from_hresult(HRESULT(-1)));
    };

    pipeline.prepare_render()?;

    if pipeline.render_loop().wants_frame_capture() {
        match unsafe { capture_surface(device, &surface) } {
            Ok((rgba, w, h)) => pipeline.render_loop().on_frame_captured(rgba, w, h),
            Err(e) => error!("Frame capture failed: {e:?}"),
        }
    }

    unsafe { device.BeginScene() }?;
    let render_result = pipeline.render(surface);
    unsafe { device.EndScene() }?;

    render_result
}

/// Copies a (possibly multisampled) render target into system memory and
/// returns it as RGBA8. Supports the common 32-bit back buffer formats.
unsafe fn capture_surface(
    device: &IDirect3DDevice9,
    src: &IDirect3DSurface9,
) -> Result<(Vec<u8>, u32, u32)> {
    let mut desc = D3DSURFACE_DESC::default();
    src.GetDesc(&mut desc)?;
    if desc.Format != D3DFMT_X8R8G8B8 && desc.Format != D3DFMT_A8R8G8B8 {
        return Err(Error::from_hresult(HRESULT(-1)));
    }
    let (w, h) = (desc.Width, desc.Height);

    // Resolve into a plain render target first: GetRenderTargetData can't
    // read multisampled surfaces.
    let mut plain = None;
    device.CreateRenderTarget(
        w,
        h,
        desc.Format,
        D3DMULTISAMPLE_NONE,
        0,
        false,
        &mut plain,
        std::ptr::null_mut(),
    )?;
    let plain = plain.ok_or_else(|| Error::from_hresult(HRESULT(-1)))?;
    device.StretchRect(src, std::ptr::null(), &plain, std::ptr::null(), D3DTEXF_NONE)?;

    let mut sys = None;
    device.CreateOffscreenPlainSurface(
        w,
        h,
        desc.Format,
        D3DPOOL_SYSTEMMEM,
        &mut sys,
        std::ptr::null_mut(),
    )?;
    let sys = sys.ok_or_else(|| Error::from_hresult(HRESULT(-1)))?;
    device.GetRenderTargetData(&plain, &sys)?;

    let mut locked = D3DLOCKED_RECT::default();
    sys.LockRect(&mut locked, std::ptr::null(), D3DLOCK_READONLY as u32)?;
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as usize {
        let row = std::slice::from_raw_parts(
            (locked.pBits as *const u8).add(y * locked.Pitch as usize),
            w as usize * 4,
        );
        for px in row.chunks_exact(4) {
            // BGRA in memory -> RGBA, opaque.
            rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
        }
    }
    sys.UnlockRect()?;
    Ok((rgba, w, h))
}

/// Whether the render loop asks to hold (pause) the game.
fn wants_hold() -> bool {
    let Some(pipeline) = (unsafe { PIPELINE.get() }) else { return false };
    let Some(mut pipeline) = pipeline.try_lock() else { return false };
    pipeline.render_loop().wants_hold()
}

/// Keeps the game's thread here while the render loop wants to hold:
/// pumps window messages (so the window stays responsive and the overlay
/// gets input), renders the overlay and presents, about 60 times a second.
unsafe fn hold(frame: impl Fn() -> Option<(IDirect3DDevice9, IDirect3DSurface9)>, present: impl Fn() -> HRESULT) {
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, PostQuitMessage, TranslateMessage, MSG, PM_REMOVE, WM_QUIT,
    };
    while wants_hold() {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            if msg.message == WM_QUIT {
                // Let the game see the quit once we return.
                PostQuitMessage(msg.wParam.0 as i32);
                return;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if let Some((device, surface)) = frame() {
            if let Err(e) = render(&device, surface) {
                error!("Render error while holding: {e:?}");
            }
        }
        let started = std::time::Instant::now();
        let _ = present();
        // Present may not block (no vsync); cap at roughly 60 fps.
        if let Some(rest) = std::time::Duration::from_millis(16).checked_sub(started.elapsed()) {
            std::thread::sleep(rest);
        }
    }
}

unsafe extern "system" fn dx9_present_impl(
    device: IDirect3DDevice9,
    psourcerect: *const RECT,
    pdestrect: *const RECT,
    hdestwindowoverride: HWND,
    pdirtyregion: *const RGNDATA,
) -> HRESULT {
    let _hook_ejection_guard = HOOK_EJECTION_BARRIER.acquire_ejection_guard();

    let Trampolines { dx9_present, .. } =
        TRAMPOLINES.get().expect("DirectX 9 trampolines uninitialized");

    match unsafe { device.GetBackBuffer(0, 0, D3DBACKBUFFER_TYPE_MONO) } {
        Ok(surface) => {
            if let Err(e) = render(&device, surface) {
                error!("Render error: {e:?}");
            }
        }
        Err(e) => error!("GetBackBuffer error: {e:?}"),
    }

    trace!("Call IDirect3DDevice9::Present trampoline");
    IN_DEVICE_PRESENT.with(|f| f.set(true));
    let result =
        dx9_present(device.clone(), psourcerect, pdestrect, hdestwindowoverride, pdirtyregion);
    hold(
        || {
            let surface = device.GetBackBuffer(0, 0, D3DBACKBUFFER_TYPE_MONO).ok()?;
            Some((device.clone(), surface))
        },
        || dx9_present(device.clone(), psourcerect, pdestrect, hdestwindowoverride, pdirtyregion),
    );
    IN_DEVICE_PRESENT.with(|f| f.set(false));
    if EJECT_REQUESTED.load(Ordering::SeqCst) {
        perform_eject();
    }
    result
}
unsafe extern "system" fn dx9_swap_present_impl(
    swapchain: IDirect3DSwapChain9,
    psourcerect: *const RECT,
    pdestrect: *const RECT,
    hdestwindowoverride: HWND,
    pdirtyregion: *const RGNDATA,
    dwflags: u32,
) -> HRESULT {
    let _hook_ejection_guard = HOOK_EJECTION_BARRIER.acquire_ejection_guard();

    let Trampolines { dx9_swap_present, .. } =
        TRAMPOLINES.get().expect("DirectX 9 trampolines uninitialized");

    if !IN_DEVICE_PRESENT.with(Cell::get) {
        let surface = swapchain.GetBackBuffer(0, D3DBACKBUFFER_TYPE_MONO);
        match (swapchain.GetDevice(), surface) {
            (Ok(device), Ok(surface)) => {
                if let Err(e) = render(&device, surface) {
                    error!("Render error: {e:?}");
                }
            }
            (Err(e), _) | (_, Err(e)) => error!("Swap chain device/back buffer error: {e:?}"),
        }
    }

    trace!("Call IDirect3DSwapChain9::Present trampoline");
    let result = dx9_swap_present(
        swapchain.clone(),
        psourcerect,
        pdestrect,
        hdestwindowoverride,
        pdirtyregion,
        dwflags,
    );
    if !IN_DEVICE_PRESENT.with(Cell::get) {
        hold(
            || {
                let surface = swapchain.GetBackBuffer(0, D3DBACKBUFFER_TYPE_MONO).ok()?;
                Some((swapchain.GetDevice().ok()?, surface))
            },
            || {
                dx9_swap_present(
                    swapchain.clone(),
                    psourcerect,
                    pdestrect,
                    hdestwindowoverride,
                    pdirtyregion,
                    dwflags,
                )
            },
        );
    }
    if EJECT_REQUESTED.load(Ordering::SeqCst) {
        perform_eject();
    }
    result
}

unsafe extern "system" fn dx9_reset_impl(
    this: IDirect3DDevice9,
    present_params: *const D3DPRESENT_PARAMETERS,
) -> HRESULT {
    let _hook_ejection_guard = HOOK_EJECTION_BARRIER.acquire_ejection_guard();

    let Trampolines { dx9_reset, .. } =
        TRAMPOLINES.get().expect("DirectX 9 trampolines uninitialized");

    trace!("Resetting pipeline");
    drop_pipeline();

    dx9_reset(this, present_params)
}

fn get_target_addrs() -> (Dx9PresentType, Dx9ResetType, Dx9SwapPresentType) {
    let d9 = unsafe { Direct3DCreate9(D3D_SDK_VERSION).unwrap() };

    let mut d3d_display_mode =
        D3DDISPLAYMODE { Width: 0, Height: 0, RefreshRate: 0, Format: D3DFORMAT(0) };
    unsafe { d9.GetAdapterDisplayMode(D3DADAPTER_DEFAULT, &mut d3d_display_mode).unwrap() };

    let mut present_params = D3DPRESENT_PARAMETERS {
        Windowed: BOOL(1),
        SwapEffect: D3DSWAPEFFECT_DISCARD,
        BackBufferFormat: d3d_display_mode.Format,
        ..Default::default()
    };

    let dummy_hwnd = DummyHwnd::new();
    let device: IDirect3DDevice9 = util::try_out_ptr(|v| {
        unsafe {
            d9.CreateDevice(
                D3DADAPTER_DEFAULT,
                D3DDEVTYPE_NULLREF,
                dummy_hwnd.hwnd(), // GetDesktopWindow(),
                D3DCREATE_SOFTWARE_VERTEXPROCESSING as u32,
                &mut present_params,
                v,
            )
        }
    })
    .expect("IDirect3DDevice9::CreateDevice: failed to create device");

    let present_ptr = device.vtable().Present;
    let reset_ptr = device.vtable().Reset;
    let swapchain = unsafe { device.GetSwapChain(0) }
        .expect("IDirect3DDevice9::GetSwapChain: failed to get implicit swap chain");
    let swap_present_ptr = swapchain.vtable().Present;

    unsafe {
        (
            mem::transmute::<
                unsafe extern "system" fn(
                    *mut c_void,
                    *const RECT,
                    *const RECT,
                    HWND,
                    *const RGNDATA,
                ) -> HRESULT,
                Dx9PresentType,
            >(present_ptr),
            mem::transmute::<
                unsafe extern "system" fn(*mut c_void, *mut D3DPRESENT_PARAMETERS) -> HRESULT,
                Dx9ResetType,
            >(reset_ptr),
            mem::transmute::<
                unsafe extern "system" fn(
                    *mut c_void,
                    *const RECT,
                    *const RECT,
                    HWND,
                    *const RGNDATA,
                    u32,
                ) -> HRESULT,
                Dx9SwapPresentType,
            >(swap_present_ptr),
        )
    }
}

/// Hooks for DirectX 9.
pub struct ImguiDx9Hooks([MhHook; 3]);

impl ImguiDx9Hooks {
    /// Construct a set of [`MhHook`]s that will render UI via the
    /// provided [`ImguiRenderLoop`].
    ///
    /// The following functions are hooked:
    /// - `IDirect3DDevice9::Present`
    /// - `IDirect3DDevice9::Reset`
    /// - `IDirect3DSwapChain9::Present`
    ///
    /// # Safety
    ///
    /// yolo
    pub unsafe fn new<T>(t: T) -> Self
    where
        T: ImguiRenderLoop + Send + Sync + 'static,
    {
        let (dx9_present_addr, dx9_reset_addr, dx9_swap_present_addr) = get_target_addrs();

        trace!("IDirect3DDevice9::Present = {:p}", dx9_present_addr as *const c_void);
        let hook_present =
            MhHook::new(dx9_present_addr as *mut c_void, dx9_present_impl as *mut c_void)
                .expect("couldn't create IDirect3DDevice9::Present hook");
        let hook_reset = MhHook::new(dx9_reset_addr as *mut c_void, dx9_reset_impl as *mut c_void)
            .expect("couldn't create IDirect3DDevice9::Reset hook");
        let hook_swap_present =
            MhHook::new(dx9_swap_present_addr as *mut c_void, dx9_swap_present_impl as *mut c_void)
                .expect("couldn't create IDirect3DSwapChain9::Present hook");

        RENDER_LOOP.get_or_init(|| Box::new(t));
        TRAMPOLINES.get_or_init(|| Trampolines {
            dx9_present: mem::transmute::<*mut c_void, Dx9PresentType>(hook_present.trampoline()),
            dx9_reset: mem::transmute::<*mut c_void, Dx9ResetType>(hook_reset.trampoline()),
            dx9_swap_present: mem::transmute::<*mut c_void, Dx9SwapPresentType>(
                hook_swap_present.trampoline(),
            ),
        });

        Self([hook_present, hook_reset, hook_swap_present])
    }
}

impl Hooks for ImguiDx9Hooks {
    fn from_render_loop<T>(t: T) -> Box<Self>
    where
        Self: Sized,
        T: ImguiRenderLoop + Send + Sync + 'static,
    {
        Box::new(unsafe { Self::new(t) })
    }

    fn hooks(&self) -> &[MhHook] {
        &self.0
    }

    unsafe fn unhook(&mut self) {
        TRAMPOLINES.take();
        PIPELINE.take().map(|p| p.into_inner().take());
        RENDER_LOOP.take();
    }
}
