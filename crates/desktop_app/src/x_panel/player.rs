#![allow(dead_code)]
//! In-app video playback via an embedded mpv child window (Windows).
//!
//! mpv (free, GPL) is located next to termy.exe (`mpv\mpv.exe` or
//! `mpv.exe`), then on PATH. Its video surface is a Win32 child window
//! we create and own inside the Termy window, positioned exactly over
//! the X panel's viewer area — it moves/resizes with Termy, never
//! resizes Termy, and is destroyed (process killed) on close. Controls
//! go over mpv's JSON IPC named pipe.
//!
//! Linux cannot embed the Win32 surface, so it degrades honestly:
//! the same mpv binary (from beside the executable or PATH) is opened
//! as a separate, external mpv window owned by the window manager.
//! There is no IPC seek/pause/progress on Linux — only alive/stop —
//! and the panel says so in the viewer instead of pretending the
//! embedded controls work. Other non-Windows builds stay inert.

use std::path::PathBuf;
#[cfg(any(windows, target_os = "linux"))]
use std::process::{Child, Command, Stdio};

/// Logical rect (x, y, w, h) in the parent window's client area.
pub type Rect = (f32, f32, f32, f32);

/// Clear strip above the native video. The video window covers whatever
/// is behind it, so the close X lives in this strip, not under mpv.
pub const CLOSE_BAND: f32 = 44.0;
pub const CLOSE_BUTTON: f32 = 36.0;

/// The embedded surface, in logical pixels, over the X panel's viewer.
/// Returns None until the window has a real viewport, so the player is
/// never created at a placeholder full-screen size and then shrunk.
/// The top [`CLOSE_BAND`] under the header is left for the close X.
pub fn embed_rect(viewport_w: f32, viewport_h: f32, fill_window: bool) -> Option<Rect> {
    if !viewport_w.is_finite() || !viewport_h.is_finite() {
        return None;
    }
    if viewport_w < 320.0 || viewport_h < 240.0 {
        return None;
    }
    let panel_w = if fill_window {
        viewport_w
    } else {
        480.0_f32.min(viewport_w)
    };
    let height = (viewport_h - 160.0 - CLOSE_BAND).clamp(80.0, viewport_h);
    let x = (viewport_w - panel_w).max(0.0);
    Some((x, 56.0 + CLOSE_BAND, panel_w, height))
}

/// Top-right X, in the band the video window does not cover.
pub fn close_button_rect(viewport_w: f32, viewport_h: f32, fill_window: bool) -> Option<Rect> {
    let video = embed_rect(viewport_w, viewport_h, fill_window)?;
    let x = video.0 + video.2 - CLOSE_BUTTON - 8.0;
    let y = video.1 - CLOSE_BAND + 4.0;
    Some((x, y, CLOSE_BUTTON, CLOSE_BUTTON))
}

pub fn rects_overlap(a: Rect, b: Rect) -> bool {
    let ax2 = a.0 + a.2;
    let ay2 = a.1 + a.3;
    let bx2 = b.0 + b.2;
    let by2 = b.1 + b.3;
    a.0 < bx2 && ax2 > b.0 && a.1 < by2 && ay2 > b.1
}

/// Screen rectangle the native host is allowed to occupy.
pub type ScreenRect = (i32, i32, i32, i32);

/// What a Win32 move/size request becomes. A fullscreen request is
/// replaced with the panel rect before the window manager applies it,
/// so there is no frame at monitor size. While closing, the window is
/// forced hidden so unparenting cannot flash it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClampedPos {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub hide: bool,
}

pub fn clamp_window_change(
    allowed: ScreenRect,
    _requested: ScreenRect,
    closing: bool,
) -> ClampedPos {
    ClampedPos {
        x: allowed.0,
        y: allowed.1,
        w: allowed.2.max(1),
        h: allowed.3.max(1),
        hide: closing,
    }
}

/// Create hidden, place, then show. Destroy is refused while the
/// window is still shown: hide first so teardown cannot restore a
/// top-level fullscreen placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostPhase {
    Hidden,
    Placed,
    Shown,
    Closing,
    Destroyed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostGate {
    pub phase: HostPhase,
    pub allowed: ScreenRect,
}

impl HostGate {
    pub fn hidden(allowed: ScreenRect) -> Self {
        Self {
            phase: HostPhase::Hidden,
            allowed,
        }
    }

    pub fn place(&mut self) -> Result<ScreenRect, &'static str> {
        match self.phase {
            HostPhase::Hidden => {
                self.phase = HostPhase::Placed;
                Ok(self.allowed)
            }
            HostPhase::Placed | HostPhase::Shown => Ok(self.allowed),
            HostPhase::Closing | HostPhase::Destroyed => Err("place after close"),
        }
    }

    pub fn show(&mut self) -> Result<(), &'static str> {
        match self.phase {
            HostPhase::Placed | HostPhase::Shown => {
                self.phase = HostPhase::Shown;
                Ok(())
            }
            HostPhase::Hidden => Err("show before the surface is placed"),
            HostPhase::Closing | HostPhase::Destroyed => Err("show while closing"),
        }
    }

    pub fn retarget(&mut self, allowed: ScreenRect) {
        if self.phase != HostPhase::Destroyed {
            self.allowed = allowed;
        }
    }

    pub fn hide_for_close(&mut self) -> Result<(), &'static str> {
        if self.phase == HostPhase::Destroyed {
            return Err("already destroyed");
        }
        self.phase = HostPhase::Closing;
        Ok(())
    }

    pub fn destroy(&mut self) -> Result<(), &'static str> {
        if self.phase != HostPhase::Closing {
            return Err("destroy while the surface can still paint");
        }
        self.phase = HostPhase::Destroyed;
        Ok(())
    }
}

