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

use std::path::{Path, PathBuf};
use termy_x::model::MediaKind;
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

/// Drawable for libVLC: a child of the Termy window, in the parent's
/// client coordinates, clipped, and not visible until it is placed.
/// This is not VLC's own top-level window.
pub fn host_child_style() -> u32 {
    WS_CHILD | HOST_STYLE_CLIP_SIBLINGS | HOST_STYLE_CLIP_CHILDREN
}

pub fn client_pixels(scale: f32, rect: Rect) -> (i32, i32, i32, i32) {
    (
        (rect.0 * scale).round() as i32,
        (rect.1 * scale).round() as i32,
        (rect.2 * scale).max(1.0).round() as i32,
        (rect.3 * scale).max(1.0).round() as i32,
    )
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

/// Verified local install. Resolution must open this file when it exists,
/// not `Command::new("vlc")`, because VLC is not on PATH.
pub const VLC_CANONICAL_WINDOWS: &str = r"C:\Program Files\VideoLAN\VLC\vlc.exe";

pub const VLC_NOT_FOUND: &str = "VLC was not found at C:\\Program Files\\VideoLAN\\VLC\\vlc.exe. No window was opened. Images stay in Termy.";

pub const VLC_ENGINE_MISSING: &str = "VLC's 64-bit engine (libvlc.dll) was not found beside C:\\Program Files\\VideoLAN\\VLC\\vlc.exe. No window was opened.";

pub const VLC_WRONG_ARCH: &str = "VLC's library beside C:\\Program Files\\VideoLAN\\VLC\\vlc.exe is not 64-bit, so Termy cannot load it. No window was opened.";

pub const VLC_PLUGINS_MISSING: &str = "VLC's plugins folder is missing or empty beside C:\\Program Files\\VideoLAN\\VLC. No window was opened.";

/// `LoadLibraryExW` search limited to the DLL's own directory and System32.
pub const DLL_LOAD_FLAGS: u32 = 0x0000_0100 | 0x0000_0800;

pub const WS_CHILD: u32 = 0x4000_0000;

pub const NO_PLAYABLE_MEDIA: &str = "No playable video or audio file was attached. The image stays in Termy. VLC was not opened.";

pub const VLC_OPEN_NOTICE: &str = "Playing inside Termy with VLC's engine. Esc or \u{2715} stops it. Volume and seek here are Termy's controls. VLC's own window is not opened.";

pub const VLC_CONTROLS_NOTE: &str = "Volume, seek, and pause are Termy's controls on VLC's engine. VLC's own buttons are not inside this panel.";

pub const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;
pub const IMAGE_FILE_MACHINE_I386: u16 = 0x14c;

/// libVLC event types from VLC 3.0 `libvlc_events.h`.
pub const LIBVLC_EVENT_END: u32 = 0x109;
pub const LIBVLC_EVENT_ERROR: u32 = 0x10a;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaDispatch {
    /// In-app photo viewer. Never VLC.
    StillImage,
    /// Playable video or audio. Windows draws it with libVLC inside the panel.
    Vlc,
    /// No still and no playable file.
    Unplayable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MediaUrlClass {
    Still,
    Playable,
    Reject,
}

/// The visible `vlc.exe` window is not a fallback. Windows uses libVLC.
pub fn launches_external_vlc() -> bool {
    false
}

pub fn reap_video_hosts() {
    #[cfg(windows)]
    vlc_embed::reap_hosts();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnginePlan {
    Embed,
    Refuse(&'static str),
}

pub fn pe_machine(bytes: &[u8]) -> Option<u16> {
    if bytes.len() < 0x40 || bytes[0] != b'M' || bytes[1] != b'Z' {
        return None;
    }
    let pe_off = u32::from_le_bytes(bytes[0x3c..0x40].try_into().ok()?) as usize;
    if bytes.len() < pe_off.saturating_add(6) {
        return None;
    }
    if &bytes[pe_off..pe_off + 4] != b"PE\0\0" {
        return None;
    }
    Some(u16::from_le_bytes(
        bytes[pe_off + 4..pe_off + 6].try_into().ok()?,
    ))
}

/// Passing `--plugin-path` does not prove the folder exists. `plugins`
/// is a count of `*_plugin.dll` files, and `plugins_all_amd64` is false
/// when any of those files is not a 64-bit image.
pub fn engine_files_plan(
    libvlc: Option<u16>,
    core: Option<u16>,
    plugins: usize,
    plugins_all_amd64: bool,
) -> EnginePlan {
    if libvlc.is_none() {
        return EnginePlan::Refuse(VLC_ENGINE_MISSING);
    }
    if libvlc != Some(IMAGE_FILE_MACHINE_AMD64) || core != Some(IMAGE_FILE_MACHINE_AMD64) {
        return EnginePlan::Refuse(if core.is_none() && libvlc == Some(IMAGE_FILE_MACHINE_AMD64) {
            VLC_ENGINE_MISSING
        } else {
            VLC_WRONG_ARCH
        });
    }
    if plugins == 0 {
        return EnginePlan::Refuse(VLC_PLUGINS_MISSING);
    }
    if !plugins_all_amd64 {
        return EnginePlan::Refuse(VLC_WRONG_ARCH);
    }
    EnginePlan::Embed
}

/// 64-bit libVLC inside our window, or a refusal. Never `vlc.exe`.
pub fn engine_plan(machine: Option<u16>, playable: bool) -> EnginePlan {
    if !playable {
        return EnginePlan::Refuse(NO_PLAYABLE_MEDIA);
    }
    match machine {
        Some(IMAGE_FILE_MACHINE_AMD64) => EnginePlan::Embed,
        Some(_) => EnginePlan::Refuse(VLC_WRONG_ARCH),
        None => EnginePlan::Refuse(VLC_ENGINE_MISSING),
    }
}

pub fn libvlc_instance_args(plugin_path: &str) -> Vec<String> {
    vec![
        "--intf=dummy".to_string(),
        "--dummy-quiet".to_string(),
        "--ignore-config".to_string(),
        "--no-video-title-show".to_string(),
        format!("--plugin-path={plugin_path}"),
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnginePhase {
    Idle,
    HostReady,
    Playing,
    Stopping,
    Released,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineStep {
    PlaceHost,
    Play,
    Stop,
    Release,
}

pub fn engine_step(phase: EnginePhase, step: EngineStep) -> Result<EnginePhase, &'static str> {
    use EnginePhase::{HostReady, Idle, Playing, Released, Stopping};
    use EngineStep::{PlaceHost, Play, Release, Stop};
    match (phase, step) {
        (Idle, PlaceHost) => Ok(HostReady),
        (HostReady, Play) => Ok(Playing),
        (Idle, Play) => Err("play before the panel surface exists"),
        (Playing, Stop) | (HostReady, Stop) => Ok(Stopping),
        (Stopping, Release) | (Released, Stop) | (Released, Release) => Ok(Released),
        _ => Err("invalid engine transition"),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EngineFlags {
    pub ended: bool,
    pub failed: bool,
}

/// LibVLC callbacks may run on a VLC thread. They record state only when
/// the generation still matches, and they do not call back into the UI.
pub fn apply_engine_callback(live: u64, event_gen: u64, kind: u32, flags: &mut EngineFlags) {
    if live != event_gen {
        return;
    }
    if kind == LIBVLC_EVENT_END {
        flags.ended = true;
    } else if kind == LIBVLC_EVENT_ERROR {
        flags.failed = true;
    }
}

pub const ENGINE_RELEASE_ORDER: &[&str] = &[
    "hide-host",
    "detach-callback",
    "pause-audio",
    "clear-hwnd",
    "blocking-stop",
    "release-player",
    "destroy-host",
];

/// libVLC setup runs on the thread that created the drawable.
pub const START_ON_WINDOW_THREAD: bool = true;

/// `Player::start` is called from a GPUI update, which still holds the
/// panel borrow. Pumping the thread queue there re-enters GPUI through
/// `DispatchMessage`. A panic on that path cannot unwind out of the
/// Win32 frame and `abort()`s. Windows records that as `0xc0000409`
/// in `ucrtbase.dll` (the installed 0278d94 crashes).
pub const DISPATCH_DURING_START: bool = false;

pub const LIBVLC_START_ORDER: &[&str] = &[
    "place-child",
    "require-clip-children",
    "libvlc-new-on-window-thread",
    "set-hwnd",
    "play",
    "show-host",
];

/// True when this start would take the fail-fast path seen on the
/// installed build: dispatching window messages during start, or
/// handing VLC 3.0.24 a drawable without `WS_CLIPCHILDREN` (the vout
/// asserts that style and aborts when the assert is compiled in).
pub fn start_would_abort_in_ucrt(dispatch_during_start: bool, clip_children: bool) -> bool {
    dispatch_during_start || !clip_children
}

pub fn surface_style_for_libvlc(style: u32) -> Result<(), &'static str> {
    if style & HOST_STYLE_POPUP != 0 || style & WS_CHILD == 0 {
        return Err("video surface must stay a child of the Termy window");
    }
    if style & HOST_STYLE_CLIP_CHILDREN == 0 {
        return Err("video surface is missing WS_CLIPCHILDREN, so VLC would abort");
    }
    Ok(())
}

/// `argc` counts real arguments. The pointer list also ends in NULL so
/// a libVLC build that walks `argv` cannot run off the end into ucrt.
pub fn libvlc_argc(argument_count: usize, terminator_present: bool) -> Result<i32, &'static str> {
    if !terminator_present {
        return Err("libvlc argv is missing its NULL terminator");
    }
    i32::try_from(argument_count).map_err(|_| "libvlc argc does not fit")
}

pub fn dispatch_media(kind: MediaKind, url: &str) -> MediaDispatch {
    if kind == MediaKind::Photo {
        return MediaDispatch::StillImage;
    }
    match media_url_class(url) {
        MediaUrlClass::Still => MediaDispatch::StillImage,
        MediaUrlClass::Playable => MediaDispatch::Vlc,
        MediaUrlClass::Reject => MediaDispatch::Unplayable,
    }
}

pub fn is_still_image_url(url: &str) -> bool {
    media_url_class(url) == MediaUrlClass::Still
}

fn media_url_class(url: &str) -> MediaUrlClass {
    let url = url.trim();
    if url.is_empty() || url.chars().any(|c| c.is_control() || c == '\u{0}') {
        return MediaUrlClass::Reject;
    }
    let lower = url.to_ascii_lowercase();
    if is_status_page(&lower) {
        return MediaUrlClass::Reject;
    }
    let path = lower.split(['?', '#']).next().unwrap_or(&lower);
    let ext = path.rsplit('.').next().unwrap_or("");
    const STILL: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "avif", "bmp"];
    const PLAYABLE: &[&str] = &[
        "mp4", "m4v", "webm", "mkv", "mov", "m3u8", "ts", "m4s", "mp3", "m4a", "aac", "wav", "ogg",
        "opus", "flac",
    ];
    if STILL.contains(&ext) {
        return MediaUrlClass::Still;
    }
    if PLAYABLE.contains(&ext) {
        return MediaUrlClass::Playable;
    }
    if lower.starts_with("https://") || lower.starts_with("http://") {
        return MediaUrlClass::Playable;
    }
    MediaUrlClass::Reject
}

fn is_status_page(lower: &str) -> bool {
    let host = lower.contains("://x.com/")
        || lower.contains("://twitter.com/")
        || lower.contains("://mobile.twitter.com/")
        || lower.contains("://mobile.x.com/");
    host && lower.contains("/status/")
}

/// Highest-ranked playable variant wins. Video mp4, then HLS, then audio.
/// A thumbnail JPEG is not a variant and is not chosen here.
pub fn variant_rank(content_type: &str) -> u8 {
    let t = content_type.trim().to_ascii_lowercase();
    if t == "video/mp4" {
        3
    } else if t == "application/x-mpegurl" || t == "application/vnd.apple.mpegurl" {
        2
    } else if t.starts_with("audio/") {
        1
    } else {
        0
    }
}

pub fn choose_variant_url<'a>(variants: &[(&'a str, &'a str, u64)]) -> Option<&'a str> {
    let mut best: Option<(u8, u64, &str)> = None;
    for (content_type, url, bitrate) in variants {
        let rank = variant_rank(content_type);
        if rank == 0 || url.is_empty() {
            continue;
        }
        let replace = match best {
            None => true,
            Some((best_rank, best_bitrate, _)) => {
                rank > best_rank || (rank == best_rank && *bitrate > best_bitrate)
            }
        };
        if replace {
            best = Some((rank, *bitrate, *url));
        }
    }
    best.map(|(_, _, url)| url)
}

pub fn vlc_playback_args(url: &str) -> Result<Vec<String>, &'static str> {
    if media_url_class(url) != MediaUrlClass::Playable {
        return Err(NO_PLAYABLE_MEDIA);
    }
    Ok(vec![
        "--no-one-instance".to_string(),
        "--no-playlist-enqueue".to_string(),
        "--play-and-exit".to_string(),
        url.trim().to_string(),
    ])
}

pub fn vlc_candidates(
    program_files: Option<&Path>,
    program_files_x86: Option<&Path>,
    exe_dir: Option<&Path>,
    path_env: &str,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut push_unique = |path: PathBuf| {
        if !out.iter().any(|existing| existing == &path) {
            out.push(path);
        }
    };
    push_unique(PathBuf::from(VLC_CANONICAL_WINDOWS));
    if let Some(dir) = program_files {
        push_unique(dir.join("VideoLAN").join("VLC").join("vlc.exe"));
    }
    if let Some(dir) = program_files_x86 {
        push_unique(dir.join("VideoLAN").join("VLC").join("vlc.exe"));
    }
    if let Some(dir) = exe_dir {
        push_unique(dir.join("vlc.exe"));
        push_unique(dir.join("vlc").join("vlc.exe"));
        push_unique(dir.join("VLC").join("vlc.exe"));
    }
    for dir in std::env::split_paths(path_env) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        push_unique(dir.join("vlc.exe"));
    }
    out
}

pub fn select_vlc<'a>(candidates: &'a [PathBuf], exists: impl Fn(&Path) -> bool) -> Option<&'a Path> {
    candidates.iter().find(|path| exists(path)).map(PathBuf::as_path)
}

pub fn find_vlc() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf));
    let program_files = std::env::var_os("ProgramFiles").map(PathBuf::from);
    let program_files_x86 = std::env::var_os("ProgramFiles(x86)").map(PathBuf::from);
    let path_env = std::env::var("PATH").unwrap_or_default();
    let candidates = vlc_candidates(
        program_files.as_deref(),
        program_files_x86.as_deref(),
        exe_dir.as_deref(),
        &path_env,
    );
    select_vlc(&candidates, Path::exists).map(Path::to_path_buf)
}

