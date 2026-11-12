//! Breathing elapsed mark and the gold caret inside a Termy XL take.

use gpui_kit::{Animation, AnimationExt, IntoElement, ParentElement, Styled, div, px, rgb};

pub(crate) fn breath_animation() -> Animation {
    Animation::new(std::time::Duration::from_millis(1600))
        .repeat()
        .with_easing(|t| 0.5 - 0.5 * (t * std::f32::consts::TAU).cos())
}

pub(crate) fn drafting_mark(secs: u64) -> gpui_kit::AnyElement {
    div()
        .text_size(px(28.0))
        .text_color(rgb(0xffe56a))
        .min_h(px(36.0))
        .child(format!("… {secs}s"))
        .with_animation("desk-drafting-breath", breath_animation(), |el, value| {
            el.opacity(0.55 + 0.45 * value)
        })
        .into_any_element()
}

pub(crate) fn reply_surface(
    text: &str,
    caret: &termy_x::draft_edit::DraftCaret,
) -> gpui_kit::AnyElement {
    let len = text.len();
    let (start, end) = caret
        .selection()
        .unwrap_or((caret.caret.min(len), caret.caret.min(len)));
    let start = start.min(len);
    let end = end.min(len).max(start);
    let before = text.get(..start).unwrap_or("");
    let middle = text.get(start..end).unwrap_or("");
    let after = text.get(end..).unwrap_or("");
    let mut row = div().flex().flex_wrap();
    if !before.is_empty() {
        row = row.child(before.to_string());
    }
    if middle.is_empty() {
        row = row.child(
            div()
                .w(px(2.0))
                .h(px(18.0))
                .bg(rgb(0xffe56a))
                .with_animation("desk-caret", breath_animation(), |el, value| {
                    el.opacity(0.15 + 0.85 * value)
                }),
        );
    } else {
        row = row.child(
            div()
                .bg(rgb(0xff4fa3))
                .text_color(rgb(0x070b14))
                .child(middle.to_string()),
        );
    }
    if !after.is_empty() {
        row = row.child(after.to_string());
    }
    row.into_any_element()
}