/// Win32 style bits for the video host. Not visible at creation, and
/// children are clipped to the host so an mpv child cannot paint at
/// monitor size outside the panel.
pub const HOST_STYLE_VISIBLE: u32 = 0x1000_0000;
pub const HOST_STYLE_CLIP_CHILDREN: u32 = 0x0200_0000;
pub const HOST_STYLE_POPUP: u32 = 0x8000_0000;
pub const HOST_STYLE_CLIP_SIBLINGS: u32 = 0x0400_0000;

pub fn host_create_style() -> u32 {
    HOST_STYLE_POPUP | HOST_STYLE_CLIP_SIBLINGS | HOST_STYLE_CLIP_CHILDREN
}

pub fn host_style_starts_hidden(style: u32) -> bool {
    style & HOST_STYLE_VISIBLE == 0 && style & HOST_STYLE_CLIP_CHILDREN != 0
}

/// mpv's Windows flip present paints a black full-screen frame when the
/// swap chain is created and when it is destroyed. The host rectangle
/// does not change. This switch turns that present off.
pub const D3D11_FLIP_OFF: &str = "--d3d11-flip=no";

/// Restored placement used before a window is destroyed. Show command 0
/// is SW_HIDE. The rect stays the panel, not the monitor.
pub fn hidden_restore(panel: ScreenRect) -> (u32, ScreenRect) {
    (0, panel)
}

/// Flags for one playback. `--media-controls=no` stops mpv from opening
/// the extra top-level `mpv-smtc` window (the 2304×1243 window seen on
/// the live desktop). Mute and volume are mpv's own, not the OS mixer.
pub fn playback_args(
    wid: Option<isize>,
    pipe: Option<&str>,
    geometry: Option<&str>,
    volume: u8,
) -> Vec<String> {
    let volume = volume.min(100);
    let mut args = Vec::new();
    if let Some(id) = wid {
        args.push(format!("--wid={id}"));
    }
    if let Some(pipe) = pipe {
        args.push(format!("--input-ipc-server={pipe}"));
    }
    if let Some(geometry) = geometry {
        args.push(format!("--geometry={geometry}"));
    }
    args.extend(
        [
            "--fs=no",
            "--fullscreen=no",
            "--no-border",
            "--no-auto-window-resize",
            "--media-controls=no",
            "--input-default-bindings=no",
            "--input-vo-keyboard=no",
            "--osc=no",
            "--audio-exclusive=no",
            "--aid=auto",
            "--no-terminal",
            "--really-quiet",
            "--keep-open=yes",
            "--cursor-autohide=no",
            "--hwdec=auto-safe",
        ]
        .into_iter()
        .map(str::to_string),
    );
    args.push(format!("--volume={volume}"));
    args.push(if volume == 0 {
        "--mute=yes".to_string()
    } else {
        "--mute=no".to_string()
    });
    #[cfg(windows)]
    args.push(D3D11_FLIP_OFF.to_string());
    args
}

/// mpv's own volume, 0–100. Zero is silent. This does not touch the OS mixer.
pub fn volume_commands(percent: u8) -> [String; 2] {
    let percent = percent.min(100);
    [
        format!(r#"{{"command":["set_property","volume",{percent}]}}"#),
        format!(
            r#"{{"command":["set_property","mute",{}]}}"#,
            if percent == 0 { "true" } else { "false" }
        ),
    ]
}

pub fn volume_percent(click_x: f32, bar_x: f32, bar_w: f32) -> u8 {
    let frac = if bar_w <= 1.0 {
        0.0
    } else {
        ((click_x - bar_x) / bar_w).clamp(0.0, 1.0)
    };
    (frac * 100.0).round().clamp(0.0, 100.0) as u8
}

/// Playback generation. Open and dismiss each bump it. A callback that
/// captured an older value must not close or reopen the current player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayGen(pub u64);

pub fn begin_playback(generation: &mut u64) -> PlayGen {
    *generation = generation.wrapping_add(1);
    PlayGen(*generation)
}

pub fn dismiss_playback(generation: &mut u64) -> PlayGen {
    *generation = generation.wrapping_add(1);
    PlayGen(*generation)
}

pub fn callback_is_current(live: u64, captured: PlayGen) -> bool {
    live == captured.0
}

pub fn find_player() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
        {
            candidates.push(dir.join("mpv").join("mpv.exe"));
            candidates.push(dir.join("mpv.exe"));
        }
        candidates.push(PathBuf::from(r"C:\Program Files\MPV Player\mpv.exe"));
        candidates.push(PathBuf::from("mpv.exe"));
        candidates.push(PathBuf::from("mpv"));
        candidates
            .into_iter()
            .find(|p| p.as_os_str() == "mpv" || p.as_os_str() == "mpv.exe" || p.exists())
    }
    #[cfg(target_os = "linux")]
    {
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
        {
            candidates.push(dir.join("mpv").join("mpv"));
            candidates.push(dir.join("mpv"));
        }
        candidates.push(PathBuf::from("mpv"));
        candidates
            .into_iter()
            .find(|p| p.as_os_str() == "mpv" || p.exists())
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        None
    }
}

