//! Place Termy XL against Termy X, or on the right half of the monitor.

use gpui_kit::{App, Window, px, size};

use crate::trends_desk::TrendsDesk;
use termy_x::trends_desk::{DeskFrame, WindowRect, snap_tracked};

#[cfg(test)]
mod tests {
    #[test]
    fn a_minimized_window_is_restored_before_focus() {
        assert_eq!(
            super::focus_steps(true),
            ["restore", "show", "raise", "foreground"]
        );
        assert!(super::focus_steps(false).contains(&"raise"));
        assert!(!super::focus_steps(false).contains(&"restore"));
    }
}

pub(crate) fn snapped_frame(cx: &App) -> termy_x::trends_desk::DeskFrame {
    let monitor = monitor_rect_app(cx);
    let root = termy_x::trends_desk::termy_xl_data_root();
    let docked = termy_x::trends_desk::DeskStore::open_isolated(&root)
        .map(|store| store.load_settings().docked)
        .unwrap_or(true);
    ensure_location_hook(&root);
    let host = seen_host();
    let (snap, line) = termy_x::trends_desk::attach_plan(host.as_ref(), monitor, true, docked);
    termy_x::trends_desk::append_desk_log(&root, &line);
    termy_x::trends_desk::DeskFrame {
        x: snap.x,
        y: snap.y,
        width: snap.width,
        height: snap.height,
        expanded: true,
        expanded_width: termy_x::trends_desk::LOCKED_OPEN_WIDTH,
    }
}

pub(crate) fn desk_window_options(
    frame: &termy_x::trends_desk::DeskFrame,
) -> gpui_kit::WindowOptions {
    use gpui_kit::{Bounds, WindowBounds, WindowKind, point, px, size};
    gpui_kit::WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(frame.x), px(frame.y)),
            size: size(px(frame.width), px(frame.height)),
        })),
        titlebar: Some(gpui_kit::TitlebarOptions {
            title: Some(termy_x::trends_desk::VISIBLE_NAME.into()),
            appears_transparent: false,
            traffic_light_position: None,
        }),
        #[cfg(target_os = "linux")]
        window_decorations: Some(gpui_kit::WindowDecorations::Server),
        kind: WindowKind::Normal,
        is_resizable: true,
        window_min_size: Some(size(px(termy_x::trends_desk::DOCK_WIDTH), px(320.0))),
        ..Default::default()
    }
}

pub(crate) fn monitor_rect(window: &Window, cx: &App) -> WindowRect {
    window
        .display(cx)
        .map(|display| rect_from_bounds(display.visible_bounds()))
        .or_else(|| {
            cx.displays()
                .first()
                .map(|display| rect_from_bounds(display.visible_bounds()))
        })
        .unwrap_or(WindowRect {
            x: 0.0,
            y: 0.0,
            width: 3440.0,
            height: 1440.0,
        })
}

pub(crate) fn monitor_rect_app(cx: &App) -> WindowRect {
    cx.displays()
        .first()
        .map(|display| rect_from_bounds(display.visible_bounds()))
        .unwrap_or(WindowRect {
            x: 0.0,
            y: 0.0,
            width: 3440.0,
            height: 1440.0,
        })
}

fn rect_from_bounds(bounds: gpui_kit::Bounds<gpui_kit::Pixels>) -> WindowRect {
    WindowRect {
        x: bounds.origin.x.into(),
        y: bounds.origin.y.into(),
        width: bounds.size.width.into(),
        height: bounds.size.height.into(),
    }
}

/// What a second launch should do. A minimized window is restored, then raised.
pub fn focus_steps(minimized: bool) -> &'static [&'static str] {
    if minimized {
        &["restore", "show", "raise", "foreground"]
    } else {
        &["show", "raise", "foreground"]
    }
}

pub(crate) fn focus_existing() {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::WindowsAndMessaging::{
            BringWindowToTop, FindWindowW, GetForegroundWindow, HWND_NOTOPMOST, HWND_TOPMOST,
            IsIconic, SW_RESTORE, SW_SHOW, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
            SetForegroundWindow, SetWindowPos, ShowWindow,
        };
        use windows::core::PCWSTR;
        let title: Vec<u16> = "Termy XL"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let Ok(hwnd) = (unsafe { FindWindowW(PCWSTR::null(), PCWSTR(title.as_ptr())) }) else {
            return;
        };
        let minimized = unsafe { IsIconic(hwnd) }.as_bool();
        let _plan = focus_steps(minimized);
        unsafe {
            if minimized {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = ShowWindow(hwnd, SW_SHOW);
            let foreground = GetForegroundWindow();
            let attached = attach_foreground_thread(foreground, true);
            let _ = BringWindowToTop(hwnd);
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
            );
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_NOTOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
            );
            let _ = SetForegroundWindow(hwnd);
            if attached {
                let _ = attach_foreground_thread(foreground, false);
            }
        }
    }
}

