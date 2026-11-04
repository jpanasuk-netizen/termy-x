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
    pub fn start(parent_hwnd: isize, scale: f32, rect: Rect, url: &str) -> Result<Self, String> {
        #[cfg(windows)]
        {
            let bin = find_player()
                .ok_or_else(|| "mpv not found (expected mpv\\mpv.exe next to Termy)".to_string())?;
            let inner = win::Player::start(bin, parent_hwnd, scale, rect, url)?;
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
            let child = Command::new(&bin)
                .args(["--really-quiet", "--keep-open=yes", url])
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
        if !muted {
            // Never unmute into volume 0 (silent "unmuted" playback).
            self.command(r#"{"command":["set_property","volume",100]}"#);
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
            let _ = child.wait();
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
    const WS_POPUP: u32 = 0x8000_0000;
    const WS_VISIBLE: u32 = 0x1000_0000;
    const WS_CLIPSIBLINGS: u32 = 0x0400_0000;
    const SWP_NOACTIVATE: u32 = 0x0010;
    const SWP_SHOWWINDOW: u32 = 0x0040;

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
                    0,
                    class.as_ptr(),
                    wide("").as_ptr(),
                    WS_POPUP | WS_VISIBLE | WS_CLIPSIBLINGS,
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
            // Unique pipe per player instance: a process-wide name let a
            // previous (leaked) poller compete with the next video's mpv.
            static PIPE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let pipe = format!(
                r"\\.\pipe\termyx-mpv-{}-{}",
                std::process::id(),
                PIPE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            );
            let mut cmd = Command::new(&bin);
            cmd.args([
                &format!("--wid={}", host as isize),
                &format!("--input-ipc-server={pipe}"),
                "--no-terminal",
                "--really-quiet",
                "--keep-open=yes",
                "--osc=yes",
                "--osd-level=1",
                "--mute=no",
                "--volume=100",
                "--cursor-autohide=no",
                "--hwdec=auto-safe",
                url,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
            let child = cmd.spawn().map_err(|e| {
                unsafe { DestroyWindow(host) };
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
                            if IsIconic(parent_h) != 0 {
                                ShowWindow(host_raw as Hwnd, 0);
                            } else {
                                let want = (origin.x + ox, origin.y + oy, w2, h2);
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
                                        SWP_NOACTIVATE | SWP_SHOWWINDOW,
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
                unsafe {
                    SetWindowPos(
                        self.host,
                        std::ptr::null_mut(),
                        r.0,
                        r.1,
                        r.2,
                        r.3,
                        SWP_NOACTIVATE | SWP_SHOWWINDOW,
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
            // Stop both background pollers first. This flag existed but
            // was never set, so every watched video leaked a progress
            // poller and a window-position poller for the process
            // lifetime; the orphaned progress poller then competed for
            // the (previously shared) IPC pipe and starved the next
            // video's progress updates — position stuck at 0:00/0:00
            // and EOF was never detected, leaving the viewer open on a
            // dead end-frame. Killing mpv below breaks any in-flight
            // pipe read so the poller can observe the flag and exit.
            self.stop_flag
                .store(true, std::sync::atomic::Ordering::Relaxed);
            self.ipc(r#"{"command":["quit"]}"#);
            if let Some(mut c) = self.child.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
            if !self.host.is_null() {
                unsafe { DestroyWindow(self.host) };
                self.host = std::ptr::null_mut();
            }
        }
    }
    impl Drop for Player {
        fn drop(&mut self) {
            self.stop_inner();
        }
    }
}