#[cfg(any(windows, target_os = "linux"))]
fn spawn_vlc(bin: &Path, url: &str) -> Result<Child, String> {
    let args = vlc_playback_args(url).map_err(str::to_string)?;
    Command::new(bin)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "VLC was found but did not start. Nothing else was opened.".to_string())
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
    /// Windows: libVLC drawing into a panel-sized host. Not `vlc.exe`.
    #[cfg(windows)]
    inner: vlc_embed::Player,
    /// Linux keeps an external mpv process. It is not the Windows path.
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
            let inner = vlc_embed::Player::start(parent_hwnd, scale, rect, url, volume)?;
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
        #[cfg(windows)]
        self.inner.toggle_pause();
        #[cfg(not(windows))]
        let _ = self;
    }
    pub fn seek_relative(&mut self, seconds: i32) {
        #[cfg(windows)]
        self.inner.seek_relative(seconds);
        #[cfg(not(windows))]
        let _ = (self, seconds);
    }
    pub fn seek_absolute(&mut self, seconds: f64) {
        #[cfg(windows)]
        self.inner.seek_absolute(seconds);
        #[cfg(not(windows))]
        let _ = (self, seconds);
    }
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
        #[cfg(windows)]
        self.inner.set_muted(muted);
        #[cfg(not(windows))]
        let _ = (self, muted);
    }

    pub fn set_volume(&mut self, percent: u8) {
        #[cfg(windows)]
        self.inner.set_volume(percent);
        #[cfg(not(windows))]
        let _ = (self, percent);
    }
    #[cfg_attr(not(any(windows, target_os = "linux")), allow(unused_mut))]
    pub fn stop(mut self) {
        #[cfg(windows)]
        self.inner.stop();
        #[cfg(target_os = "linux")]
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
    pub fn take_failure(&mut self) -> Option<&'static str> {
        #[cfg(windows)]
        {
            self.inner.take_failure()
        }
        #[cfg(not(windows))]
        {
            None
        }
    }
    /// False once playback has ended or the engine reported an error.
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
mod vlc_embed {
    use super::{
        client_pixels, engine_files_plan, find_vlc, host_child_style, libvlc_instance_args,
        media_url_class, pe_machine, EnginePlan, MediaUrlClass, Rect, DLL_LOAD_FLAGS,
        IMAGE_FILE_MACHINE_AMD64, LIBVLC_EVENT_END, LIBVLC_EVENT_ERROR,
    };
    use std::ffi::CString;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Instant;

