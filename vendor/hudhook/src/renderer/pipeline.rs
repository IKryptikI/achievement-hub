use std::collections::HashMap;
use std::mem;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use imgui::Context;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use tracing::{error, warn};
use windows::core::{Error, Result};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetWindowThreadProcessId, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK, MSG,
    PM_REMOVE, WH_GETMESSAGE, WM_NULL,
};

use crate::renderer::input::imgui_wnd_proc_impl;
use crate::renderer::RenderEngine;
use crate::{util, ImguiRenderLoop, MessageFilter};

type RenderLoop = Box<dyn ImguiRenderLoop + Send + Sync>;

// Safety: HWND is an opaque integer handle, safe to send/share across threads.
#[derive(Clone, Copy, Debug)]
#[repr(transparent)]
pub(crate) struct SendableHwnd(HWND);
unsafe impl Send for SendableHwnd {}
unsafe impl Sync for SendableHwnd {}

static PIPELINE_STATES: Lazy<Mutex<HashMap<usize, Arc<PipelineSharedState>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Debug)]
pub(crate) struct PipelineMessage(
    pub(crate) SendableHwnd,
    pub(crate) u32,
    pub(crate) WPARAM,
    pub(crate) LPARAM,
);

pub(crate) struct PipelineSharedState {
    pub(crate) message_filter: AtomicU32,
    pub(crate) tx: Sender<PipelineMessage>,
}

pub(crate) struct Pipeline<T: RenderEngine> {
    hwnd: HWND,
    ctx: Context,
    engine: T,
    render_loop: RenderLoop,
    rx: Receiver<PipelineMessage>,
    shared_state: Arc<PipelineSharedState>,
    queue_buffer: Vec<PipelineMessage>,
    last_frame: Option<Instant>,
    /// WH_GETMESSAGE hook handle (stored as an integer; HHOOK isn't Send).
    msg_hook: isize,
}

impl<T: RenderEngine> Pipeline<T> {
    pub(crate) fn new(
        hwnd: HWND,
        mut ctx: Context,
        mut engine: T,
        mut render_loop: RenderLoop,
    ) -> std::result::Result<Self, (Error, RenderLoop)> {
        let (width, height) = util::win_size(hwnd);

        ctx.io_mut().display_size = [width as f32, height as f32];

        render_loop.initialize(&mut ctx, &mut engine);

        if let Err(e) = engine.setup_fonts(&mut ctx) {
            return Err((e, render_loop));
        }

        // Achievement Hub patch: observe input with a WH_GETMESSAGE hook on the
        // window's thread instead of subclassing the window procedure. Other
        // layers (e.g. DXVK) also subclass the window and re-subclass when the
        // device is recreated; mixing the two chains can make them call each
        // other forever (stack overflow). A message hook isn't part of that
        // chain, and can still block input by turning messages into WM_NULL.
        let thread = unsafe { GetWindowThreadProcessId(hwnd, None) };
        let msg_hook =
            match unsafe { SetWindowsHookExW(WH_GETMESSAGE, Some(pipeline_msg_hook), None, thread) } {
                Ok(h) => h.0 as isize,
                Err(e) => return Err((e, render_loop)),
            };

        let (tx, rx) = mpsc::channel();
        let shared_state = Arc::new(PipelineSharedState {
            message_filter: AtomicU32::new(MessageFilter::empty().bits()),
            tx,
        });

        PIPELINE_STATES.lock().insert(hwnd.0 as usize, Arc::clone(&shared_state));

        Ok(Self {
            hwnd,
            ctx,
            engine,
            render_loop,
            rx,
            shared_state: Arc::clone(&shared_state),
            queue_buffer: Vec::new(),
            last_frame: None,
            msg_hook,
        })
    }

    pub(crate) fn prepare_render(&mut self) -> Result<()> {
        let mut queue_buffer = mem::take(&mut self.queue_buffer);
        queue_buffer.extend(self.rx.try_iter());
        queue_buffer.drain(..).for_each(
            |PipelineMessage(SendableHwnd(hwnd), umsg, wparam, lparam)| {
                imgui_wnd_proc_impl(hwnd, umsg, wparam, lparam, self);
            },
        );
        self.queue_buffer = queue_buffer;

        // Window size isn't observed through the message hook (WM_SIZE is sent,
        // not posted), so track it directly.
        let (width, height) = util::win_size(self.hwnd);
        self.resize(width as u32, height as u32);

        let message_filter = self.render_loop.message_filter(self.ctx.io());

        self.shared_state.message_filter.store(message_filter.bits(), Ordering::SeqCst);

        let io = self.ctx.io_mut();

        io.nav_active = true;
        io.nav_visible = true;

        self.render_loop.before_render(&mut self.ctx, &mut self.engine);

        Ok(())
    }

    pub(crate) fn render(&mut self, render_target: T::RenderTarget) -> Result<()> {
        let now = Instant::now();
        let delta_time = self.last_frame.map_or(Duration::ZERO, |last| now - last);
        self.last_frame = Some(now);

        self.ctx.io_mut().update_delta_time(delta_time);

        let [w, h] = self.ctx.io().display_size;
        let [fsw, fsh] = self.ctx.io().display_framebuffer_scale;

        if (w * fsw) <= 0.0 || (h * fsh) <= 0.0 {
            warn!(
                "Insufficient display size: {w}x{h}, framebuffer_scale: {fsw}x{fsh}; skipping \
                 frame"
            );
            return Ok(());
        }

        let ui = self.ctx.frame();
        self.render_loop.render(ui);
        let draw_data = self.ctx.render();

        self.engine.render(draw_data, render_target)?;

        Ok(())
    }

    pub(crate) fn context(&mut self) -> &mut Context {
        &mut self.ctx
    }

    pub(crate) fn render_loop(&mut self) -> &mut RenderLoop {
        &mut self.render_loop
    }

    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.ctx.io_mut().display_size = [width as f32, height as f32];
        }
    }

    pub(crate) fn update_display_size_from_swap_chain(&mut self, width: u32, height: u32) {
        self.resize(width, height);
    }

    pub(crate) fn wait_idle(&mut self) -> Result<()> {
        self.engine.wait_idle()
    }

    pub(crate) fn cleanup(&mut self) {
        if self.msg_hook != 0 {
            let _ = unsafe { UnhookWindowsHookEx(HHOOK(self.msg_hook as _)) };
            self.msg_hook = 0;
        }
        PIPELINE_STATES.lock().remove(&(self.hwnd.0 as usize));
    }

    pub(crate) fn take(mut self) -> RenderLoop {
        self.cleanup();
        self.render_loop
    }
}

unsafe extern "system" fn pipeline_msg_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && wparam.0 as u32 == PM_REMOVE.0 {
        // SAFETY: for WH_GETMESSAGE, lparam points at the MSG being retrieved.
        let msg = unsafe { &mut *(lparam.0 as *mut MSG) };
        let shared_state = PIPELINE_STATES
            .try_lock()
            .and_then(|states| states.get(&(msg.hwnd.0 as usize)).cloned());
        if let Some(shared_state) = shared_state {
            if let Err(e) = shared_state.tx.send(PipelineMessage(
                SendableHwnd(msg.hwnd),
                msg.message,
                msg.wParam,
                msg.lParam,
            )) {
                error!("Could not send window message through pipeline: {e:?}");
            }
            let message_filter =
                MessageFilter::from_bits_retain(shared_state.message_filter.load(Ordering::SeqCst));
            if message_filter.is_blocking(msg.message) {
                msg.message = WM_NULL;
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}
