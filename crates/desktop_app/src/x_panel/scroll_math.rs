//! Pure scroll math for the tweet list: Chrome-style middle-press
//! autoscroll velocity, Chrome wheel deltas, and scrollbar geometry.
//! Kept side-effect free so it is unit-testable without a window.

/// Chrome-style middle-press autoscroll velocity, in scroll-offset
/// px/sec (negative = list scrolls down). Chrome does not map mouse
/// position to scroll position: while engaged, the list keeps scrolling
/// at a speed proportional to how far the cursor sits from the press
/// anchor, stops inside a small dead zone around the anchor, and
/// reverses on the other side. The cursor is tracked globally (even
/// outside the window), so leaving the window never pauses it. Gain/cap
/// keep it a slow, controlled drift: ~130px/s at 50px from the anchor,
/// ~280px/s at 100px, capped at 600px/s far away.
pub fn autoscroll_velocity(anchor_y: f32, mouse_y: f32) -> f32 {
    let dy = mouse_y - anchor_y;
    let dead = 6.0;
    if dy.abs() <= dead {
        return 0.0;
    }
    let speed = ((dy.abs() - dead) * 3.0).min(600.0);
    if dy > 0.0 { -speed } else { speed }
}

/// Chrome wheel feel for line-based wheels. GPUI's Windows platform
/// reports one wheel notch as one line (measured on-device: the raw
/// native scroll is ~26px/notch, one line-height), so one line maps to
/// Chrome's standard ~100px notch. Capped so a single event never throws
/// more than ~110px regardless of driver multipliers.
pub fn chrome_line_delta(lines_y: f32) -> f32 {
    (lines_y * 100.0).clamp(-110.0, 110.0)
}

/// Draggable-scrollbar geometry: (thumb_y, thumb_h) inside a viewport of
/// `viewport_h` with `max_offset` of scroll range at offset `offset_y`.
pub fn thumb_geometry(viewport_h: f32, max_offset: f32, offset_y: f32) -> (f32, f32) {
    if viewport_h <= 0.0 || max_offset <= 0.0 {
        return (0.0, viewport_h.max(0.0));
    }
    let content_h = viewport_h + max_offset;
    let thumb_h = (viewport_h * viewport_h / content_h).clamp(28.0, viewport_h);
    let frac = (-offset_y / max_offset).clamp(0.0, 1.0);
    (frac * (viewport_h - thumb_h), thumb_h)
}

/// Inverse: scroll offset for a thumb dragged to `thumb_y`.
pub fn offset_for_thumb_y(viewport_h: f32, max_offset: f32, thumb_y: f32) -> f32 {
    let (_, thumb_h) = thumb_geometry(viewport_h, max_offset, 0.0);
    let travel = (viewport_h - thumb_h).max(1.0);
    let frac = (thumb_y / travel).clamp(0.0, 1.0);
    -frac * max_offset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autoscroll_velocity_is_chrome_shaped() {
        // Dead zone around the anchor: no motion.
        assert_eq!(autoscroll_velocity(100.0, 100.0), 0.0);
        assert_eq!(autoscroll_velocity(100.0, 105.0), 0.0);
        // Below anchor: scrolls down (offset velocity negative), slow.
        let v = autoscroll_velocity(100.0, 150.0);
        assert!(
            (-180.0..-90.0).contains(&v),
            "50px out should drift ~130px/s, got {v}"
        );
        // Above anchor: scrolls up, symmetric.
        assert_eq!(autoscroll_velocity(100.0, 50.0), -v);
        // Far away: capped, never a teleport.
        assert_eq!(autoscroll_velocity(100.0, 5000.0), -600.0);
        assert_eq!(autoscroll_velocity(100.0, -5000.0), 600.0);
    }

    #[test]
    fn wheel_line_delta_matches_chrome_notch() {
        // One GPUI line = one wheel notch ~= 100px, as in Chrome.
        assert_eq!(chrome_line_delta(1.0), 100.0);
        assert_eq!(chrome_line_delta(-1.0), -100.0);
        // Driver multipliers are capped per event.
        assert_eq!(chrome_line_delta(3.0), 110.0);
        assert_eq!(chrome_line_delta(0.0), 0.0);
    }

    #[test]
    fn thumb_geometry_and_inverse_round_trip() {
        let (y, h) = thumb_geometry(600.0, 5400.0, 0.0);
        assert_eq!(y, 0.0);
        assert!(h >= 28.0 && h < 600.0);
        let (y2, _) = thumb_geometry(600.0, 5400.0, -5400.0);
        assert_eq!(y2, 600.0 - h);
        let off = offset_for_thumb_y(600.0, 5400.0, y2);
        assert!((off - -5400.0).abs() < 1.0);
        // No scroll range: full-height thumb, zero offset.
        assert_eq!(thumb_geometry(600.0, 0.0, 0.0), (0.0, 600.0));
        assert_eq!(offset_for_thumb_y(600.0, 0.0, 100.0), 0.0);
    }
}