    type Hwnd = *mut core::ffi::c_void;
    type Hmodule = *mut core::ffi::c_void;

    const SW_HIDE: i32 = 0;
    const SW_SHOWNA: i32 = 8;
    const SWP_NOACTIVATE: u32 = 0x0010;
    const SWP_NOZORDER: u32 = 0x0004;
    const WM_SYSCOMMAND: u32 = 0x0112;
    const SC_MAXIMIZE: usize = 0xF030;
    const WS_EX_NOPARENTNOTIFY: u32 = 0x0000_0004;
    const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
    const GWL_STYLE: i32 = -16;

    #[repr(C)]
    struct WndClass {
        cb_size: u32,
        style: u32,
        lpfn: Option<unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize>,
        cb_cls_extra: i32,
        cb_wnd_extra: i32,
        h_instance: Hwnd,
        h_icon: Hwnd,
        h_cursor: Hwnd,
        hbr_background: Hwnd,
        lpsz_menu_name: *const u16,
        lpsz_class_name: *const u16,
        h_icon_sm: Hwnd,
    }
    #[repr(C)]
    struct LibvlcEvent {
        kind: i32,
        object: *mut core::ffi::c_void,
    }

    unsafe extern "system" {
        fn RegisterClassExW(c: *const WndClass) -> u16;
        fn CreateWindowExW(
            ex: u32, class: *const u16, name: *const u16, style: u32, x: i32, y: i32, w: i32,
            h: i32, parent: Hwnd, menu: Hwnd, inst: Hwnd, param: *mut core::ffi::c_void,
        ) -> Hwnd;
        fn DestroyWindow(h: Hwnd) -> i32;
        fn ShowWindow(h: Hwnd, cmd: i32) -> i32;
        fn SetWindowPos(h: Hwnd, after: Hwnd, x: i32, y: i32, w: i32, h: i32, flags: u32) -> i32;
        fn DefWindowProcW(h: Hwnd, m: u32, w: usize, l: isize) -> isize;
        fn GetModuleHandleW(name: *const u16) -> Hwnd;
        fn LoadLibraryExW(name: *const u16, file: Hwnd, flags: u32) -> Hmodule;
        fn GetProcAddress(module: Hmodule, name: *const i8) -> *mut core::ffi::c_void;
        fn SetPropW(hwnd: Hwnd, name: *const u16, data: *mut core::ffi::c_void) -> i32;
        fn GetWindowLongW(hwnd: Hwnd, index: i32) -> i32;
        fn SetWindowLongW(hwnd: Hwnd, index: i32, value: i32) -> i32;
    }

    type Cb = unsafe extern "system" fn(*const LibvlcEvent, *mut core::ffi::c_void);
    struct Api {
        new: unsafe extern "system" fn(i32, *const *const i8) -> *mut core::ffi::c_void,
        release: unsafe extern "system" fn(*mut core::ffi::c_void),
        media_new: unsafe extern "system" fn(*mut core::ffi::c_void, *const i8) -> *mut core::ffi::c_void,
        media_release: unsafe extern "system" fn(*mut core::ffi::c_void),
        player_new: unsafe extern "system" fn(*mut core::ffi::c_void) -> *mut core::ffi::c_void,
        player_release: unsafe extern "system" fn(*mut core::ffi::c_void),
        play: unsafe extern "system" fn(*mut core::ffi::c_void) -> i32,
        set_pause: unsafe extern "system" fn(*mut core::ffi::c_void, i32),
        stop: unsafe extern "system" fn(*mut core::ffi::c_void),
        set_hwnd: unsafe extern "system" fn(*mut core::ffi::c_void, Hwnd),
        set_volume: unsafe extern "system" fn(*mut core::ffi::c_void, i32) -> i32,
        set_mute: unsafe extern "system" fn(*mut core::ffi::c_void, i32),
        get_time: unsafe extern "system" fn(*mut core::ffi::c_void) -> i64,
        get_length: unsafe extern "system" fn(*mut core::ffi::c_void) -> i64,
        set_time: unsafe extern "system" fn(*mut core::ffi::c_void, i64) -> i32,
        get_position: unsafe extern "system" fn(*mut core::ffi::c_void) -> f32,
        is_seekable: unsafe extern "system" fn(*mut core::ffi::c_void) -> i32,
        set_mouse: unsafe extern "system" fn(*mut core::ffi::c_void, u32),
        set_key: unsafe extern "system" fn(*mut core::ffi::c_void, u32),
        event_manager: unsafe extern "system" fn(*mut core::ffi::c_void) -> *mut core::ffi::c_void,
        event_attach: unsafe extern "system" fn(*mut core::ffi::c_void, i32, Cb, *mut core::ffi::c_void) -> i32,
        event_detach: unsafe extern "system" fn(*mut core::ffi::c_void, i32, Cb, *mut core::ffi::c_void),
        get_state: unsafe extern "system" fn(*mut core::ffi::c_void) -> i32,
    }

