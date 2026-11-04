//! Pure contain-fitting math for tweet media and the in-app viewer.
//! No cropping, no stretching: the whole frame is always visible.

pub const MEDIA_MIN_H: f32 = 96.0;
pub const MEDIA_MAX_H: f32 = 240.0;
pub const MEDIA_DEFAULT_ASPECT: f32 = 16.0 / 9.0;

/// Height reserved for a media box of `container_w` showing media with
/// aspect `aspect_w / aspect_h` (falls back to 16:9). Clamped so a tall
/// portrait clip reserves a bounded box instead of eating the timeline;
/// the image itself is contain-fitted inside, never cropped.
pub fn media_box_height(container_w: f32, aspect: Option<f32>) -> f32 {
    let a = aspect
        .filter(|a| a.is_finite() && *a > 0.05 && *a < 20.0)
        .unwrap_or(MEDIA_DEFAULT_ASPECT);
    (container_w / a).clamp(MEDIA_MIN_H, MEDIA_MAX_H)
}

/// Exact (w, h) of an image contain-fitted inside `box_w x box_h`.
pub fn contain_size(box_w: f32, box_h: f32, aspect: Option<f32>) -> (f32, f32) {
    let a = aspect
        .filter(|a| a.is_finite() && *a > 0.05 && *a < 20.0)
        .unwrap_or(MEDIA_DEFAULT_ASPECT);
    if box_w / box_h > a {
        (box_h * a, box_h)
    } else {
        (box_w, box_w / a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn landscape_uses_full_width() {
        assert_eq!(media_box_height(424.0, Some(16.0 / 9.0)), 238.5);
        assert_eq!(contain_size(424.0, 238.5, Some(16.0 / 9.0)), (424.0, 238.5));
    }
    #[test]
    fn portrait_is_capped_and_letterboxed_not_cropped() {
        // 9:16 portrait in a 424-wide card: uncapped height would be 753px.
        assert_eq!(media_box_height(424.0, Some(9.0 / 16.0)), MEDIA_MAX_H);
        let (w, h) = contain_size(424.0, MEDIA_MAX_H, Some(9.0 / 16.0));
        assert_eq!((w, h), (135.0, 240.0)); // whole frame, shrunk to fit the capped area
    }
    #[test]
    fn unknown_aspect_defaults_to_16_9_and_square_fits() {
        assert_eq!(media_box_height(424.0, None), 238.5);
        assert_eq!(contain_size(300.0, 300.0, Some(1.0)), (300.0, 300.0));
        assert_eq!(media_box_height(424.0, Some(0.0)), 238.5);
    }
}

/// Fraction 0..=1 of a seek bar click at window-x `click_x`, where the
/// bar starts at `bar_x` and is `bar_w` wide.
pub fn seek_fraction(click_x: f32, bar_x: f32, bar_w: f32) -> f32 {
    if bar_w <= 1.0 {
        return 0.0;
    }
    ((click_x - bar_x) / bar_w).clamp(0.0, 1.0)
}

/// "1:23" / "12:05" for a seconds position (negatives/NaN clamp to 0).
pub fn format_time(seconds: f64) -> String {
    let total = if seconds.is_finite() && seconds > 0.0 {
        seconds as u64
    } else {
        0
    };
    format!("{}:{:02}", total / 60, total % 60)
}

#[cfg(test)]
mod seek_tests {
    use super::*;
    #[test]
    fn seek_fraction_clamps_and_time_formats() {
        assert_eq!(seek_fraction(150.0, 100.0, 200.0), 0.25);
        assert_eq!(seek_fraction(50.0, 100.0, 200.0), 0.0);
        assert_eq!(seek_fraction(999.0, 100.0, 200.0), 1.0);
        assert_eq!(format_time(83.0), "1:23");
        assert_eq!(format_time(725.0), "12:05");
        assert_eq!(format_time(-3.0), "0:00");
    }
}
