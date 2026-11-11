//! Place Termy XL against Termy X, or on the right half of the monitor.

use gpui_kit::{App, Window, px, size};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::trends_desk::TrendsDesk;
use termy_x::trends_desk::{DeskFrame, WindowRect, is_termy_x_title, snap_beside};

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
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowRect, GetWindowTextW, IsWindowVisible,
    };

    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let slot = &mut *(lparam.0 as *mut Option<RECT>);
        if !IsWindowVisible(hwnd).as_bool() {
            return BOOL(1);
        }
        let mut buf = [0u16; 256];
        let len = GetWindowTextW(hwnd, &mut buf);
        if len > 0 {
            let title = String::from_utf16_lossy(&buf[..len as usize]);
            if is_termy_x_title(&title) {
                let mut rect = RECT::default();
                if GetWindowRect(hwnd, &mut rect).is_ok() {
                    *slot = Some(rect);
                    return BOOL(0);
                }
            }
        }
        BOOL(1)
    }

    let mut found: Option<RECT> = None;
    unsafe {
        let _ = EnumWindows(
            Some(visit),
            LPARAM(&mut found as *mut Option<RECT> as isize),
        );
    }
    found.map(|rect| WindowRect {
        x: rect.left as f32,
        y: rect.top as f32,
        width: (rect.right - rect.left) as f32,
        height: (rect.bottom - rect.top) as f32,
    })
}

#[cfg(target_os = "windows")]
fn place_win32(window: &mut Window, frame: &DeskFrame) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos};

    let Ok(handle) = window.window_handle() else {
        return false;
    };
    let RawWindowHandle::Win32(raw) = handle.as_raw() else {
        return false;
    };
    let hwnd = HWND(raw.hwnd.get() as *mut core::ffi::c_void);
    unsafe {
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
    }
}