    struct Slot {
        generation: u64,
        closed: AtomicBool,
        ended: AtomicBool,
        failed: AtomicBool,
        released: AtomicBool,
        time_ms: AtomicU64,
        length_ms: AtomicU64,
        seekable: AtomicBool,
        audio_stop_ms: AtomicU64,
    }

    struct Handles {
        instance: usize,
        media: usize,
        player: usize,
        generation: u64,
        slot: Arc<Slot>,
    }

    enum Job {
        Pause { generation: u64, paused: bool },
        Seek { generation: u64, ms: i64 },
        Volume { generation: u64, percent: i32 },
        Mute { generation: u64, muted: bool },
        Query { generation: u64 },
        Stop { generation: u64, slot: Arc<Slot> },
    }

    struct Reap {
        host: usize,
        slot: Arc<Slot>,
    }

    fn events() -> &'static Mutex<Vec<(u64, u32)>> {
        static EVENTS: Mutex<Vec<(u64, u32)>> = Mutex::new(Vec::new());
        &EVENTS
    }
    fn current() -> &'static Mutex<Option<Handles>> {
        static CURRENT: Mutex<Option<Handles>> = Mutex::new(None);
        &CURRENT
    }
    fn reaps() -> &'static Mutex<Vec<Reap>> {
        static REAPS: Mutex<Vec<Reap>> = Mutex::new(Vec::new());
        &REAPS
    }
    fn job_queue() -> &'static (Mutex<Vec<Job>>, Condvar) {
        static Q: std::sync::OnceLock<(Mutex<Vec<Job>>, Condvar)> = std::sync::OnceLock::new();
        Q.get_or_init(|| (Mutex::new(Vec::new()), Condvar::new()))
    }

    unsafe extern "system" fn on_event(event: *const LibvlcEvent, data: *mut core::ffi::c_void) {
        // A panic here would unwind through libVLC and abort the process.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if event.is_null() || data.is_null() {
                return;
            }
            let slot = unsafe { &*(data as *const Slot) };
            if slot.closed.load(Ordering::Acquire) {
                return;
            }
            let kind = unsafe { (*event).kind } as u32;
            if let Ok(mut queue) = events().lock() {
                queue.push((slot.generation, kind));
            }
        }));
    }

    unsafe extern "system" fn wndproc(h: Hwnd, m: u32, w: usize, l: isize) -> isize {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if m == WM_SYSCOMMAND && (w & 0xFFF0) == SC_MAXIMIZE {
                return 0;
            }
            unsafe { DefWindowProcW(h, m, w, l) }
        }))
        .unwrap_or(0)
    }

    fn wide(text: &std::ffi::OsStr) -> Vec<u16> {
        text.encode_wide().chain(Some(0)).collect()
    }

    fn proc_of<T>(module: Hmodule, name: &str) -> Result<T, String> {
        let c = CString::new(name).map_err(|_| "bad libvlc name".to_string())?;
        let ptr = unsafe { GetProcAddress(module, c.as_ptr()) };
        if ptr.is_null() {
            return Err("VLC's engine is missing a required entry. No window was opened.".into());
        }
        Ok(unsafe { std::mem::transmute_copy(&ptr) })
    }

    fn read_machine(path: &Path) -> Option<u16> {
        let mut file = std::fs::File::open(path).ok()?;
        let mut prefix = [0u8; 4096];
        use std::io::Read;
        let n = file.read(&mut prefix).ok()?;
        pe_machine(&prefix[..n])
    }

    fn scan_plugins(dir: &Path) -> (usize, bool) {
        let mut count = 0usize;
        let mut all_64 = true;
        let mut stack = vec![dir.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(read) = std::fs::read_dir(&dir) else { continue };
            for entry in read.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
                if !name.ends_with("_plugin.dll") {
                    continue;
                }
                match read_machine(&path) {
                    Some(IMAGE_FILE_MACHINE_AMD64) => count += 1,
                    Some(_) => {
                        count += 1;
                        all_64 = false;
                    }
                    None => all_64 = false,
                }
            }
        }
        (count, all_64 && count > 0)
    }

    fn validate_tree(dir: &Path) -> Result<(), String> {
        let libvlc = read_machine(&dir.join("libvlc.dll"));
        let core = read_machine(&dir.join("libvlccore.dll"));
        let (plugins, all_64) = scan_plugins(&dir.join("plugins"));
        match engine_files_plan(libvlc, core, plugins, all_64) {
            EnginePlan::Embed => Ok(()),
            EnginePlan::Refuse(text) => Err(text.to_string()),
        }
    }

    fn load_api(dir: &Path) -> Result<&'static Api, String> {
        static API: std::sync::OnceLock<Api> = std::sync::OnceLock::new();
        if let Some(api) = API.get() {
            return Ok(api);
        }
        validate_tree(dir)?;
        let core = dir.join("libvlccore.dll");
        let core_w = wide(core.as_os_str());
        let core_module = unsafe { LoadLibraryExW(core_w.as_ptr(), std::ptr::null_mut(), DLL_LOAD_FLAGS) };
        if core_module.is_null() {
            return Err(super::VLC_ENGINE_MISSING.into());
        }
        let _ = core_module;
        let dll = dir.join("libvlc.dll");
        let dll_w = wide(dll.as_os_str());
        let module = unsafe { LoadLibraryExW(dll_w.as_ptr(), std::ptr::null_mut(), DLL_LOAD_FLAGS) };
        if module.is_null() {
            return Err(super::VLC_ENGINE_MISSING.into());
        }
        let api = Api {
            new: proc_of(module, "libvlc_new")?,
            release: proc_of(module, "libvlc_release")?,
            media_new: proc_of(module, "libvlc_media_new_location")?,
            media_release: proc_of(module, "libvlc_media_release")?,
            player_new: proc_of(module, "libvlc_media_player_new_from_media")?,
            player_release: proc_of(module, "libvlc_media_player_release")?,
            play: proc_of(module, "libvlc_media_player_play")?,
            set_pause: proc_of(module, "libvlc_media_player_set_pause")?,
            stop: proc_of(module, "libvlc_media_player_stop")?,
            set_hwnd: proc_of(module, "libvlc_media_player_set_hwnd")?,
            set_volume: proc_of(module, "libvlc_audio_set_volume")?,
            set_mute: proc_of(module, "libvlc_audio_set_mute")?,
            get_time: proc_of(module, "libvlc_media_player_get_time")?,
            get_length: proc_of(module, "libvlc_media_player_get_length")?,
            set_time: proc_of(module, "libvlc_media_player_set_time")?,
            get_position: proc_of(module, "libvlc_media_player_get_position")?,
            is_seekable: proc_of(module, "libvlc_media_player_is_seekable")?,
            set_mouse: proc_of(module, "libvlc_video_set_mouse_input")?,
            set_key: proc_of(module, "libvlc_video_set_key_input")?,
            event_manager: proc_of(module, "libvlc_media_player_event_manager")?,
            event_attach: proc_of(module, "libvlc_event_attach")?,
            event_detach: proc_of(module, "libvlc_event_detach")?,
            get_state: proc_of(module, "libvlc_media_player_get_state")?,
        };
        let _ = module;
        Ok(API.get_or_init(|| api))
    }

    fn enqueue(job: Job) {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let _ = std::thread::Builder::new().name("termy-vlc".into()).spawn(worker_loop);
        });
        let (lock, cv) = job_queue();
        if let Ok(mut queue) = lock.lock() {
            queue.push(job);
            cv.notify_one();
        }
    }

    fn worker_loop() {
        loop {
            let job = {
                let (lock, cv) = job_queue();
                let mut queue = lock.lock().unwrap_or_else(|e| e.into_inner());
                while queue.is_empty() {
                    queue = cv.wait(queue).unwrap_or_else(|e| e.into_inner());
                }
                queue.remove(0)
            };
            match job {
                Job::Pause { generation, paused } => worker_pause(generation, paused),
                Job::Seek { generation, ms } => worker_seek(generation, ms),
                Job::Volume { generation, percent } => worker_volume(generation, percent),
                Job::Mute { generation, muted } => worker_mute(generation, muted),
                Job::Query { generation } => worker_query(generation),
                Job::Stop { generation, slot } => worker_stop(generation, slot),
            }
        }
    }

    fn with_player<T>(generation: u64, f: impl FnOnce(&Api, *mut core::ffi::c_void) -> T) -> Option<T> {
        let cur = current().lock().ok()?;
        let cur = cur.as_ref()?;
        if cur.generation != generation {
            return None;
        }
        let api = load_api(Path::new(".")).ok()?;
        Some(f(api, cur.player as *mut core::ffi::c_void))
    }

    fn ensure_clip_children(host: Hwnd) -> Result<(), String> {
        let current = unsafe { GetWindowLongW(host, GWL_STYLE) } as u32;
        let next = current | super::HOST_STYLE_CLIP_CHILDREN;
        unsafe { SetWindowLongW(host, GWL_STYLE, next as i32); }
        let after = unsafe { GetWindowLongW(host, GWL_STYLE) } as u32;
        if super::start_would_abort_in_ucrt(false, after & super::HOST_STYLE_CLIP_CHILDREN != 0) {
            return Err("video surface is missing WS_CLIPCHILDREN, so VLC would abort. No window was opened.".into());
        }
        super::surface_style_for_libvlc(after).map_err(str::to_string)
    }

    /// Called on the window thread. Does not pump messages: the caller is
    /// inside a GPUI update, and `DispatchMessage` from here abort()s.
    fn open_embedded(url: CString, volume: i32, hwnd: usize, generation: u64, slot: Arc<Slot>) -> Result<(), String> {
        if !super::START_ON_WINDOW_THREAD || super::start_would_abort_in_ucrt(super::DISPATCH_DURING_START, true) {
            return Err("video start would abort before playback. No window was opened.".into());
        }
        ensure_clip_children(hwnd as Hwnd)?;
        let exe = find_vlc().ok_or_else(|| super::VLC_NOT_FOUND.to_string())?;
        let Some(dir) = exe.parent() else {
            return Err(super::VLC_ENGINE_MISSING.into());
        };
        let api = load_api(dir)?;
        let old = current().lock().ok().and_then(|mut g| g.take());
        if let Some(old) = old {
            release_handles(api, old);
        }
        let plugins = dir.join("plugins");
        let args = libvlc_instance_args(&plugins.to_string_lossy());
        let c_args: Vec<CString> = args.iter().map(|arg| CString::new(arg.as_str()).unwrap_or_else(|_| CString::new("").unwrap())).collect();
        let mut ptrs: Vec<*const i8> = c_args.iter().map(|arg| arg.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        let argc = super::libvlc_argc(c_args.len(), ptrs.last().is_some_and(|ptr| ptr.is_null()))
            .map_err(str::to_string)?;
        let instance = unsafe { (api.new)(argc, ptrs.as_ptr()) };
        if instance.is_null() {
            return Err("VLC's engine did not start. No window was opened.".into());
        }
        let media = unsafe { (api.media_new)(instance, url.as_ptr()) };
        if media.is_null() {
            unsafe { (api.release)(instance) };
            return Err("VLC's engine could not open this media. No window was opened.".into());
        }
        let player = unsafe { (api.player_new)(media) };
        if player.is_null() {
            unsafe {
                (api.media_release)(media);
                (api.release)(instance);
            }
            return Err("VLC's engine could not create playback. No window was opened.".into());
        }
        unsafe {
            (api.set_hwnd)(player, hwnd as Hwnd);
            (api.set_mouse)(player, 0);
            (api.set_key)(player, 0);
            let _ = (api.set_volume)(player, volume);
            (api.set_mute)(player, if volume == 0 { 1 } else { 0 });
            let manager = (api.event_manager)(player);
            if !manager.is_null() {
                let user = Arc::as_ptr(&slot) as *mut core::ffi::c_void;
                let _ = (api.event_attach)(manager, LIBVLC_EVENT_END as i32, on_event, user);
                let _ = (api.event_attach)(manager, LIBVLC_EVENT_ERROR as i32, on_event, user);
            }
            if (api.play)(player) != 0 {
                (api.set_hwnd)(player, std::ptr::null_mut());
                (api.stop)(player);
                (api.player_release)(player);
                (api.media_release)(media);
                (api.release)(instance);
                return Err("VLC's engine could not play this file. No window was opened.".into());
            }
        }
        let handles = Handles {
            instance: instance as usize,
            media: media as usize,
            player: player as usize,
            generation,
            slot,
        };
        match current().lock() {
            Ok(mut guard) => {
                *guard = Some(handles);
                Ok(())
            }
            Err(_) => {
                release_handles(api, handles);
                Err("VLC's engine could not keep this playback. No window was opened.".into())
            }
        }
    }

    fn worker_pause(generation: u64, paused: bool) {
        let _ = with_player(generation, |api, player| unsafe {
            (api.set_pause)(player, if paused { 1 } else { 0 });
        });
    }

    fn worker_seek(generation: u64, ms: i64) {
        let _ = with_player(generation, |api, player| unsafe {
            if (api.is_seekable)(player) != 0 {
                let _ = (api.set_time)(player, ms);
            }
        });
    }

    fn worker_volume(generation: u64, percent: i32) {
        let _ = with_player(generation, |api, player| unsafe {
            let _ = (api.set_volume)(player, percent);
            (api.set_mute)(player, if percent == 0 { 1 } else { 0 });
        });
    }

    fn worker_mute(generation: u64, muted: bool) {
        let _ = with_player(generation, |api, player| unsafe {
            (api.set_mute)(player, if muted { 1 } else { 0 });
        });
    }

    fn worker_query(generation: u64) {
        let looked = current().lock().ok().and_then(|guard| {
            let cur = guard.as_ref()?;
            if cur.generation != generation {
                None
            } else {
                Some((cur.player, Arc::clone(&cur.slot)))
            }
        });
        let Some((player, slot)) = looked else {
            return;
        };
        let Ok(api) = load_api(Path::new(".")) else {
            return;
        };
        let player = player as *mut core::ffi::c_void;
        unsafe {
            let time = (api.get_time)(player).max(0) as u64;
            let length = (api.get_length)(player).max(0) as u64;
            let _position = (api.get_position)(player);
            slot.time_ms.store(time, Ordering::Release);
            slot.length_ms.store(length, Ordering::Release);
            slot.seekable
                .store((api.is_seekable)(player) != 0, Ordering::Release);
            let state = (api.get_state)(player);
            if state == 6 {
                slot.ended.store(true, Ordering::Release);
            } else if state == 7 {
                slot.failed.store(true, Ordering::Release);
            }
        }
    }

    fn release_handles(api: &Api, handles: Handles) {
        handles.slot.closed.store(true, Ordering::Release);
        let player = handles.player as *mut core::ffi::c_void;
        let media = handles.media as *mut core::ffi::c_void;
        let instance = handles.instance as *mut core::ffi::c_void;
        let user = Arc::as_ptr(&handles.slot) as *mut core::ffi::c_void;
        unsafe {
            let manager = (api.event_manager)(player);
            if !manager.is_null() {
                (api.event_detach)(manager, LIBVLC_EVENT_END as i32, on_event, user);
                (api.event_detach)(manager, LIBVLC_EVENT_ERROR as i32, on_event, user);
            }
            let started = Instant::now();
            (api.set_pause)(player, 1);
            let ms = started.elapsed().as_millis() as u64;
            handles.slot.audio_stop_ms.store(ms.max(1), Ordering::Release);
            (api.set_hwnd)(player, std::ptr::null_mut());
            (api.stop)(player);
            (api.player_release)(player);
            (api.media_release)(media);
            (api.release)(instance);
        }
        if let Ok(mut queue) = events().lock() {
            queue.retain(|(generation, _)| *generation != handles.generation);
        }
        handles.slot.released.store(true, Ordering::Release);
        let _ = note_audio_stop(handles.slot.audio_stop_ms.load(Ordering::Acquire));
    }

    fn note_audio_stop(ms: u64) -> std::io::Result<()> {
        let line = format!("termy-video event=audio-stop ms={ms}");
        let path = std::env::temp_dir().join("termy-video-hwnd.log");
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
        use std::io::Write;
        writeln!(file, "{line}")
    }

    fn worker_stop(generation: u64, slot: Arc<Slot>) {
        let exe = find_vlc();
        let api = exe.as_ref().and_then(|exe| exe.parent()).and_then(|dir| load_api(dir).ok());
        let handles = current().lock().ok().and_then(|mut guard| {
            if guard.as_ref().is_some_and(|cur| cur.generation == generation) {
                guard.take()
            } else {
                None
            }
        });
        if let (Some(api), Some(handles)) = (api, handles) {
            release_handles(api, handles);
        } else {
            slot.released.store(true, Ordering::Release);
        }
    }

    pub struct Player {
        host: Hwnd,
        parent: Hwnd,
        scale: f32,
        rect: Rect,
        generation: u64,
        slot: Arc<Slot>,
        paused: bool,
        hidden: bool,
    }
    unsafe impl Send for Player {}

    impl Player {
        pub fn start(parent: isize, scale: f32, rect: Rect, url: &str, volume: u8) -> Result<Self, String> {
            reap_hosts();
            if parent == 0 {
                return Err("Termy window is not ready. No window was opened.".into());
            }
            if media_url_class(url) != MediaUrlClass::Playable {
                return Err(super::NO_PLAYABLE_MEDIA.into());
            }
            let location = CString::new(url.trim()).map_err(|_| super::NO_PLAYABLE_MEDIA.to_string())?;
            let (x, y, w, h) = client_pixels(scale, rect);
            let class = wide(std::ffi::OsStr::new("TermyXVlcHost"));
            let host = unsafe {
                let wc = WndClass {
                    cb_size: std::mem::size_of::<WndClass>() as u32,
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
                let _ = RegisterClassExW(&wc);
                CreateWindowExW(
                    WS_EX_NOPARENTNOTIFY | WS_EX_NOACTIVATE,
                    class.as_ptr(),
                    wide(std::ffi::OsStr::new("")).as_ptr(),
                    host_child_style(),
                    x, y, w, h,
                    parent as Hwnd,
                    std::ptr::null_mut(),
                    GetModuleHandleW(std::ptr::null()),
                    std::ptr::null_mut(),
                )
            };
            if host.is_null() {
                return Err("Could not place the video inside Termy. No window was opened.".into());
            }
            let rude = wide(std::ffi::OsStr::new("NonRudeHWND"));
            unsafe { SetPropW(host, rude.as_ptr(), 1 as *mut core::ffi::c_void); }
            static GEN: AtomicU64 = AtomicU64::new(1);
            let generation = GEN.fetch_add(1, Ordering::Relaxed);
            let slot = Arc::new(Slot {
                generation,
                closed: AtomicBool::new(false),
                ended: AtomicBool::new(false),
                failed: AtomicBool::new(false),
                released: AtomicBool::new(false),
                time_ms: AtomicU64::new(0),
                length_ms: AtomicU64::new(0),
                seekable: AtomicBool::new(false),
                audio_stop_ms: AtomicU64::new(0),
            });
            // No PeekMessage/DispatchMessage here. This runs inside a GPUI
            // update; pumping the thread queue re-enters that borrow and
            // abort()s (0xc0000409 in ucrtbase). libVLC runs on this thread,
            // then GPUI's own loop runs after the update returns.
            if let Err(err) = open_embedded(
                location,
                i32::from(volume.min(100)),
                host as usize,
                generation,
                Arc::clone(&slot),
            ) {
                slot.closed.store(true, Ordering::Release);
                slot.released.store(true, Ordering::Release);
                unsafe { DestroyWindow(host) };
                return Err(err);
            }
            unsafe { ShowWindow(host, SW_SHOWNA) };
            Ok(Self { host, parent: parent as Hwnd, scale, rect, generation, slot, paused: false, hidden: false })
        }

        pub fn reposition(&mut self, scale: f32, rect: Rect) {
            self.scale = scale;
            self.rect = rect;
            if self.host.is_null() || self.hidden {
                return;
            }
            let (x, y, w, h) = client_pixels(scale, rect);
            unsafe {
                SetWindowPos(self.host, std::ptr::null_mut(), x, y, w, h, SWP_NOACTIVATE | SWP_NOZORDER);
            }
        }

        pub fn toggle_pause(&mut self) {
            self.paused = !self.paused;
            enqueue(Job::Pause { generation: self.generation, paused: self.paused });
        }

        pub fn seek_relative(&mut self, seconds: i32) {
            let (pos, _) = self.progress();
            self.seek_absolute((pos + f64::from(seconds)).max(0.0));
        }

        pub fn seek_absolute(&mut self, seconds: f64) {
            if !self.slot.seekable.load(Ordering::Acquire) && self.slot.time_ms.load(Ordering::Acquire) == 0 {
                enqueue(Job::Query { generation: self.generation });
            }
            let ms = (seconds.max(0.0) * 1000.0) as i64;
            enqueue(Job::Seek { generation: self.generation, ms });
        }

        pub fn progress(&self) -> (f64, f64) {
            enqueue(Job::Query { generation: self.generation });
            let time = self.slot.time_ms.load(Ordering::Acquire);
            let length = self.slot.length_ms.load(Ordering::Acquire);
            (time as f64 / 1000.0, length as f64 / 1000.0)
        }

        pub fn set_volume(&mut self, percent: u8) {
            enqueue(Job::Volume { generation: self.generation, percent: i32::from(percent.min(100)) });
        }

        pub fn set_muted(&mut self, muted: bool) {
            enqueue(Job::Mute { generation: self.generation, muted });
        }

        pub fn take_failure(&mut self) -> Option<&'static str> {
            self.drain_events();
            if self.slot.failed.load(Ordering::Acquire) {
                Some("VLC's engine could not play this file. No window was opened.")
            } else {
                None
            }
        }

        fn drain_events(&self) {
            let queued = events().lock().ok().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default();
            let mut flags = super::EngineFlags::default();
            for (generation, kind) in queued {
                super::apply_engine_callback(self.generation, generation, kind, &mut flags);
            }
            if flags.ended {
                self.slot.ended.store(true, Ordering::Release);
            }
            if flags.failed {
                self.slot.failed.store(true, Ordering::Release);
            }
        }

        pub fn is_alive(&mut self) -> bool {
            self.drain_events();
            !self.hidden && !self.slot.ended.load(Ordering::Acquire) && !self.slot.failed.load(Ordering::Acquire)
        }

        pub fn stop(&mut self) {
            if self.hidden {
                return;
            }
            self.hidden = true;
            self.slot.closed.store(true, Ordering::Release);
            if !self.host.is_null() {
                unsafe { ShowWindow(self.host, SW_HIDE) };
                if let Ok(mut guard) = reaps().lock() {
                    guard.push(Reap { host: self.host as usize, slot: Arc::clone(&self.slot) });
                }
                self.host = std::ptr::null_mut();
            }
            enqueue(Job::Stop { generation: self.generation, slot: Arc::clone(&self.slot) });
        }
    }

    pub fn reap_hosts() {
        let ready = {
            let Ok(mut guard) = reaps().lock() else { return };
            let mut left = Vec::new();
            let mut ready = Vec::new();
            for reap in guard.drain(..) {
                if reap.slot.released.load(Ordering::Acquire) {
                    ready.push(reap.host);
                } else {
                    left.push(reap);
                }
            }
            *guard = left;
            ready
        };
        for host in ready {
            if host != 0 {
                unsafe { DestroyWindow(host as Hwnd) };
            }
        }
    }

    impl Drop for Player {
        fn drop(&mut self) {
            self.stop();
        }
    }
}

