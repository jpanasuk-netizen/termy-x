//! Where Termy XL sits relative to Termy X.
//!
//! The dock's left edge is Termy X's right edge. The workspace grows to the
//! right from that edge. If Termy X is not found, XL uses the right half of
//! the monitor.

use super::{COMPANION_WIDTH, DOCK_WIDTH};

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
    if lower.contains("termy xl") {
        return false;
    }
    lower == "termy" || lower.starts_with("termy ")
}

pub fn snap_beside(
    host: Option<WindowRect>,
    monitor: WindowRect,
    expanded: bool,
    preferred_width: f32,
) -> Snap {
    let preferred = if preferred_width <= DOCK_WIDTH {
        COMPANION_WIDTH
    } else {
        preferred_width
    };
    let (x, y, height, source) = if let Some(host) = host {
        (
            host.x + host.width,
            host.y,
            host.height.max(240.0),
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
    let room = (monitor.x + monitor.width - x).max(DOCK_WIDTH);
    let width = if expanded {
        preferred.min(room)
    } else {
        DOCK_WIDTH.min(room)
    };
    Snap {
        x,
        y,
        width,
        height,
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
        assert_eq!(dock.y, 20.0);
        assert_eq!(dock.width, DOCK_WIDTH);
        assert_eq!(dock.height, 820.0);
        let open = snap_beside(Some(host()), monitor(), true, 2560.0);
        assert_eq!(open.x, dock.x);
        assert_eq!(open.y, dock.y);
        assert_eq!(open.width, 3440.0 - 1320.0);
        let wide = WindowRect {
            width: 5000.0,
            ..monitor()
        };
        let full = snap_beside(Some(host()), wide, true, 2560.0);
        assert_eq!(full.x, 1320.0);
        assert_eq!(full.width, 2560.0);
    }

    #[test]
    fn missing_host_uses_the_right_half() {
        let dock = snap_beside(None, monitor(), false, 2560.0);
        assert_eq!(dock.source, SnapSource::MonitorHalf);
        assert_eq!(dock.x, 1720.0);
        assert_eq!(dock.width, DOCK_WIDTH);
        let open = snap_beside(None, monitor(), true, 2560.0);
        assert_eq!(open.x, 1720.0);
        assert!(open.width <= 3440.0 - 1720.0);
    }

    #[test]
    fn follow_tracks_a_moved_termy_window() {
        let mut host = host();
        let before = snap_beside(Some(host), monitor(), false, 2560.0);
        host.x += 80.0;
        host.height = 900.0;
        let after = snap_beside(Some(host), monitor(), false, 2560.0);
        assert_eq!(after.x, before.x + 80.0);
        assert_eq!(after.height, 900.0);
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
