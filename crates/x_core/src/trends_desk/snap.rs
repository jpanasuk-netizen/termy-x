//! Where Termy XL sits relative to Termy X.
//!
//! The dock's left edge is Termy X's right edge. The workspace grows to the
//! right from that edge. If Termy X is not found, XL uses the right half of
//! the monitor.

use super::{DOCK_WIDTH, LOCKED_OPEN_WIDTH};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapSource {
    HostEdge,
    MonitorHalf,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snap {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub source: SnapSource,
}

/// True for the Termy X window. Termy XL's own title does not match.
pub fn is_termy_x_title(title: &str) -> bool {
    let title = title.trim();
    let lower = title.to_ascii_lowercase();
    if lower.contains("termy xl") || lower.contains("trends_desk") {
        return false;
    }
    lower == "termy"
        || lower.starts_with("termy ")
        || lower.ends_with(" - termy")
        || lower.contains(" - termy")
}

/// Pull an outer window rectangle fully inside the work area. Width stays
/// unless it is wider than the work area.
pub fn clamp_outer(
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
) -> (i32, i32, i32, i32) {
    let max_width = (right - left).max(1);
    let max_height = (bottom - top).max(1);
    let width = width.min(max_width).max(1);
    let height = height.min(max_height).max(1);
    let mut x = x.max(left);
    let mut y = y.max(top);
    if x + width > right {
        x = right - width;
    }
    if y + height > bottom {
        y = bottom - height;
    }
    (x, y, width, height)
}

pub fn snap_beside(
    host: Option<WindowRect>,
    monitor: WindowRect,
    expanded: bool,
    saved_width: f32,
) -> Snap {
    snap_tracked(host, false, monitor, expanded, saved_width)
}

/// Place XL on Termy X's right edge, flush with that window.
/// A minimized or missing Termy X uses the standalone right half.
/// The open width stays the locked 687.
pub fn snap_tracked(
    host: Option<WindowRect>,
    minimized: bool,
    monitor: WindowRect,
    expanded: bool,
    _saved_width: f32,
) -> Snap {
    let width = if expanded {
        LOCKED_OPEN_WIDTH
    } else {
        DOCK_WIDTH
    };
    let attached = host.filter(|rect| !minimized && rect.width > 40.0 && rect.height > 40.0);
    let (x, y, height, source) = if let Some(host) = attached {
        (
            host.x + host.width,
            host.y,
            host.height,
            SnapSource::HostEdge,
        )
    } else {
        (
            monitor.x + monitor.width / 2.0,
            monitor.y,
            monitor.height.max(240.0),
            SnapSource::MonitorHalf,
        )
    };
    let (x, y, width, height) = clamp_outer(
        x.round() as i32,
        y.round() as i32,
        width.round() as i32,
        height.round() as i32,
        monitor.x.round() as i32,
        monitor.y.round() as i32,
        (monitor.x + monitor.width).round() as i32,
        (monitor.y + monitor.height).round() as i32,
    );
    Snap {
        x: x as f32,
        y: y as f32,
        width: width as f32,
        height: height as f32,
        source,
    }
}

/// Termy X areas XL will host. Writes stay behind an explicit click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XlArea {
    Desk,
    Timeline,
    Search,
    Trends,
    Compose,
    Reply,
    Repost,
    Quote,
    Follow,
    Viewer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteGate {
    ReadOnly,
    ExplicitClick,
    ImageUnsupported,
}

pub fn area_gate(area: XlArea, image_quote: bool) -> WriteGate {
    if area == XlArea::Quote && image_quote {
        return WriteGate::ImageUnsupported;
    }
    match area {
        XlArea::Desk | XlArea::Timeline | XlArea::Search | XlArea::Trends | XlArea::Viewer => {
            WriteGate::ReadOnly
        }
        XlArea::Compose | XlArea::Reply | XlArea::Repost | XlArea::Quote | XlArea::Follow => {
            WriteGate::ExplicitClick
        }
    }
}

pub fn termy_areas() -> &'static [XlArea] {
    &[
        XlArea::Timeline,
        XlArea::Search,
        XlArea::Trends,
        XlArea::Compose,
        XlArea::Reply,
        XlArea::Repost,
        XlArea::Quote,
        XlArea::Follow,
        XlArea::Viewer,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trends_desk::LOCKED_OPEN_WIDTH;

    fn monitor() -> WindowRect {
        WindowRect {
            x: 0.0,
            y: 0.0,
            width: 3440.0,
            height: 1440.0,
        }
    }

    fn host() -> WindowRect {
        WindowRect {
            x: 40.0,
            y: 20.0,
            width: 1280.0,
            height: 820.0,
        }
    }

    #[test]
    fn dock_sits_on_termy_x_right_edge_and_expands_right() {
        let dock = snap_beside(Some(host()), monitor(), false, 2560.0);
        assert_eq!(dock.source, SnapSource::HostEdge);
        assert_eq!(dock.x, 1320.0);
        assert_eq!(dock.y, host().y);
        assert_eq!(dock.width, DOCK_WIDTH);
        assert_eq!(dock.height, host().height);
        let open = snap_beside(Some(host()), monitor(), true, 2560.0);
        assert_eq!(open.x, dock.x);
        assert_eq!(open.y, host().y);
        assert_eq!(open.width, LOCKED_OPEN_WIDTH);
        assert_eq!(open.height, host().height);
        let stale = snap_beside(Some(host()), monitor(), true, 1800.0);
        assert_eq!(stale.width, LOCKED_OPEN_WIDTH);
    }

    #[test]
    fn missing_host_uses_the_right_half() {
        let dock = snap_beside(None, monitor(), false, 2560.0);
        assert_eq!(dock.source, SnapSource::MonitorHalf);
        assert_eq!(dock.x, 1720.0);
        assert_eq!(dock.width, DOCK_WIDTH);
        let open = snap_beside(None, monitor(), true, 2560.0);
        assert_eq!(open.x, 1720.0);
        assert_eq!(open.width, LOCKED_OPEN_WIDTH);
    }

    #[test]
    fn follow_tracks_a_moved_termy_window() {
        let mut host = host();
        let before = snap_beside(Some(host), monitor(), false, 2560.0);
        host.x += 80.0;
        host.width += 40.0;
        host.y += 15.0;
        host.height = 900.0;
        let after = snap_beside(Some(host), monitor(), false, 2560.0);
        assert_eq!(after.x, before.x + 80.0 + 40.0);
        assert_eq!(after.y, before.y + 15.0);
        assert_eq!(after.height, 900.0);
        assert_eq!(after.source, SnapSource::HostEdge);
        let parked = snap_tracked(Some(host), true, monitor(), false, 2560.0);
        assert_eq!(parked.source, SnapSource::MonitorHalf);
        let back = snap_tracked(Some(host), false, monitor(), true, 2560.0);
        assert_eq!(back.source, SnapSource::HostEdge);
        assert_eq!(back.x, host.x + host.width);
        assert_eq!(back.width, LOCKED_OPEN_WIDTH);
        for (width, height) in [(1920.0, 1080.0), (1536.0, 864.0), (1280.0, 720.0)] {
            let work = WindowRect {
                x: 0.0,
                y: 0.0,
                width,
                height,
            };
            let edge = WindowRect {
                x: width - 400.0,
                y: 10.0,
                width: 360.0,
                height: height - 20.0,
            };
            let fitted = snap_beside(Some(edge), work, true, 2560.0);
            assert!(fitted.x >= work.x);
            assert!(fitted.x + fitted.width <= work.x + work.width + 0.5);
            assert!(fitted.y + fitted.height <= work.y + work.height + 0.5);
            assert_eq!(fitted.width, LOCKED_OPEN_WIDTH.min(width));
        }
    }

    #[test]
    fn a_window_above_the_work_area_is_pulled_inside() {
        let (x, y, width, height) = clamp_outer(-10, -23, 687, 2116, 0, 0, 3840, 2100);
        assert_eq!(width, 687);
        assert!(x >= 0 && x + width <= 3840);
        assert!(y >= 0 && y + height <= 2100);
        assert_eq!(y, 0);
        assert_eq!(height, 2100);
    }

    #[test]
    fn title_match_skips_termy_xl() {
        assert!(is_termy_x_title("Termy"));
        assert!(is_termy_x_title("Termy "));
        assert!(!is_termy_x_title("Termy XL"));
        assert!(!is_termy_x_title("Termy XL — Trends"));
    }

    #[test]
    fn writes_need_a_click_and_image_quotes_are_unsupported() {
        for area in termy_areas() {
            if matches!(
                area,
                XlArea::Compose | XlArea::Reply | XlArea::Repost | XlArea::Follow
            ) {
                assert_eq!(area_gate(*area, false), WriteGate::ExplicitClick);
            }
        }
        assert_eq!(area_gate(XlArea::Quote, false), WriteGate::ExplicitClick);
        assert_eq!(area_gate(XlArea::Quote, true), WriteGate::ImageUnsupported);
        assert_eq!(area_gate(XlArea::Timeline, false), WriteGate::ReadOnly);
        assert_eq!(area_gate(XlArea::Search, false), WriteGate::ReadOnly);
    }
}
