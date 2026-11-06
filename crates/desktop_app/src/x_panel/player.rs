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

pub const TRACE_LINE_CAP: u32 = 100;
pub const TRACE_RECT_CAP: u32 = 50;
/// Other processes stop being recorded after this many lines so the host
/// and the player process still have room through Stop.
pub const TRACE_OTHER_CAP: u32 = 80;

pub const SWP_NOSIZE: u32 = 0x0001;
pub const SWP_NOMOVE: u32 = 0x0002;
pub const SWP_NOZORDER: u32 = 0x0004;
pub const SWP_NOREDRAW: u32 = 0x0008;
pub const SWP_NOACTIVATE_F: u32 = 0x0010;
pub const SWP_FRAMECHANGED: u32 = 0x0020;
pub const SWP_SHOWWINDOW_F: u32 = 0x0040;
pub const SWP_HIDEWINDOW_F: u32 = 0x0080;
pub const SWP_NOCOPYBITS: u32 = 0x0100;
pub const SWP_NOOWNERZORDER: u32 = 0x0200;
pub const SWP_NOSENDCHANGING: u32 = 0x0400;
pub const SWP_DEFERERASE: u32 = 0x2000;
pub const SWP_ASYNCWINDOWPOS: u32 = 0x4000;

pub fn event_name(code: u32) -> &'static str {
    match code {
        0x8000 => "CREATE",
        0x8001 => "DESTROY",
        0x8002 => "SHOW",
        0x8003 => "HIDE",
        0x800B => "LOCATION",
        0x8017 => "CLOAK",
        0x8018 => "UNCLOAK",
        _ => "OTHER",
    }
}

pub fn swp_names(flags: u32) -> String {
    const BITS: [(u32, &str); 13] = [
        (SWP_NOSIZE, "SWP_NOSIZE"),
        (SWP_NOMOVE, "SWP_NOMOVE"),
        (SWP_NOZORDER, "SWP_NOZORDER"),
        (SWP_NOREDRAW, "SWP_NOREDRAW"),
        (SWP_NOACTIVATE_F, "SWP_NOACTIVATE"),
        (SWP_FRAMECHANGED, "SWP_FRAMECHANGED"),
        (SWP_SHOWWINDOW_F, "SWP_SHOWWINDOW"),
        (SWP_HIDEWINDOW_F, "SWP_HIDEWINDOW"),
        (SWP_NOCOPYBITS, "SWP_NOCOPYBITS"),
        (SWP_NOOWNERZORDER, "SWP_NOOWNERZORDER"),
        (SWP_NOSENDCHANGING, "SWP_NOSENDCHANGING"),
        (SWP_DEFERERASE, "SWP_DEFERERASE"),
        (SWP_ASYNCWINDOWPOS, "SWP_ASYNCWINDOWPOS"),
    ];
    let mut names = Vec::new();
    for (bit, name) in BITS {
        if flags & bit != 0 {
            names.push(name);
        }
    }
    if names.is_empty() {
        "0".to_string()
    } else {
        names.join("|")
    }
}

/// `requested=0,0,0,0` is not a move when both of these flags are set.
pub fn pos_is_noop(flags: u32) -> bool {
    flags & SWP_NOSIZE != 0 && flags & SWP_NOMOVE != 0
}

pub fn admit_trace_line(pid: u32, host_pid: u32, child_pid: u32, lines: u32) -> bool {
    if lines >= TRACE_LINE_CAP {
        return false;
    }
    // pid 0 is a window that is already going away. Keep those lines
    // through the same cap as the host so Destroy is not dropped.
    if pid == 0 || pid == host_pid || (child_pid != 0 && pid == child_pid) {
        return true;
    }
    lines < TRACE_OTHER_CAP
}

pub fn keep_main_window(class: &str, rect_changed: bool) -> bool {
    class != "Zed::Window" || rect_changed
}

pub fn rect_changed(prev: Option<(i32, i32, i32, i32)>, next: (i32, i32, i32, i32)) -> bool {
    prev != Some(next)
}

/// True when width or height moves by more than half the created size.
/// For the logged host that created size is 600×770.
pub fn live_size_differs_half(live_w: i32, live_h: i32, base_w: i32, base_h: i32) -> bool {
    let over = |live: i32, base: i32| base > 0 && (live - base).unsigned_abs() > (base as u32) / 2;
    over(live_w, base_w) || over(live_h, base_h)
}

