//! Place Termy XL against Termy X, or on the right half of the monitor.

use gpui_kit::{App, Window, px, size};

use crate::trends_desk::TrendsDesk;
use termy_x::trends_desk::{DeskFrame, WindowRect, snap_beside};

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

pub(crate) fn focus_existing() {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::WindowsAndMessaging::{
            FindWindowW, SW_RESTORE, SetForegroundWindow, ShowWindow,
        };
        use windows::core::PCWSTR;
        let title: Vec<u16> = "Termy XL"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        if let Ok(hwnd) = unsafe { FindWindowW(PCWSTR::null(), PCWSTR(title.as_ptr())) } {
            unsafe {
                let _ = ShowWindow(hwnd, SW_RESTORE);
                let _ = SetForegroundWindow(hwnd);
            }
        }
    }
}

pub(crate) fn find_termy_x() -> Option<WindowRect> {
    #[cfg(target_os = "windows")]
    {
        find_termy_x_windows()
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
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
            .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(1))
        {
            return;
        }
        self.last_follow = Some(std::time::Instant::now());
        let host = find_termy_x();
        if host == self.last_host {
            return;
        }
        self.last_host = host;
        self.retarget(window, cx);
    }

    pub(crate) fn retarget(&mut self, window: &mut Window, cx: &mut gpui_kit::Context<Self>) {
        let monitor = monitor_rect(window, cx);
        let snap = snap_beside(
            find_termy_x(),
            monitor,
            self.frame.expanded,
            self.frame.expanded_width,
        );
        self.frame.x = snap.x;
        self.frame.y = snap.y;
        self.frame.width = snap.width;
        self.frame.height = snap.height;
        place_frame(window, &self.frame);
        self.write_frame();
        cx.notify();
    }
}

#[cfg(target_os = "windows")]
fn find_termy_x_windows() -> Option<WindowRect> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowRect, IsWindowVisible};
    use windows::core::PCWSTR;

    // Exact title. "Termy XL" does not match.
    let title: Vec<u16> = "Termy".encode_utf16().chain(std::iter::once(0)).collect();
    let hwnd = unsafe { FindWindowW(PCWSTR::null(), PCWSTR(title.as_ptr())) }.ok()?;
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return None;
    }
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }.ok()?;
    Some(WindowRect {
        x: rect.left as f32,
        y: rect.top as f32,
        width: (rect.right - rect.left) as f32,
        height: (rect.bottom - rect.top) as f32,
    })
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