// The embedded mpv host is not the Play path. It stays out of the build
// so a Windows compile does not depend on that surface.
#[cfg(any())]
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
        CLOSE_BAND, HOST_STYLE_CLIP_CHILDREN, HOST_STYLE_VISIBLE, HostGate, HostPhase,
        DLL_LOAD_FLAGS, EngineFlags, EnginePhase, EnginePlan, EngineStep, HOST_STYLE_POPUP,
        MediaDispatch, NO_PLAYABLE_MEDIA, VLC_PLUGINS_MISSING, WS_CHILD,
        PlayGen, SWP_NOMOVE, SWP_NOSIZE, TRACE_LINE_CAP, VLC_ENGINE_MISSING, VLC_WRONG_ARCH,
        TRACE_OTHER_CAP, VLC_CANONICAL_WINDOWS, VLC_NOT_FOUND, admit_trace_line, begin_playback,
        callback_is_current, choose_variant_url, clamp_window_change, close_button_rect,
        dismiss_playback, dispatch_media, embed_rect, event_name, format_obs_line,
        host_create_style, host_style_starts_hidden, keep_main_window, live_size_differs_half,
        marker_name, playback_args, pos_is_noop, rect_changed, rects_overlap, sanitize_token,
        select_vlc, swp_names, vlc_candidates, vlc_playback_args, volume_commands, volume_percent,
        apply_engine_callback, client_pixels, engine_files_plan, engine_plan, engine_step,
        host_child_style, launches_external_vlc, libvlc_argc, libvlc_instance_args, pe_machine,
        start_would_abort_in_ucrt, surface_style_for_libvlc, ENGINE_RELEASE_ORDER,
        DISPATCH_DURING_START, IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_I386, LIBVLC_EVENT_END,
        LIBVLC_EVENT_ERROR, LIBVLC_START_ORDER, START_ON_WINDOW_THREAD,
    };
    use std::path::{Path, PathBuf};
    use termy_x::model::MediaKind;

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

    #[test]
    fn photos_stay_in_termy_and_only_playable_video_goes_to_vlc() {
        assert_eq!(
            dispatch_media(MediaKind::Photo, "https://pbs.twimg.com/media/ABC.jpg"),
            MediaDispatch::StillImage
        );
        assert_eq!(
            dispatch_media(MediaKind::Photo, "https://video.twimg.com/a.mp4"),
            MediaDispatch::StillImage
        );
        assert_eq!(
            dispatch_media(
                MediaKind::Video,
                "https://video.twimg.com/amplify_video/9/vid/avc1/320x568/a.mp4?tag=29"
            ),
            MediaDispatch::Vlc
        );
        assert_eq!(
            dispatch_media(MediaKind::Gif, "https://video.twimg.com/tweet_video/a.mp4"),
            MediaDispatch::Vlc
        );
        assert_eq!(
            dispatch_media(
                MediaKind::Video,
                "https://pbs.twimg.com/amplify_video_thumb/9/img/p.jpg"
            ),
            MediaDispatch::StillImage
        );
        assert_eq!(
            dispatch_media(MediaKind::Video, "https://video.twimg.com/audio/a.m4a"),
            MediaDispatch::Vlc
        );
        assert_eq!(
            dispatch_media(MediaKind::Video, "https://x.com/someone/status/1"),
            MediaDispatch::Unplayable
        );
        assert_eq!(dispatch_media(MediaKind::Video, ""), MediaDispatch::Unplayable);
        assert_eq!(
            dispatch_media(MediaKind::Video, "https://video.twimg.com/a.mp4\ncalc"),
            MediaDispatch::Unplayable
        );
    }

    #[test]
    fn vlc_uses_the_program_files_executable_and_its_own_window() {
        let candidates = vlc_candidates(
            Some(Path::new(r"C:\Program Files")),
            Some(Path::new(r"C:\Program Files (x86)")),
            Some(Path::new(r"D:\Termy")),
            r"C:\Windows\System32",
        );
        assert_eq!(candidates[0].to_string_lossy(), VLC_CANONICAL_WINDOWS);
        let chosen = select_vlc(&candidates, |path| {
            path.to_string_lossy() == VLC_CANONICAL_WINDOWS
        });
        assert_eq!(chosen.unwrap().to_string_lossy(), VLC_CANONICAL_WINDOWS);
        assert!(select_vlc(&[PathBuf::from("vlc"), PathBuf::from("vlc.exe")], |_| false).is_none());
        assert!(!VLC_NOT_FOUND.contains("http"));
        assert!(!NO_PLAYABLE_MEDIA.contains("http"));
        let url = "https://video.twimg.com/a.mp4?tag=1&x=1";
        let args = vlc_playback_args(url).unwrap();
        assert!(args.iter().any(|arg| arg == "--no-one-instance"));
        assert!(args.iter().any(|arg| arg == "--no-playlist-enqueue"));
        assert!(args.iter().any(|arg| arg == "--play-and-exit"));
        assert!(args.iter().all(|arg| {
            arg != "--fullscreen"
                && arg != "-f"
                && arg != "--one-instance"
                && arg != "--playlist-enqueue"
                && !arg.starts_with("--volume")
        }));
        assert_eq!(args.last().map(String::as_str), Some(url));
        assert_eq!(args.iter().filter(|arg| arg.contains("http")).count(), 1);
        assert!(vlc_playback_args("https://pbs.twimg.com/media/a.jpg").is_err());
        assert_eq!(
            choose_variant_url(&[
                ("application/x-mpegURL", "https://video.twimg.com/a.m3u8", 0),
                ("audio/mp4", "https://video.twimg.com/a.m4a", 0),
                ("video/mp4", "https://video.twimg.com/low.mp4", 256_000),
                ("video/mp4", "https://video.twimg.com/high.mp4", 2_176_000),
            ]),
            Some("https://video.twimg.com/high.mp4")
        );
        assert_eq!(
            choose_variant_url(&[("audio/mp4", "https://video.twimg.com/a.m4a", 64_000)]),
            Some("https://video.twimg.com/a.m4a")
        );
        assert_eq!(choose_variant_url(&[]), None);
    }

    #[test]
    fn libvlc_embed_refuses_a_bad_library_and_does_not_spawn_vlc() {
        let mut pe = vec![0u8; 0x80];
        pe[0] = b'M';
        pe[1] = b'Z';
        pe[0x3c] = 0x40;
        pe[0x40] = b'P';
        pe[0x41] = b'E';
        pe[0x44] = (IMAGE_FILE_MACHINE_AMD64 & 0xff) as u8;
        pe[0x45] = (IMAGE_FILE_MACHINE_AMD64 >> 8) as u8;
        assert_eq!(pe_machine(&pe), Some(IMAGE_FILE_MACHINE_AMD64));
        pe[0x44] = (IMAGE_FILE_MACHINE_I386 & 0xff) as u8;
        pe[0x45] = (IMAGE_FILE_MACHINE_I386 >> 8) as u8;
        assert_eq!(pe_machine(&pe), Some(IMAGE_FILE_MACHINE_I386));
        assert_eq!(engine_plan(Some(IMAGE_FILE_MACHINE_AMD64), true), EnginePlan::Embed);
        assert_eq!(
            engine_plan(Some(IMAGE_FILE_MACHINE_I386), true),
            EnginePlan::Refuse(VLC_WRONG_ARCH)
        );
        assert_eq!(engine_plan(None, true), EnginePlan::Refuse(VLC_ENGINE_MISSING));
        assert_eq!(
            engine_plan(Some(IMAGE_FILE_MACHINE_AMD64), false),
            EnginePlan::Refuse(NO_PLAYABLE_MEDIA)
        );
        assert!(!VLC_WRONG_ARCH.contains("http"));
        assert!(!VLC_ENGINE_MISSING.contains("vlc.exe --"));
        let args = libvlc_instance_args(r"C:\Program Files\VideoLAN\VLC\plugins");
        assert!(args.iter().any(|arg| arg == "--intf=dummy"));
        assert!(args.iter().any(|arg| arg == "--dummy-quiet"));
        assert!(args.iter().any(|arg| arg == "--ignore-config"));
        assert!(args.iter().all(|arg| arg != "--fullscreen" && !arg.contains("http")));
        assert_eq!(args.iter().filter(|arg| arg.starts_with("--plugin-path=")).count(), 1);
        assert_eq!(engine_step(EnginePhase::Idle, EngineStep::Play), Err("play before the panel surface exists"));
        assert_eq!(
            engine_step(EnginePhase::Idle, EngineStep::PlaceHost).unwrap(),
            EnginePhase::HostReady
        );
        let playing = engine_step(EnginePhase::HostReady, EngineStep::Play).unwrap();
        assert_eq!(playing, EnginePhase::Playing);
        assert_eq!(
            engine_step(playing, EngineStep::Stop).and_then(|phase| engine_step(phase, EngineStep::Release)).unwrap(),
            EnginePhase::Released
        );
        assert_eq!(
            engine_step(EnginePhase::Released, EngineStep::Stop).unwrap(),
            EnginePhase::Released
        );
        let mut flags = EngineFlags::default();
        apply_engine_callback(2, 1, LIBVLC_EVENT_END, &mut flags);
        assert!(!flags.ended);
        apply_engine_callback(1, 1, LIBVLC_EVENT_END, &mut flags);
        assert!(flags.ended);
        apply_engine_callback(1, 1, LIBVLC_EVENT_ERROR, &mut flags);
        assert!(flags.failed);
        assert_eq!(ENGINE_RELEASE_ORDER[0], "hide-host");
        assert_eq!(ENGINE_RELEASE_ORDER.last().copied(), Some("destroy-host"));
        assert!(!launches_external_vlc());
        let child = host_child_style();
        assert_ne!(child & WS_CHILD, 0);
        assert_ne!(child & HOST_STYLE_CLIP_CHILDREN, 0);
        assert_eq!(child & HOST_STYLE_POPUP, 0);
        assert_eq!(child & HOST_STYLE_VISIBLE, 0);
        assert_eq!(client_pixels(1.25, (0.0, 0.0, 480.0, 616.0)), (0, 0, 600, 770));
        assert_eq!(DLL_LOAD_FLAGS, 0x0000_0100 | 0x0000_0800);
        assert_eq!(
            engine_files_plan(Some(IMAGE_FILE_MACHINE_AMD64), Some(IMAGE_FILE_MACHINE_AMD64), 4, true),
            EnginePlan::Embed
        );
        assert_eq!(
            engine_files_plan(Some(IMAGE_FILE_MACHINE_AMD64), Some(IMAGE_FILE_MACHINE_AMD64), 0, true),
            EnginePlan::Refuse(VLC_PLUGINS_MISSING)
        );
        assert_eq!(
            engine_files_plan(Some(IMAGE_FILE_MACHINE_AMD64), Some(IMAGE_FILE_MACHINE_I386), 4, true),
            EnginePlan::Refuse(VLC_WRONG_ARCH)
        );
        let flags_only = libvlc_instance_args(r"C:\Program Files\VideoLAN\VLC\plugins");
        assert!(flags_only.iter().any(|arg| arg.starts_with("--plugin-path=")));
        assert_ne!(
            engine_files_plan(Some(IMAGE_FILE_MACHINE_AMD64), Some(IMAGE_FILE_MACHINE_AMD64), 0, true),
            EnginePlan::Embed
        );
    }

    #[test]
    fn play_start_does_not_pump_the_ui_thread_or_skip_clip_children() {
        // Installed 0278d94 aborted in ucrtbase (0xc0000409) on play.
        // Start runs inside a GPUI update. DispatchMessage re-enters that
        // borrow; the panic cannot unwind through the Win32 frame and abort()s.
        // VLC 3.0.24 also aborts when the drawable lacks WS_CLIPCHILDREN.
        assert!(START_ON_WINDOW_THREAD);
        assert!(!DISPATCH_DURING_START);
        assert!(!start_would_abort_in_ucrt(DISPATCH_DURING_START, true));
        assert!(start_would_abort_in_ucrt(true, true));
        assert!(start_would_abort_in_ucrt(false, false));
        assert_eq!(
            LIBVLC_START_ORDER,
            [
                "place-child",
                "require-clip-children",
                "libvlc-new-on-window-thread",
                "set-hwnd",
                "play",
                "show-host",
            ]
        );
        let play_at = LIBVLC_START_ORDER.iter().position(|step| *step == "play").unwrap();
        let thread_at = LIBVLC_START_ORDER
            .iter()
            .position(|step| *step == "libvlc-new-on-window-thread")
            .unwrap();
        assert!(thread_at < play_at);
        assert!(surface_style_for_libvlc(host_child_style()).is_ok());
        assert!(surface_style_for_libvlc(host_child_style() & !HOST_STYLE_CLIP_CHILDREN).is_err());
        assert!(surface_style_for_libvlc(host_create_style()).is_err());
        assert_eq!(libvlc_argc(5, true).unwrap(), 5);
        assert!(libvlc_argc(5, false).is_err());
        let args = libvlc_instance_args(r"C:\Program Files\VideoLAN\VLC\plugins");
        assert_eq!(libvlc_argc(args.len(), true).unwrap() as usize, args.len());
        assert!(args.iter().all(|arg| !arg.contains("http")));
    }
}