pub struct VideoPlayer {
    #[cfg(windows)]
    inner: win::Player,
    /// Linux external-mpv child (no embedded surface, no IPC).
    #[cfg(target_os = "linux")]
    child: Option<Child>,
    #[cfg(not(any(windows, target_os = "linux")))]
    _priv: (),
}

impl VideoPlayer {
    pub fn start(
        parent_hwnd: isize,
        scale: f32,
        rect: Rect,
        url: &str,
        volume: u8,
    ) -> Result<Self, String> {
        #[cfg(windows)]
        {
            let bin = find_player()
                .ok_or_else(|| "mpv not found (expected mpv\\mpv.exe next to Termy)".to_string())?;
            let inner = win::Player::start(bin, parent_hwnd, scale, rect, url, volume)?;
            Ok(Self { inner })
        }
        #[cfg(target_os = "linux")]
        {
            // External window fallback: mpv opens its own window; the
            // parent hwnd / rect do not apply and are ignored honestly.
            let _ = (parent_hwnd, scale, rect);
            let bin = find_player().ok_or_else(|| {
                "mpv not found on PATH (install mpv for external video playback)".to_string()
            })?;
            let mut args = playback_args(None, None, None, volume);
            args.push(url.to_string());
            let child = Command::new(&bin)
                .args(&args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("could not start mpv ({}): {e}", bin.display()))?;
            Ok(Self { child: Some(child) })
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            let _ = (parent_hwnd, scale, rect, url);
            Err("in-app video is Windows-only in this build".into())
        }
    }
    pub fn reposition(&mut self, scale: f32, rect: Rect) {
        #[cfg(windows)]
        self.inner.reposition(scale, rect);
        #[cfg(not(windows))]
        let _ = (scale, rect);
    }
    pub fn toggle_pause(&mut self) {
        self.command(r#"{"command":["cycle","pause"]}"#);
    }
    pub fn seek_relative(&mut self, seconds: i32) {
        self.command(&format!(r#"{{"command":["seek",{seconds},"relative"]}}"#));
    }
    pub fn seek_absolute(&mut self, seconds: f64) {
        self.command(&format!(
            r#"{{"command":["seek",{seconds:.3},"absolute"]}}"#
        ));
        #[cfg(windows)]
        self.inner.set_progress_pos(seconds);
    }
    /// Latest (position, duration) in seconds from the background poller.
    pub fn progress(&self) -> (f64, f64) {
        #[cfg(windows)]
        {
            self.inner.progress()
        }
        #[cfg(not(windows))]
        {
            (0.0, 0.0)
        }
    }
    pub fn set_muted(&mut self, muted: bool) {
        self.command(&format!(
            r#"{{"command":["set_property","mute",{}]}}"#,
            if muted { "true" } else { "false" }
        ));
    }

    pub fn set_volume(&mut self, percent: u8) {
        for command in volume_commands(percent) {
            self.command(&command);
        }
    }
    fn command(&mut self, json: &str) {
        #[cfg(windows)]
        self.inner.ipc(json);
        #[cfg(not(windows))]
        let _ = json;
    }
    #[cfg_attr(not(any(windows, target_os = "linux")), allow(unused_mut))]
    pub fn stop(mut self) {
        #[cfg(windows)]
        self.inner.stop_inner();
        #[cfg(target_os = "linux")]
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
    /// False once the mpv process has exited (EOF quit, crash, or its
    /// window closed) — the panel polls this to clear playing state.
    pub fn is_alive(&mut self) -> bool {
        #[cfg(windows)]
        {
            self.inner.is_alive()
        }
        #[cfg(target_os = "linux")]
        {
            match self.child.as_mut() {
                Some(child) => matches!(child.try_wait(), Ok(None)),
                None => false,
            }
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            false
        }
    }
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    #[allow(non_camel_case_types)]
    type Hwnd = *mut core::ffi::c_void;
    type Hinstance = *mut core::ffi::c_void;
    type Hmenu = *mut core::ffi::c_void;
    type Lresult = isize;
    type Wparam = usize;
    type Lparam = isize;
    const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
    const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
    const SWP_NOACTIVATE: u32 = 0x0010;
    const SWP_NOZORDER: u32 = 0x0004;
    const SWP_SHOWWINDOW: u32 = 0x0040;
    const SWP_HIDEWINDOW: u32 = 0x0080;
    const WM_WINDOWPOSCHANGING: u32 = 0x0046;
    const WM_SYSCOMMAND: u32 = 0x0112;
    const SC_MAXIMIZE: usize = 0xF030;
    const SW_HIDE: i32 = 0;
    const SW_SHOWNA: i32 = 8;

    #[repr(C)]
    struct WindowPos {
        hwnd: Hwnd,
        insert_after: Hwnd,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        flags: u32,
    }

    struct HostSlot {
        allowed: super::ScreenRect,
        closing: bool,
    }

    fn hosts() -> &'static std::sync::Mutex<std::collections::HashMap<isize, HostSlot>> {
        static HOSTS: std::sync::OnceLock<
            std::sync::Mutex<std::collections::HashMap<isize, HostSlot>>,
        > = std::sync::OnceLock::new();
        HOSTS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
    }

    fn trace_hwnd(hwnd: isize, event: &str, bounds: super::ScreenRect) {
        let line = format!(
            "termy-video hwnd={hwnd:#x} event={event} bounds={},{},{},{}",
            bounds.0, bounds.1, bounds.2, bounds.3
        );
        eprintln!("{line}");
        let path = std::env::temp_dir().join("termy-video-hwnd.log");
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            use std::io::Write;
            let _ = writeln!(file, "{line}");
        }
    }

    #[repr(C)]
    struct WndClassExW {
        cb_size: u32,
        style: u32,
        lpfn: Option<unsafe extern "system" fn(Hwnd, u32, Wparam, Lparam) -> Lresult>,
        cb_cls_extra: i32,
        cb_wnd_extra: i32,
        h_instance: Hinstance,
        h_icon: *mut core::ffi::c_void,
        h_cursor: *mut core::ffi::c_void,
        hbr_background: *mut core::ffi::c_void,
        lpsz_menu_name: *const u16,
        lpsz_class_name: *const u16,
        h_icon_sm: *mut core::ffi::c_void,
    }
    unsafe extern "system" {
        fn RegisterClassExW(c: *const WndClassExW) -> u16;
        fn CreateWindowExW(
            ex: u32,
            class: *const u16,
            name: *const u16,
            style: u32,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            parent: Hwnd,
            menu: Hmenu,
            inst: Hinstance,
            param: *mut core::ffi::c_void,
        ) -> Hwnd;
        fn DestroyWindow(h: Hwnd) -> i32;
        fn SetWindowPos(h: Hwnd, after: Hwnd, x: i32, y: i32, w: i32, h: i32, flags: u32) -> i32;
        fn DefWindowProcW(h: Hwnd, m: u32, w: Wparam, l: Lparam) -> Lresult;
        fn GetModuleHandleW(name: *const u16) -> Hinstance;
        fn ClientToScreen(h: Hwnd, pt: *mut Point) -> i32;
        fn GetWindowRect(h: Hwnd, r: *mut RectWin) -> i32;
        fn IsIconic(h: Hwnd) -> i32;
        fn ShowWindow(h: Hwnd, cmd: i32) -> i32;
        fn GetWindow(h: Hwnd, cmd: u32) -> Hwnd;
        fn GetClassNameW(h: Hwnd, buf: *mut u16, max: i32) -> i32;
        fn SetWindowPlacement(h: Hwnd, place: *const WindowPlacement) -> i32;
    }
    #[repr(C)]
    struct WindowPlacement {
        length: u32,
        flags: u32,
        show_cmd: u32,
        pt_min: Point,
        pt_max: Point,
        rc_normal: RectWin,
    }
    fn bury_window(hwnd: Hwnd, panel: super::ScreenRect) {
        if hwnd.is_null() {
            return;
        }
        let mut class = [0u16; 48];
        let n = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), 48) };
        let class_name = if n > 0 {
            String::from_utf16_lossy(&class[..n as usize])
        } else {
            String::new()
        };
        let mut was = RectWin {
            l: 0,
            t: 0,
            r: 0,
            b: 0,
        };
        unsafe { GetWindowRect(hwnd, &mut was) };
        trace_hwnd(
            hwnd as isize,
            &format!(
                "bury class={class_name} was={},{},{},{}",
                was.l,
                was.t,
                was.r - was.l,
                was.b - was.t
            ),
            panel,
        );
        let (show_cmd, rect) = super::hidden_restore(panel);
        let place = WindowPlacement {
            length: std::mem::size_of::<WindowPlacement>() as u32,
            flags: 0,
            show_cmd,
            pt_min: Point { x: 0, y: 0 },
            pt_max: Point { x: 0, y: 0 },
            rc_normal: RectWin {
                l: rect.0,
                t: rect.1,
                r: rect.0 + rect.2,
                b: rect.1 + rect.3,
            },
        };
        unsafe {
            SetWindowPlacement(hwnd, &place);
            ShowWindow(hwnd, SW_HIDE);
        }
    }
    fn bury_children(parent: Hwnd, panel: super::ScreenRect) {
        let mut child = unsafe { GetWindow(parent, 5) };
        let mut guard = 0;
        while !child.is_null() && guard < 16 {
            let next = unsafe { GetWindow(child, 2) };
            bury_window(child, panel);
            child = next;
            guard += 1;
        }
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Point {
        x: i32,
        y: i32,
    }
    #[repr(C)]
    struct RectWin {
        l: i32,
        t: i32,
        r: i32,
        b: i32,
    }
    /// GPUI presents the panel through DirectComposition, which paints
    /// over legacy child HWNDs (airspace) — an embedded child surface
    /// stays black. So the host is an *owned top-level* window docked
    /// exactly over the panel's viewer area: it renders normally, stays
    /// above Termy, and a poll thread keeps it glued to Termy when the
    /// Termy window moves (GPUI only re-renders on resize).
    fn screen_rect(parent: Hwnd, scale: f32, rect: Rect) -> (i32, i32, i32, i32) {
        let (x, y, w, h) = phys(scale, rect);
        let mut origin = Point { x: 0, y: 0 };
        unsafe { ClientToScreen(parent, &mut origin) };
        (origin.x + x, origin.y + y, w, h)
    }
    unsafe extern "system" fn wndproc(h: Hwnd, m: u32, w: Wparam, l: Lparam) -> Lresult {
        if m == WM_SYSCOMMAND && (w & 0xFFF0) == SC_MAXIMIZE {
            return 0;
        }
        if m == WM_WINDOWPOSCHANGING && l != 0 {
            let key = h as isize;
            if let Some(slot) = hosts().lock().ok().and_then(|guard| {
                guard.get(&key).map(|slot| HostSlot {
                    allowed: slot.allowed,
                    closing: slot.closing,
                })
            }) {
                let pos = unsafe { &mut *(l as *mut WindowPos) };
                let requested = (pos.x, pos.y, pos.cx, pos.cy);
                let clamped = super::clamp_window_change(slot.allowed, requested, slot.closing);
                pos.x = clamped.x;
                pos.y = clamped.y;
                pos.cx = clamped.w;
                pos.cy = clamped.h;
                if clamped.hide {
                    pos.flags |= SWP_HIDEWINDOW;
                    pos.flags &= !SWP_SHOWWINDOW;
                }
                if requested != (clamped.x, clamped.y, clamped.w, clamped.h) || clamped.hide {
                    trace_hwnd(
                        key,
                        &format!(
                            "{} requested={},{},{},{}",
                            if clamped.hide { "clamp-hide" } else { "clamp" },
                            requested.0,
                            requested.1,
                            requested.2,
                            requested.3
                        ),
                        (clamped.x, clamped.y, clamped.w, clamped.h),
                    );
                }
            }
        }
        unsafe { DefWindowProcW(h, m, w, l) }
    }
    fn extract_number(line: &str, key: &str) -> Option<f64> {
        let start = line.find(key)? + key.len();
        let rest = &line[start..];
        let end = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
            .unwrap_or(rest.len());
        rest[..end].parse::<f64>().ok()
    }
    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(Some(0))
            .collect()
    }
    fn phys(scale: f32, rect: Rect) -> (i32, i32, i32, i32) {
        (
            (rect.0 * scale).round() as i32,
            (rect.1 * scale).round() as i32,
            (rect.2 * scale).max(1.0).round() as i32,
            (rect.3 * scale).max(1.0).round() as i32,
        )
    }

    pub struct Player {
        child: Option<Child>,
        host: Hwnd,
        pipe: String,
        parent: Hwnd,
        scale: f32,
        rect: Rect,
        last_rect: (i32, i32, i32, i32),
        stop_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
        progress: std::sync::Arc<std::sync::Mutex<(f64, f64)>>,
        shared_rect: std::sync::Arc<std::sync::Mutex<(i32, i32, i32, i32)>>,
        #[allow(dead_code)]
        poll: Option<std::thread::JoinHandle<()>>,
    }
    // HWND is owned by this (UI-spawned) player; Termy drives it from the UI thread.
    unsafe impl Send for Player {}

    impl Player {
        pub fn start(
            bin: PathBuf,
            parent: isize,
            scale: f32,
            rect: Rect,
            url: &str,
            volume: u8,
        ) -> Result<Self, String> {
            if parent == 0 {
                return Err("Termy window handle not ready yet".into());
            }
            let class = wide("TermyXVideoHost");
            let (x, y, w, h) = screen_rect(parent as Hwnd, scale, rect);
            let host = unsafe {
                let wc = WndClassExW {
                    cb_size: std::mem::size_of::<WndClassExW>() as u32,
                    style: 0,
                    lpfn: Some(wndproc),
                    cb_cls_extra: 0,
                    cb_wnd_extra: 0,
                    h_instance: GetModuleHandleW(std::ptr::null()),
                    h_icon: std::ptr::null_mut(),
                    h_cursor: std::ptr::null_mut(),
                    hbr_background: std::ptr::null_mut(),
                    lpsz_menu_name: std::ptr::null(),
                    lpsz_class_name: class.as_ptr(),
                    h_icon_sm: std::ptr::null_mut(),
                };
                let _ = RegisterClassExW(&wc); // already-registered is fine
                CreateWindowExW(
                    WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                    class.as_ptr(),
                    wide("").as_ptr(),
                    // Hidden until place+show. WS_VISIBLE here is the
                    // full-size flash: mpv resizes the wid before the
                    // first glue tick.
                    super::host_create_style(),
                    x,
                    y,
                    w,
                    h,
                    parent as Hwnd,
                    std::ptr::null_mut(),
                    GetModuleHandleW(std::ptr::null()),
                    std::ptr::null_mut(),
                )
            };
            if host.is_null() {
                return Err("could not create the in-app video surface".into());
            }
            let mut gate = super::HostGate::hidden((x, y, w, h));
            trace_hwnd(host as isize, "create-hidden", (x, y, w, h));
            let placed = gate.place().map_err(|err| {
                unsafe { DestroyWindow(host) };
                err.to_string()
            })?;
            hosts().lock().unwrap().insert(host as isize, HostSlot {
                allowed: placed,
                closing: false,
            });
            unsafe {
                SetWindowPos(
                    host,
                    std::ptr::null_mut(),
                    placed.0,
                    placed.1,
                    placed.2,
                    placed.3,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
            }
            gate.show().map_err(|err| {
                hosts().lock().unwrap().remove(&(host as isize));
                unsafe { DestroyWindow(host) };
                err.to_string()
            })?;
            unsafe { ShowWindow(host, SW_SHOWNA) };
            trace_hwnd(host as isize, "shown", placed);
            // Unique pipe per player instance: a process-wide name let a
            // previous (leaked) poller compete with the next video's mpv.
            static PIPE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let pipe = format!(
                r"\\.\pipe\termyx-mpv-{}-{}",
                std::process::id(),
                PIPE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            );
            let mut args = super::playback_args(
                Some(host as isize),
                Some(&pipe),
                Some(&format!("{w}x{h}+{x}+{y}")),
                volume,
            );
            args.push(url.to_string());
            trace_hwnd(host as isize, "spawn", (x, y, w, h));
            let mut cmd = Command::new(&bin);
            cmd.args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
            let child = cmd.spawn().map_err(|e| {
                hosts().lock().unwrap().remove(&(host as isize));
                unsafe {
                    ShowWindow(host, SW_HIDE);
                    DestroyWindow(host);
                }
                format!("could not start mpv ({}): {e}", bin.display())
            })?;
            let stop_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let progress_arc = std::sync::Arc::new(std::sync::Mutex::new((0.0f64, 0.0f64)));
            {
                // Position poller: background thread only, 2x/sec, short
                // named-pipe round-trips. Never touches the UI thread.
                let flag = stop_flag.clone();
                let prog = progress_arc.clone();
                let pipe_name = pipe.clone();
                std::thread::spawn(move || {
                    use std::io::{BufRead, BufReader, Write};
                    use std::sync::atomic::Ordering;
                    while !flag.load(Ordering::Relaxed) {
                        if let Ok(file) = std::fs::OpenOptions::new()
                            .read(true)
                            .write(true)
                            .open(&pipe_name)
                        {
                            let mut writer = match file.try_clone() {
                                Ok(w) => w,
                                Err(_) => file,
                            };
                            let _ = writer.write_all(b"{\"command\":[\"get_property\",\"time-pos\"],\"request_id\":11}\n{\"command\":[\"get_property\",\"duration\"],\"request_id\":12}\n");
                            let mut reader = BufReader::new(writer);
                            let (mut pos, mut dur) = (None, None);
                            // A get_property against a still-loading stream
                            // is answered with an error line (no "data").
                            // That still ANSWERS the request, so count
                            // answers, not just values: waiting for a data
                            // line that will never come leaves this read
                            // blocked until mpv happens to emit a seek/EOF
                            // event, which froze the seek bar at 0:00/0:00
                            // on fresh plays and hid EOF from the panel.
                            let (mut answered_pos, mut answered_dur) = (false, false);
                            for _ in 0..8 {
                                let mut line = String::new();
                                match reader.read_line(&mut line) {
                                    Ok(0) | Err(_) => break,
                                    Ok(_) => {
                                        if line.contains("\"request_id\":11") {
                                            answered_pos = true;
                                            pos = extract_number(&line, "\"data\":");
                                        }
                                        if line.contains("\"request_id\":12") {
                                            answered_dur = true;
                                            dur = extract_number(&line, "\"data\":");
                                        }
                                        if answered_pos && answered_dur {
                                            break;
                                        }
                                    }
                                }
                            }
                            if let (Some(p2), Some(d2)) = (pos, dur) {
                                *prog.lock().unwrap() = (p2, d2);
                            }
                        }
                        std::thread::sleep(std::time::Duration::from_millis(500));
                    }
                });
            }
            let shared_rect = std::sync::Arc::new(std::sync::Mutex::new(phys(scale, rect)));
            let poll = {
                let flag = stop_flag.clone();
                let shared = shared_rect.clone();
                let (parent_raw, host_raw) = (parent, host as isize);
                std::thread::spawn(move || {
                    use std::sync::atomic::Ordering;
                    while !flag.load(Ordering::Relaxed) {
                        unsafe {
                            let parent_h = parent_raw as Hwnd;
                            let mut origin = Point { x: 0, y: 0 };
                            ClientToScreen(parent_h, &mut origin);
                            let (ox, oy, w2, h2) = *shared.lock().unwrap();
                            if flag.load(Ordering::Relaxed) {
                                break;
                            }
                            let want = (origin.x + ox, origin.y + oy, w2, h2);
                            if let Ok(mut guard) = hosts().lock()
                                && let Some(slot) = guard.get_mut(&host_raw)
                            {
                                if slot.closing {
                                    break;
                                }
                                slot.allowed = want;
                            }
                            if IsIconic(parent_h) != 0 {
                                ShowWindow(host_raw as Hwnd, 0);
                            } else {
                                let mut r = RectWin {
                                    l: 0,
                                    t: 0,
                                    r: 0,
                                    b: 0,
                                };
                                GetWindowRect(host_raw as Hwnd, &mut r);
                                if (r.l, r.t, r.r - r.l, r.b - r.t) != want {
                                    SetWindowPos(
                                        host_raw as Hwnd,
                                        std::ptr::null_mut(),
                                        want.0,
                                        want.1,
                                        want.2,
                                        want.3,
                                        SWP_NOACTIVATE | SWP_NOZORDER,
                                    );
                                }
                            }
                        }
                        std::thread::sleep(std::time::Duration::from_millis(120));
                    }
                })
            };
            Ok(Self {
                child: Some(child),
                host,
                pipe,
                parent: parent as Hwnd,
                scale,
                rect,
                last_rect: (x, y, w, h),
                stop_flag,
                progress: progress_arc,
                shared_rect,
                poll: Some(poll),
            })
        }
        pub fn reposition(&mut self, scale: f32, rect: Rect) {
            self.scale = scale;
            self.rect = rect;
            *self.shared_rect.lock().unwrap() = phys(scale, rect);
            let r = screen_rect(self.parent, scale, rect);
            if r != self.last_rect {
                self.last_rect = r;
                if let Ok(mut guard) = hosts().lock()
                    && let Some(slot) = guard.get_mut(&(self.host as isize))
                    && !slot.closing
                {
                    slot.allowed = r;
                }
                unsafe {
                    SetWindowPos(
                        self.host,
                        std::ptr::null_mut(),
                        r.0,
                        r.1,
                        r.2,
                        r.3,
                        SWP_NOACTIVATE | SWP_NOZORDER,
                    )
                };
            }
        }
        pub fn progress(&self) -> (f64, f64) {
            *self.progress.lock().unwrap()
        }
        pub fn is_alive(&mut self) -> bool {
            match self.child.as_mut() {
                Some(child) => matches!(child.try_wait(), Ok(None)),
                None => false,
            }
        }
        pub fn set_progress_pos(&self, seconds: f64) {
            let mut p = self.progress.lock().unwrap();
            p.0 = seconds;
        }
        pub fn ipc(&mut self, json: &str) {
            // One short-lived pipe connection per command; mpv accepts many clients.
            for _ in 0..3 {
                match std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&self.pipe)
                {
                    Ok(mut f) => {
                        use std::io::Write;
                        let _ = f.write_all(json.as_bytes());
                        let _ = f.write_all(b"\n");
                        return;
                    }
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(60)),
                }
            }
        }
        pub fn stop_inner(&mut self) {
            // Mark closing before mpv can unparent the wid. Unparent
            // posts a move to the restored top-level rect (often the
            // monitor). The window proc turns that move into a hide.
            self.stop_flag
                .store(true, std::sync::atomic::Ordering::Relaxed);
            let hwnd = self.host as isize;
            if let Ok(mut guard) = hosts().lock()
                && let Some(slot) = guard.get_mut(&hwnd)
            {
                slot.closing = true;
            }
            if !self.host.is_null() {
                bury_children(self.host, self.last_rect);
                bury_window(self.host, self.last_rect);
                trace_hwnd(hwnd, "hide", self.last_rect);
            }
            if let Some(mut c) = self.child.take() {
                let _ = c.kill();
                std::thread::spawn(move || {
                    let _ = c.wait();
                });
            }
            if !self.host.is_null() {
                trace_hwnd(hwnd, "destroy", self.last_rect);
                unsafe { DestroyWindow(self.host) };
                self.host = std::ptr::null_mut();
            }
            hosts().lock().unwrap().remove(&hwnd);
        }
    }
    impl Drop for Player {
        fn drop(&mut self) {
            self.stop_inner();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CLOSE_BAND, HostGate, HostPhase, PlayGen, begin_playback, callback_is_current,
        clamp_window_change, close_button_rect, dismiss_playback, embed_rect,
        host_create_style, host_style_starts_hidden, playback_args, rects_overlap,
        volume_commands, volume_percent, D3D11_FLIP_OFF, hidden_restore,
        HOST_STYLE_CLIP_CHILDREN, HOST_STYLE_VISIBLE,
    };

    #[test]
    fn embed_rect_is_the_panel_and_waits_for_a_real_viewport() {
        assert!(embed_rect(0.0, 0.0, false).is_none());
        assert!(embed_rect(100.0, 800.0, false).is_none());
        let rect = embed_rect(1600.0, 900.0, false).unwrap();
        assert_eq!(rect.2, 480.0);
        assert_eq!(rect.0, 1120.0);
        assert_eq!(rect.1, 56.0 + CLOSE_BAND);
        assert!(rect.2 < 1600.0);
        assert!((rect.3 - (740.0 - CLOSE_BAND)).abs() < 0.1);
        let again = embed_rect(1600.0, 900.0, false).unwrap();
        assert_eq!(rect, again);
        let close = close_button_rect(1600.0, 900.0, false).unwrap();
        assert!(!rects_overlap(rect, close));
        assert!(close.2 >= 36.0 && close.3 >= 36.0);
        assert!(close.0 + close.2 <= 1600.0);
        assert!(close.1 >= 56.0);
        let filled = embed_rect(1600.0, 900.0, true).unwrap();
        assert_eq!(filled.2, 1600.0);
        assert!(filled.3 <= 900.0);
    }

    #[test]
    fn dismiss_invalidates_the_open_callback_once() {
        let mut generation = 0u64;
        let opened = begin_playback(&mut generation);
        assert!(callback_is_current(generation, opened));
        let closed = dismiss_playback(&mut generation);
        assert!(!callback_is_current(generation, opened));
        assert!(callback_is_current(generation, closed));
        assert_ne!(opened, closed);
        let _ = PlayGen(generation);
        assert!(!callback_is_current(generation, opened));
    }

    #[test]
    fn close_button_uses_the_same_dismiss_as_escape() {
        let mut generation = 0u64;
        let opened = begin_playback(&mut generation);
        let closed = dismiss_playback(&mut generation);
        assert!(!callback_is_current(generation, opened));
        assert!(callback_is_current(generation, closed));
        assert!(close_button_rect(1280.0, 800.0, false).is_some());
    }

    #[test]
    fn fullscreen_request_is_clamped_before_it_can_paint() {
        let panel = (1400, 80, 480, 600);
        let monitor = (0, 0, 1920, 1080);
        let open = clamp_window_change(panel, monitor, false);
        assert_eq!((open.x, open.y, open.w, open.h), panel);
        assert!(!open.hide);
        let closing = clamp_window_change(panel, monitor, true);
        assert!(closing.hide);
        assert_eq!((closing.x, closing.y, closing.w, closing.h), panel);
    }

    #[test]
    fn host_cannot_show_before_place_or_destroy_while_shown() {
        let mut gate = HostGate::hidden((10, 20, 480, 600));
        assert!(gate.show().is_err());
        assert_eq!(gate.place().unwrap(), (10, 20, 480, 600));
        assert!(gate.show().is_ok());
        assert_eq!(gate.phase, HostPhase::Shown);
        assert!(gate.destroy().is_err());
        assert!(gate.hide_for_close().is_ok());
        assert!(gate.destroy().is_ok());
        assert_eq!(gate.phase, HostPhase::Destroyed);
    }

    #[test]
    fn host_is_created_hidden_and_clips_children() {
        let style = host_create_style();
        assert!(host_style_starts_hidden(style));
        assert_eq!(style & HOST_STYLE_VISIBLE, 0);
        assert_ne!(style & HOST_STYLE_CLIP_CHILDREN, 0);
    }

    #[test]
    fn volume_slider_maps_to_player_commands() {
        assert_eq!(volume_percent(10.0, 10.0, 200.0), 0);
        assert_eq!(volume_percent(210.0, 10.0, 200.0), 100);
        assert_eq!(volume_percent(110.0, 10.0, 200.0), 50);
        assert_eq!(volume_percent(-10.0, 10.0, 200.0), 0);
        let silent = volume_commands(0);
        assert!(silent[0].contains("\"volume\",0"));
        assert!(silent[1].contains("\"mute\",true"));
        let mid = volume_commands(40);
        assert!(mid[0].contains("\"volume\",40"));
        assert!(mid[1].contains("\"mute\",false"));
        let args = playback_args(Some(1), None, None, 40);
        assert!(args.iter().any(|arg| arg == "--media-controls=no"));
        assert!(args.iter().any(|arg| arg == "--volume=40"));
        assert!(args.iter().any(|arg| arg == "--mute=no"));
        assert!(args.iter().all(|arg| arg != "--ao=null"));
        let quiet = playback_args(None, None, None, 0);
        assert!(quiet.iter().any(|arg| arg == "--mute=yes"));
        assert!(quiet.iter().any(|arg| arg == "--volume=0"));
    }

    #[test]
    fn stop_restores_the_panel_rect_hidden() {
        let panel = (2276, 655, 600, 770);
        let (show_cmd, rect) = hidden_restore(panel);
        assert_eq!(show_cmd, 0);
        assert_eq!(rect, panel);
        assert!(rect.2 < 1920 && rect.3 < 1080);
        assert_eq!(D3D11_FLIP_OFF, "--d3d11-flip=no");
    }
}