/// Fixed phase names. Anything else is dropped so a title, typed text,
/// or URL cannot be written into the trace.
pub fn marker_name(name: &str) -> Option<&'static str> {
    match name {
        "play" => Some("play"),
        "stop" => Some("stop"),
        "esc" => Some("esc"),
        "close" => Some("close"),
        "eof" => Some("eof"),
        "hook" => Some("hook"),
        "hookfail" => Some("hookfail"),
        _ => None,
    }
}

pub fn sanitize_token(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.chars().take(48) {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | ':' | '-') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() { "_".into() } else { out }
}

pub fn format_obs_line(
    source: &str,
    event: &str,
    class: &str,
    flags: u32,
    bounds: (i32, i32, i32, i32),
    live: (i32, i32, i32, i32),
) -> String {
    format!(
        "source={} event={} class={} flags={:#x} swp={} bounds={},{},{},{} live={},{},{},{}",
        sanitize_token(source),
        sanitize_token(event),
        sanitize_token(class),
        flags,
        swp_names(flags),
        bounds.0,
        bounds.1,
        bounds.2,
        bounds.3,
        live.0,
        live.1,
        live.2,
        live.3,
    )
}

pub fn note_player_phase(phase: &str) {
    #[cfg(windows)]
    win::note_phase(phase);
    #[cfg(not(windows))]
    let _ = phase;
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
    const WM_DESTROY: u32 = 0x0002;
    const WM_MOVE: u32 = 0x0003;
    const WM_SIZE: u32 = 0x0005;
    const WM_SHOWWINDOW: u32 = 0x0018;
    const WM_WINDOWPOSCHANGING: u32 = 0x0046;
    const WM_WINDOWPOSCHANGED: u32 = 0x0047;
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
        observe("checkpoint", event, hwnd as Hwnd, Some(bounds), None);
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
        fn GetParent(h: Hwnd) -> Hwnd;
        fn GetAncestor(h: Hwnd, flags: u32) -> Hwnd;
        fn GetWindowThreadProcessId(h: Hwnd, pid: *mut u32) -> u32;
        fn GetWindowLongPtrW(h: Hwnd, index: i32) -> isize;
        fn IsWindowVisible(h: Hwnd) -> i32;
        fn GetDpiForWindow(h: Hwnd) -> u32;
        fn MonitorFromWindow(h: Hwnd, flags: u32) -> Hwnd;
        fn GetMonitorInfoW(monitor: Hwnd, info: *mut MonitorInfo) -> i32;
        fn SetWinEventHook(
            min: u32,
            max: u32,
            module: *mut core::ffi::c_void,
            proc: Option<
                unsafe extern "system" fn(*mut core::ffi::c_void, u32, Hwnd, i32, i32, u32, u32),
            >,
            pid: u32,
            tid: u32,
            flags: u32,
        ) -> *mut core::ffi::c_void;
        fn PeekMessageW(msg: *mut Msg, hwnd: Hwnd, min: u32, max: u32, remove: u32) -> i32;
        fn TranslateMessage(msg: *const Msg) -> i32;
        fn DispatchMessageW(msg: *const Msg) -> Lresult;
        fn GetClientRect(h: Hwnd, r: *mut RectWin) -> i32;
        fn IsZoomed(h: Hwnd) -> i32;
        fn IsWindow(h: Hwnd) -> i32;
        fn UnhookWinEvent(hook: *mut core::ffi::c_void) -> i32;
        fn EnumWindows(
            proc: Option<unsafe extern "system" fn(Hwnd, isize) -> i32>,
            param: isize,
        ) -> i32;
        fn EnumChildWindows(
            parent: Hwnd,
            proc: Option<unsafe extern "system" fn(Hwnd, isize) -> i32>,
            param: isize,
        ) -> i32;
    }
    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmGetWindowAttribute(
            hwnd: Hwnd,
            attr: u32,
            value: *mut core::ffi::c_void,
            size: u32,
        ) -> i32;
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
    #[repr(C)]
    struct MonitorInfo {
        cb_size: u32,
        rc_monitor: RectWin,
        rc_work: RectWin,
        flags: u32,
    }
    #[repr(C)]
    struct Msg {
        hwnd: Hwnd,
        message: u32,
        _pad_msg: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        pt: Point,
        lprivate: u32,
    }

    fn mono_us() -> u64 {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        START
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_micros() as u64
    }

    fn wall_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    #[derive(Clone, Copy)]
    struct Seen {
        rect: (i32, i32, i32, i32),
        vis: u8,
        cloak: u32,
        zoomed: u8,
        rects: u32,
    }

    struct GenState {
        generation: u64,
        armed: bool,
        host: u64,
        host_pid: u32,
        child_pid: u32,
        base_w: i32,
        base_h: i32,
        started: std::time::Instant,
        lines: u32,
        seq: u64,
        seen: std::collections::HashMap<u64, Seen>,
        main_seen: bool,
        main_rect: Option<(i32, i32, i32, i32)>,
        noop_logged: bool,
        enum_show: bool,
        enum_big: bool,
        hooks: Vec<usize>,
    }

    fn gen_state() -> &'static std::sync::Mutex<GenState> {
        static STATE: std::sync::OnceLock<std::sync::Mutex<GenState>> = std::sync::OnceLock::new();
        STATE.get_or_init(|| {
            std::sync::Mutex::new(GenState {
                generation: 0,
                armed: false,
                host: 0,
                host_pid: 0,
                child_pid: 0,
                base_w: 0,
                base_h: 0,
                started: std::time::Instant::now(),
                lines: 0,
                seq: 0,
                seen: std::collections::HashMap::new(),
                main_seen: false,
                main_rect: None,
                noop_logged: false,
                enum_show: false,
                enum_big: false,
                hooks: Vec::new(),
            })
        })
    }

    static HOOK_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    struct PosRequest {
        flags: u32,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        insert: isize,
    }

    struct Sample {
        pid: u32,
        tid: u32,
        class: String,
        parent: u64,
        owner: u64,
        root: u64,
        style: u32,
        exstyle: u32,
        live: (i32, i32, i32, i32),
        client: (i32, i32, i32, i32),
        parent_rect: (i32, i32, i32, i32),
        vis: u8,
        iconic: u8,
        zoomed: u8,
        cloak: u32,
        dpi: u32,
        mon: (i32, i32, i32, i32),
    }

    fn rect_of(raw: RectWin) -> (i32, i32, i32, i32) {
        (
            raw.l,
            raw.t,
            raw.r.saturating_sub(raw.l),
            raw.b.saturating_sub(raw.t),
        )
    }

    fn read_rect(hwnd: Hwnd) -> (i32, i32, i32, i32) {
        let mut raw = RectWin {
            l: 0,
            t: 0,
            r: 0,
            b: 0,
        };
        unsafe { GetWindowRect(hwnd, &mut raw) };
        rect_of(raw)
    }

    fn sample_hwnd(hwnd: Hwnd) -> Sample {
        if hwnd.is_null() {
            return Sample {
                pid: 0,
                tid: 0,
                class: String::new(),
                parent: 0,
                owner: 0,
                root: 0,
                style: 0,
                exstyle: 0,
                live: (0, 0, 0, 0),
                client: (0, 0, 0, 0),
                parent_rect: (0, 0, 0, 0),
                vis: 0,
                iconic: 0,
                zoomed: 0,
                cloak: 0,
                dpi: 0,
                mon: (0, 0, 0, 0),
            };
        }
        let mut pid = 0u32;
        let tid = unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        let mut class_buf = [0u16; 48];
        let n = unsafe { GetClassNameW(hwnd, class_buf.as_mut_ptr(), 48) };
        let class = if n > 0 {
            String::from_utf16_lossy(&class_buf[..n as usize])
        } else {
            String::new()
        };
        let parent = unsafe { GetParent(hwnd) };
        let owner = unsafe { GetWindow(hwnd, 4) };
        let root = unsafe { GetAncestor(hwnd, 2) };
        let mut client = RectWin {
            l: 0,
            t: 0,
            r: 0,
            b: 0,
        };
        unsafe { GetClientRect(hwnd, &mut client) };
        let mut cloaked = 0xffffu32;
        let hr = unsafe {
            DwmGetWindowAttribute(
                hwnd,
                14,
                &mut cloaked as *mut u32 as *mut core::ffi::c_void,
                4,
            )
        };
        if hr != 0 {
            cloaked = 0xffff;
        }
        let mut mon = RectWin {
            l: 0,
            t: 0,
            r: 0,
            b: 0,
        };
        let hmon = unsafe { MonitorFromWindow(hwnd, 2) };
        if !hmon.is_null() {
            let mut info = MonitorInfo {
                cb_size: std::mem::size_of::<MonitorInfo>() as u32,
                rc_monitor: RectWin {
                    l: 0,
                    t: 0,
                    r: 0,
                    b: 0,
                },
                rc_work: RectWin {
                    l: 0,
                    t: 0,
                    r: 0,
                    b: 0,
                },
                flags: 0,
            };
            if unsafe { GetMonitorInfoW(hmon, &mut info) } != 0 {
                mon = info.rc_monitor;
            }
        }
        Sample {
            pid,
            tid,
            class,
            parent: parent as usize as u64,
            owner: owner as usize as u64,
            root: root as usize as u64,
            style: unsafe { GetWindowLongPtrW(hwnd, -16) as u32 },
            exstyle: unsafe { GetWindowLongPtrW(hwnd, -20) as u32 },
            live: read_rect(hwnd),
            client: (
                client.l,
                client.t,
                client.r.saturating_sub(client.l),
                client.b.saturating_sub(client.t),
            ),
            parent_rect: if parent.is_null() {
                (0, 0, 0, 0)
            } else {
                read_rect(parent)
            },
            vis: unsafe { if IsWindowVisible(hwnd) != 0 { 1 } else { 0 } },
            iconic: unsafe { if IsIconic(hwnd) != 0 { 1 } else { 0 } },
            zoomed: unsafe { if IsZoomed(hwnd) != 0 { 1 } else { 0 } },
            cloak: cloaked,
            dpi: unsafe { GetDpiForWindow(hwnd) },
            mon: (mon.l, mon.t, mon.r, mon.b),
        }
    }

    fn append_line(line: &str) {
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

    fn write_line(
        source: &str,
        event: &str,
        hwnd: u64,
        sample: &Sample,
        bounds: (i32, i32, i32, i32),
        pos: Option<&PosRequest>,
        generation: u64,
        t_ms: u64,
        seq: u64,
        spawn_pid: u32,
    ) {
        let pos_fields = match pos {
            Some(pos) => format!(
                "flags={:#x} swp={} req={},{},{},{} insert={:#x}",
                pos.flags,
                super::swp_names(pos.flags),
                pos.x,
                pos.y,
                pos.cx,
                pos.cy,
                pos.insert as usize
            ),
            None => "flags=na swp=na req=na insert=na".to_string(),
        };
        let head = if source == "checkpoint" {
            format!(
                "t_us={} termy-video hwnd={hwnd:#x} event={event} bounds={},{},{},{}",
                mono_us(),
                bounds.0,
                bounds.1,
                bounds.2,
                bounds.3
            )
        } else {
            format!(
                "t_us={} source={} event={} hwnd={hwnd:#x}",
                mono_us(),
                super::sanitize_token(source),
                super::sanitize_token(event)
            )
        };
        let line = format!(
            "{head} wall_ms={} gen={generation} t_ms={t_ms} seq={seq} class={} pid={} tid={} parent={:#x} owner={:#x} root={:#x} style={:#x} ex={:#x} {pos_fields} live={},{},{},{} client={},{},{},{} parent_rect={},{},{},{} vis={} iconic={} zoomed={} cloak={} dpi={} mon={},{},{},{} spawn_pid={spawn_pid}",
            super::sanitize_token(&sample.class),
            sample.pid,
            sample.tid,
            sample.parent,
            sample.owner,
            sample.root,
            sample.style,
            sample.exstyle,
            sample.live.0,
            sample.live.1,
            sample.live.2,
            sample.live.3,
            sample.client.0,
            sample.client.1,
            sample.client.2,
            sample.client.3,
            sample.parent_rect.0,
            sample.parent_rect.1,
            sample.parent_rect.2,
            sample.parent_rect.3,
            sample.vis,
            sample.iconic,
            sample.zoomed,
            sample.cloak,
            sample.dpi,
            sample.mon.0,
            sample.mon.1,
            sample.mon.2,
            sample.mon.3,
        );
        append_line(&line);
    }

    fn observe(
        source: &str,
        event: &str,
        hwnd: Hwnd,
        bounds: Option<(i32, i32, i32, i32)>,
        pos: Option<PosRequest>,
    ) {
        let sample = sample_hwnd(hwnd);
        let key = hwnd as usize as u64;
        let live = sample.live;
        let stored = bounds.unwrap_or(live);
        let location = event == "LOCATION";
        let mut trigger_enum = false;
        let mut host_pid = 0u32;
        let mut child_pid = 0u32;
        let mut host = 0u64;
        let snapshot = {
            let Ok(mut guard) = gen_state().lock() else {
                return;
            };
            if !guard.armed && source != "marker" {
                return;
            }
            if sample.class == "Zed::Window" {
                let log = guard.main_seen && super::rect_changed(guard.main_rect, live);
                guard.main_seen = true;
                guard.main_rect = Some(live);
                if !log {
                    return;
                }
            }
            if location {
                let prev = guard.seen.get(&key).copied();
                let changed = match prev {
                    None => true,
                    Some(prev) => {
                        super::rect_changed(Some(prev.rect), live)
                            || prev.vis != sample.vis
                            || prev.cloak != sample.cloak
                            || prev.zoomed != sample.zoomed
                    }
                };
                if !changed {
                    return;
                }
                let rects = prev.map(|prev| prev.rects).unwrap_or(0);
                if super::rect_changed(prev.map(|prev| prev.rect), live)
                    && rects >= super::TRACE_RECT_CAP
                {
                    return;
                }
            }
            if source != "marker"
                && !super::admit_trace_line(
                    sample.pid,
                    guard.host_pid,
                    guard.child_pid,
                    guard.lines,
                )
            {
                if guard.lines >= super::TRACE_LINE_CAP {
                    HOOK_ON.store(false, std::sync::atomic::Ordering::Release);
                }
                return;
            }
            let prev = guard.seen.get(&key).copied();
            let rects = prev.map(|prev| prev.rects).unwrap_or(0);
            let rects = if super::rect_changed(prev.map(|prev| prev.rect), live) {
                rects.saturating_add(1)
            } else {
                rects
            };
            guard.seen.insert(
                key,
                Seen {
                    rect: live,
                    vis: sample.vis,
                    cloak: sample.cloak,
                    zoomed: sample.zoomed,
                    rects,
                },
            );
            if source != "marker" {
                guard.lines = guard.lines.saturating_add(1);
                if guard.lines >= super::TRACE_LINE_CAP {
                    HOOK_ON.store(false, std::sync::atomic::Ordering::Release);
                }
            }
            guard.seq = guard.seq.saturating_add(1);
            if source != "enum"
                && source != "winevent"
                && !guard.enum_big
                && super::live_size_differs_half(live.2, live.3, guard.base_w, guard.base_h)
            {
                guard.enum_big = true;
                trigger_enum = true;
                host_pid = guard.host_pid;
                child_pid = guard.child_pid;
                host = guard.host;
            }
            let generation = guard.generation;
            let t_ms = guard.started.elapsed().as_millis() as u64;
            let seq = guard.seq;
            let spawn_pid = guard.child_pid;
            (generation, t_ms, seq, spawn_pid)
        };
        let (generation, t_ms, seq, spawn_pid) = snapshot;
        write_line(
            source,
            event,
            key,
            &sample,
            stored,
            pos.as_ref(),
            generation,
            t_ms,
            seq,
            spawn_pid,
        );
        if trigger_enum {
            enumerate_players(host_pid, child_pid, host);
        }
    }

    struct EnumJob {
        host_pid: u32,
        child_pid: u32,
        host: u64,
        children: bool,
        left: u32,
    }

    unsafe extern "system" fn enum_cb(hwnd: Hwnd, param: isize) -> i32 {
        let job = unsafe { &mut *(param as *mut EnumJob) };
        if job.left == 0 {
            return 0;
        }
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        let key = hwnd as usize as u64;
        let interesting =
            job.children || pid == job.child_pid || key == job.host || pid == job.host_pid;
        if !interesting {
            return 1;
        }
        job.left = job.left.saturating_sub(1);
        observe("enum", "ENUM", hwnd, None, None);
        if !job.children && (pid == job.child_pid || key == job.host) {
            let mut child = EnumJob {
                host_pid: job.host_pid,
                child_pid: job.child_pid,
                host: job.host,
                children: true,
                left: job.left,
            };
            unsafe { EnumChildWindows(hwnd, Some(enum_cb), &mut child as *mut EnumJob as isize) };
            job.left = child.left;
        }
        if job.left == 0 { 0 } else { 1 }
    }

    fn enumerate_players(host_pid: u32, child_pid: u32, host: u64) {
        let mut job = EnumJob {
            host_pid,
            child_pid,
            host,
            children: false,
            left: 24,
        };
        unsafe { EnumWindows(Some(enum_cb), &mut job as *mut EnumJob as isize) };
        if job.left == 0 {
            append_line(&format!(
                "t_us={} source=enum event=ENUM_TRUNC gen={}",
                mono_us(),
                gen_state().lock().map(|g| g.generation).unwrap_or(0)
            ));
        }
    }

    pub fn note_phase(phase: &str) {
        let Some(name) = super::marker_name(phase) else {
            return;
        };
        let (generation, t_ms, seq) = gen_state()
            .lock()
            .map(|mut guard| {
                guard.seq = guard.seq.saturating_add(1);
                (
                    guard.generation,
                    guard.started.elapsed().as_millis() as u64,
                    guard.seq,
                )
            })
            .unwrap_or((0, 0, 0));
        append_line(&format!(
            "t_us={} source=marker event=MARKER:{name} wall_ms={} gen={generation} t_ms={t_ms} seq={seq} class=_ pid=0 tid=0",
            mono_us(),
            wall_ms()
        ));
    }

    pub fn set_player_pid(pid: u32) {
        if let Ok(mut guard) = gen_state().lock() {
            guard.child_pid = pid;
        }
    }

    fn ensure_trace_thread() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let _ = std::thread::Builder::new()
                .name("termy-video-trace".into())
                .spawn(trace_thread);
        });
    }

    pub fn begin_generation(host: Hwnd, width: i32, height: i32) {
        let generation = {
            let Ok(mut guard) = gen_state().lock() else {
                return;
            };
            guard.generation = guard.generation.saturating_add(1);
            guard.armed = true;
            guard.host = host as usize as u64;
            guard.host_pid = std::process::id();
            guard.child_pid = 0;
            guard.base_w = width;
            guard.base_h = height;
            guard.started = std::time::Instant::now();
            guard.lines = 0;
            guard.seen.clear();
            guard.main_seen = false;
            guard.main_rect = None;
            guard.noop_logged = false;
            guard.enum_show = false;
            guard.enum_big = false;
            guard.generation
        };
        HOOK_ON.store(true, std::sync::atomic::Ordering::Release);
        ensure_trace_thread();
        let host_bits = host as usize as isize;
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(3));
            let gone = unsafe { IsWindow(host_bits as Hwnd) } == 0;
            if !gone {
                return;
            }
            let mut disarm = false;
            if let Ok(mut guard) = gen_state().lock() {
                if guard.generation == generation && guard.armed {
                    guard.armed = false;
                    disarm = true;
                }
            }
            if disarm {
                HOOK_ON.store(false, std::sync::atomic::Ordering::Release);
            }
        });
    }

    pub fn disarm_generation() {
        if let Ok(mut guard) = gen_state().lock() {
            guard.armed = false;
        }
        HOOK_ON.store(false, std::sync::atomic::Ordering::Release);
    }

    pub fn note_show_window(hwnd: Hwnd, shown: bool) {
        observe(
            "wndproc",
            if shown {
                "SHOWWINDOW:1"
            } else {
                "SHOWWINDOW:0"
            },
            hwnd,
            None,
            None,
        );
        if !shown {
            return;
        }
        let job = {
            let Ok(mut guard) = gen_state().lock() else {
                return;
            };
            if !guard.armed || guard.enum_show {
                return;
            }
            guard.enum_show = true;
            (guard.host_pid, guard.child_pid, guard.host)
        };
        enumerate_players(job.0, job.1, job.2);
    }

    pub fn note_pos(hwnd: Hwnd, event: &str, pos: &WindowPos) {
        let noop = super::pos_is_noop(pos.flags);
        if noop {
            let Ok(mut guard) = gen_state().lock() else {
                return;
            };
            if !guard.armed || guard.noop_logged {
                return;
            }
            guard.noop_logged = true;
        }
        observe(
            "wndproc",
            event,
            hwnd,
            None,
            Some(PosRequest {
                flags: pos.flags,
                x: pos.x,
                y: pos.y,
                cx: pos.cx,
                cy: pos.cy,
                insert: pos.insert_after as isize,
            }),
        );
    }

    fn install_hooks() {
        unsafe {
            let mut msg = std::mem::zeroed::<Msg>();
            PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, 0);
            let lifecycle = SetWinEventHook(
                0x8000,
                0x8003,
                std::ptr::null_mut(),
                Some(win_event),
                0,
                0,
                0,
            );
            let location = SetWinEventHook(
                0x800B,
                0x800B,
                std::ptr::null_mut(),
                Some(win_event),
                0,
                0,
                0,
            );
            let cloak = SetWinEventHook(
                0x8017,
                0x8018,
                std::ptr::null_mut(),
                Some(win_event),
                0,
                0,
                0,
            );
            let ok = !lifecycle.is_null() && !location.is_null() && !cloak.is_null();
            if let Ok(mut guard) = gen_state().lock() {
                guard.hooks = vec![lifecycle as usize, location as usize, cloak as usize];
            }
            note_phase(if ok { "hook" } else { "hookfail" });
        }
    }

    fn remove_hooks() {
        let hooks = gen_state()
            .lock()
            .map(|mut guard| std::mem::take(&mut guard.hooks))
            .unwrap_or_default();
        for hook in hooks {
            if hook != 0 {
                unsafe { UnhookWinEvent(hook as *mut core::ffi::c_void) };
            }
        }
    }

    fn trace_thread() {
        let mut hooked = false;
        loop {
            let on = HOOK_ON.load(std::sync::atomic::Ordering::Acquire);
            if on && !hooked {
                install_hooks();
                hooked = true;
            } else if !on && hooked {
                remove_hooks();
                hooked = false;
            }
            unsafe {
                let mut msg = std::mem::zeroed::<Msg>();
                while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, 1) != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(if hooked {
                15
            } else {
                40
            }));
        }
    }

    unsafe extern "system" fn win_event(
        _hook: *mut core::ffi::c_void,
        event: u32,
        hwnd: Hwnd,
        id_object: i32,
        _id_child: i32,
        _event_thread: u32,
        _event_ms: u32,
    ) {
        if hwnd.is_null() || id_object != 0 || !HOOK_ON.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        let name = match event {
            0x8000 => "CREATE",
            0x8001 => "DESTROY",
            0x8002 => "SHOW",
            0x8003 => "HIDE",
            0x800B => "LOCATION",
            0x8017 => "CLOAK",
            0x8018 => "UNCLOAK",
            _ => return,
        };
        observe("winevent", name, hwnd, None, None);
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
        if m == WM_DESTROY {
            observe("wndproc", "DESTROY", h, None, None);
        } else if m == WM_SHOWWINDOW {
            note_show_window(h, w != 0);
        } else if m == WM_SIZE {
            observe("wndproc", "SIZE", h, None, None);
        } else if m == WM_MOVE {
            observe("wndproc", "MOVE", h, None, None);
        } else if m == WM_WINDOWPOSCHANGED && l != 0 {
            let pos = unsafe { &*(l as *const WindowPos) };
            note_pos(h, "WINDOWPOSCHANGED", pos);
        }
        if m == WM_WINDOWPOSCHANGING && l != 0 {
            let pos = unsafe { &*(l as *const WindowPos) };
            note_pos(h, "WINDOWPOSCHANGING", pos);
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
                            "{} flags={:#x} requested={},{},{},{}",
                            if clamped.hide { "clamp-hide" } else { "clamp" },
                            pos.flags,
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
            note_phase("play");
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
            begin_generation(host, w, h);
            let mut gate = super::HostGate::hidden((x, y, w, h));
            trace_hwnd(host as isize, "create-hidden", (x, y, w, h));
            let placed = gate.place().map_err(|err| {
                unsafe { DestroyWindow(host) };
                err.to_string()
            })?;
            hosts().lock().unwrap().insert(
                host as isize,
                HostSlot {
                    allowed: placed,
                    closing: false,
                },
            );
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
            set_player_pid(child.id());
            trace_hwnd(host as isize, "spawn", (x, y, w, h));
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
            note_phase("stop");
            self.stop_flag
                .store(true, std::sync::atomic::Ordering::Relaxed);
            let hwnd = self.host as isize;
            if let Ok(mut guard) = hosts().lock()
                && let Some(slot) = guard.get_mut(&hwnd)
            {
                slot.closing = true;
            }
            if !self.host.is_null() {
                trace_hwnd(hwnd, "hide", self.last_rect);
                unsafe { ShowWindow(self.host, SW_HIDE) };
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
            disarm_generation();
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
        CLOSE_BAND, HOST_STYLE_CLIP_CHILDREN, HOST_STYLE_VISIBLE, HostGate, HostPhase, PlayGen,
        SWP_NOMOVE, SWP_NOSIZE, TRACE_LINE_CAP, TRACE_OTHER_CAP, admit_trace_line, begin_playback,
        callback_is_current, clamp_window_change, close_button_rect, dismiss_playback, embed_rect,
        event_name, format_obs_line, host_create_style, host_style_starts_hidden, keep_main_window,
        live_size_differs_half, marker_name, playback_args, pos_is_noop, rect_changed,
        rects_overlap, sanitize_token, swp_names, volume_commands, volume_percent,
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
        assert!(args.iter().all(|arg| !arg.contains("d3d11")));
        let quiet = playback_args(None, None, None, 0);
        assert!(quiet.iter().any(|arg| arg == "--mute=yes"));
        assert!(quiet.iter().any(|arg| arg == "--volume=0"));
    }

    #[test]
    fn video_trace_separates_requested_rect_from_live_rect() {
        assert!(pos_is_noop(SWP_NOSIZE | SWP_NOMOVE));
        assert!(!pos_is_noop(SWP_NOSIZE));
        assert!(swp_names(SWP_NOSIZE | SWP_NOMOVE).contains("SWP_NOSIZE"));
        assert!(swp_names(SWP_NOSIZE | SWP_NOMOVE).contains("SWP_NOMOVE"));
        assert_eq!(event_name(0x8002), "SHOW");
        assert_eq!(event_name(0x8017), "CLOAK");
        assert!(admit_trace_line(9, 1, 2, 79));
        assert!(!admit_trace_line(9, 1, 2, 80));
        assert!(admit_trace_line(2, 1, 2, 80));
        assert!(admit_trace_line(0, 1, 2, 80));
        assert!(!admit_trace_line(0, 1, 2, TRACE_LINE_CAP));
        assert!(!admit_trace_line(2, 1, 2, TRACE_LINE_CAP));
        assert!(admit_trace_line(1, 1, 2, TRACE_OTHER_CAP));
        assert!(!keep_main_window("Zed::Window", false));
        assert!(keep_main_window("Zed::Window", true));
        assert!(keep_main_window("mpv", false));
        assert!(!rect_changed(Some((1, 2, 3, 4)), (1, 2, 3, 4)));
        assert!(rect_changed(None, (1, 2, 3, 4)));
        assert!(!live_size_differs_half(600, 770, 600, 770));
        assert!(!live_size_differs_half(900, 770, 600, 770));
        assert!(live_size_differs_half(901, 770, 600, 770));
        assert!(live_size_differs_half(1920, 1080, 600, 770));
        assert_eq!(marker_name("esc"), Some("esc"));
        assert_eq!(marker_name("https://example.test/watch"), None);
        let line = format_obs_line(
            "wndproc",
            "WINDOWPOSCHANGING",
            "https://x.test/watch title",
            SWP_NOSIZE | SWP_NOMOVE,
            (2276, 655, 600, 770),
            (0, 0, 3072, 1728),
        );
        assert!(line.contains("bounds=2276,655,600,770"));
        assert!(line.contains("live=0,0,3072,1728"));
        assert!(line.contains("SWP_NOSIZE"));
        assert!(!line.contains('/'));
        let class = line
            .split_whitespace()
            .find(|part| part.starts_with("class="))
            .unwrap();
        assert_eq!(class, "class=https:__x_test_watch_title");
        assert_eq!(sanitize_token("Zed::Window"), "Zed::Window");
    }
}