#[cfg(target_os = "windows")]
unsafe fn attach_foreground_thread(
    foreground: windows::Win32::Foundation::HWND,
    attach: bool,
) -> bool {
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    if foreground.is_invalid() {
        return false;
    }
    let foreground_thread = unsafe { GetWindowThreadProcessId(foreground, None) };
    let current = unsafe { GetCurrentThreadId() };
    if foreground_thread == 0 || foreground_thread == current {
        return false;
    }
    unsafe { AttachThreadInput(current, foreground_thread, attach) }.as_bool()
}

pub(crate) struct TermyHost {
    pub rect: WindowRect,
    pub minimized: bool,
}

pub(crate) fn find_termy_x() -> Option<WindowRect> {
    termy_host().map(|host| host.rect)
}

pub(crate) fn termy_host() -> Option<TermyHost> {
    seen_host().map(|host| TermyHost {
        rect: host.rect,
        minimized: host.minimized,
    })
}

pub(crate) fn seen_host() -> Option<termy_x::trends_desk::SeenWindow> {
    termy_x::trends_desk::pick_termy_host(&scan_windows())
}

fn scan_windows() -> Vec<termy_x::trends_desk::SeenWindow> {
    #[cfg(target_os = "windows")]
    {
        scan_windows_win32()
    }
    #[cfg(not(target_os = "windows"))]
    {
        Vec::new()
    }
}

pub(crate) fn ensure_location_hook(root: &std::path::Path) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let registered = register_location_hook();
    termy_x::trends_desk::append_desk_log(
        root,
        if registered {
            "INFO dock: location hook registered"
        } else {
            "INFO dock: location hook failed"
        },
    );
}

fn register_location_hook() -> bool {
    #[cfg(target_os = "windows")]
    {
        register_location_hook_win32()
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

pub(crate) fn log_attach(
    root: &std::path::Path,
    monitor: termy_x::trends_desk::WindowRect,
    expanded: bool,
    docked_setting: bool,
) {
    use std::sync::Mutex;
    static LAST: Mutex<String> = Mutex::new(String::new());
    let host = seen_host();
    let (_snap, line) =
        termy_x::trends_desk::attach_plan(host.as_ref(), monitor, expanded, docked_setting);
    let mut last = LAST.lock().unwrap_or_else(|err| err.into_inner());
    if *last == line {
        return;
    }
    *last = line.clone();
    termy_x::trends_desk::append_desk_log(root, &line);
}

pub(crate) fn fit_window(window: &mut Window) -> bool {
    #[cfg(target_os = "windows")]
    {
        crate::native_sdk::win32_window::register_window(
            window,
            termy_x::trends_desk::VISIBLE_NAME,
        );
        if let Some(hwnd) = crate::native_sdk::win32_window::hwnd_for_window(window) {
            return fit_hwnd(hwnd);
        }
        return false;
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = window;
        true
    }
}

pub(crate) fn place_frame(window: &mut Window, frame: &DeskFrame) -> bool {
    #[cfg(target_os = "windows")]
    if place_win32(window, frame) {
        return true;
    }
    window.resize(size(px(frame.width), px(frame.height)));
    false
}

impl TrendsDesk {
    pub(crate) fn follow_host(&mut self, window: &mut Window, cx: &mut gpui_kit::Context<Self>) {
        if self
            .last_follow
            .is_some_and(|at| at.elapsed() < std::time::Duration::from_millis(200))
        {
            return;
        }
        self.last_follow = Some(std::time::Instant::now());
        ensure_location_hook(&self.data_root);
        log_attach(
            &self.data_root,
            monitor_rect(window, cx),
            self.frame.expanded,
            self.settings.docked,
        );
        let host = termy_host();
        if host.as_ref().map(|item| (item.rect, item.minimized)) == self.last_host_key {
            return;
        }
        self.last_host_key = host.as_ref().map(|item| (item.rect, item.minimized));
        self.last_host = host.as_ref().map(|item| item.rect);
        self.apply_snap(window, cx, false);
    }

    pub(crate) fn retarget(&mut self, window: &mut Window, cx: &mut gpui_kit::Context<Self>) {
        self.apply_snap(window, cx, true);
    }

    fn apply_snap(&mut self, window: &mut Window, cx: &mut gpui_kit::Context<Self>, save: bool) {
        let monitor = monitor_rect(window, cx);
        let host = termy_host();
        let snap = snap_tracked(
            host.as_ref().map(|item| item.rect),
            host.as_ref().is_some_and(|item| item.minimized),
            monitor,
            self.frame.expanded,
            self.frame.expanded_width,
        );
        self.frame.x = snap.x;
        self.frame.y = snap.y;
        self.frame.width = snap.width;
        self.frame.height = snap.height;
        place_frame(window, &self.frame);
        if save {
            self.write_frame();
        }
        cx.notify();
    }
}

#[cfg(target_os = "windows")]
fn scan_windows_win32() -> Vec<termy_x::trends_desk::SeenWindow> {
    #[repr(C)]
    struct RectWin {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }
    struct Bag {
        windows: Vec<termy_x::trends_desk::SeenWindow>,
    }
    unsafe extern "system" {
        fn EnumWindows(
            proc: Option<unsafe extern "system" fn(isize, isize) -> i32>,
            param: isize,
        ) -> i32;
        fn IsWindowVisible(hwnd: isize) -> i32;
        fn IsIconic(hwnd: isize) -> i32;
        fn GetWindowTextW(hwnd: isize, buf: *mut u16, max: i32) -> i32;
        fn GetWindowThreadProcessId(hwnd: isize, pid: *mut u32) -> u32;
        fn GetWindowRect(hwnd: isize, rect: *mut RectWin) -> i32;
    }

    unsafe extern "system" fn each(hwnd: isize, param: isize) -> i32 {
        let bag = unsafe { &mut *(param as *mut Bag) };
        let minimized = unsafe { IsIconic(hwnd) } != 0;
        if unsafe { IsWindowVisible(hwnd) } == 0 && !minimized {
            return 1;
        }
        let mut title_buf = [0u16; 512];
        let title_len =
            unsafe { GetWindowTextW(hwnd, title_buf.as_mut_ptr(), title_buf.len() as i32) };
        let title = String::from_utf16_lossy(&title_buf[..title_len.max(0) as usize]);
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        let image = process_image(pid).unwrap_or_default();
        if title.is_empty() && image.is_empty() {
            return 1;
        }
        let mut rect = RectWin {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
            return 1;
        }
        bag.windows.push(termy_x::trends_desk::SeenWindow {
            title,
            image,
            minimized,
            rect: WindowRect {
                x: rect.left as f32,
                y: rect.top as f32,
                width: (rect.right - rect.left) as f32,
                height: (rect.bottom - rect.top) as f32,
            },
        });
        1
    }

    let mut bag = Bag {
        windows: Vec::new(),
    };
    unsafe { EnumWindows(Some(each), std::ptr::from_mut(&mut bag) as isize) };
    bag.windows
}

#[cfg(target_os = "windows")]
fn process_image(pid: u32) -> Option<String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };
    use windows::core::PWSTR;
    if pid == 0 {
        return None;
    }
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buffer = [0u16; 1024];
    let mut len = buffer.len() as u32;
    let result = unsafe {
        QueryFullProcessImageNameW(
            handle,
            Default::default(),
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        )
    };
    let _ = unsafe { CloseHandle(handle) };
    result.ok()?;
    Some(String::from_utf16_lossy(&buffer[..len as usize]))
}

#[cfg(target_os = "windows")]
fn register_location_hook_win32() -> bool {
    use std::sync::atomic::{AtomicBool, Ordering};
    static MOVED: AtomicBool = AtomicBool::new(false);

    unsafe extern "system" {
        fn SetWinEventHook(
            min: u32,
            max: u32,
            module: *mut core::ffi::c_void,
            proc: Option<
                unsafe extern "system" fn(*mut core::ffi::c_void, u32, isize, i32, i32, u32, u32),
            >,
            pid: u32,
            tid: u32,
            flags: u32,
        ) -> *mut core::ffi::c_void;
    }

    unsafe extern "system" fn on_location(
        _hook: *mut core::ffi::c_void,
        event: u32,
        _hwnd: isize,
        _object: i32,
        _child: i32,
        _thread: u32,
        _time: u32,
    ) {
        if event == 0x800B {
            MOVED.store(true, Ordering::Relaxed);
        }
    }

    let hook = unsafe {
        SetWinEventHook(
            0x800B,
            0x800B,
            core::ptr::null_mut(),
            Some(on_location),
            0,
            0,
            0,
        )
    };
    if hook.is_null() {
        return false;
    }
    // The hook is a raw pointer. It stays installed until UnhookWinEvent,
    // which this process never calls.
    let _ = hook;
    let _ = MOVED.load(Ordering::Relaxed);
    true
}

#[cfg(target_os = "windows")]
fn place_win32(window: &mut Window, frame: &DeskFrame) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos};

    crate::native_sdk::win32_window::register_window(window, termy_x::trends_desk::VISIBLE_NAME);
    let Some(hwnd) = crate::native_sdk::win32_window::hwnd_for_window(window) else {
        return false;
    };
    let placed = unsafe {
        SetWindowPos(
            hwnd,
            None,
            frame.x.round() as i32,
            frame.y.round() as i32,
            frame.width.round() as i32,
            frame.height.round() as i32,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
        .is_ok()
    };
    fit_hwnd(hwnd);
    placed
}

#[cfg(target_os = "windows")]
fn fit_hwnd(hwnd: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowRect, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos,
    };

    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
        return false;
    }
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_invalid() {
        return false;
    }
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        rcMonitor: RECT::default(),
        rcWork: RECT::default(),
        dwFlags: 0,
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return false;
    }
    let work = info.rcWork;
    let (x, y, width, height) = termy_x::trends_desk::clamp_outer(
        rect.left,
        rect.top,
        rect.right - rect.left,
        rect.bottom - rect.top,
        work.left,
        work.top,
        work.right,
        work.bottom,
    );
    if x == rect.left
        && y == rect.top
        && width == rect.right - rect.left
        && height == rect.bottom - rect.top
    {
        return true;
    }
    unsafe {
        SetWindowPos(
            hwnd,
            None,
            x,
            y,
            width,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
        .is_ok()
    }
}
