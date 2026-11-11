//! Slim Termy XL dock. Expanding changes width only, so the left edge stays.

use gpui_kit::{
    Context, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, px, rgb, size,
};

use crate::trends_desk::TrendsDesk;
use termy_x::trends_desk::{DOCK_WIDTH, step_width, toggle_dock};

impl TrendsDesk {
    pub(crate) fn xl_mark(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        div()
            .id("desk-xl-mark")
            .flex()
            .items_center()
            .justify_center()
            .w(px(64.0))
            .h(px(64.0))
            .rounded(px(18.0))
            .cursor_pointer()
            .text_size(px(28.0))
            .font_weight(FontWeight::BOLD)
            .bg(rgb(0x10243a))
            .text_color(rgb(0x7af6ff))
            .border_1()
            .border_color(rgb(0x7af6ff))
            .hover(|style| {
                style
                    .bg(rgb(0x2a1030))
                    .text_color(rgb(0xff4fa3))
                    .border_color(rgb(0xff4fa3))
            })
            .child("XL")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.toggle_frame(window, cx)),
            )
            .into_any_element()
    }

    pub(crate) fn toggle_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let bounds = window.bounds();
        self.frame.x = bounds.origin.x.into();
        self.frame.y = bounds.origin.y.into();
        self.frame.height = bounds.size.height.into();
        if self.frame.expanded {
            self.frame.width = bounds.size.width.into();
        }
        self.frame = toggle_dock(self.frame);
        self.write_frame();
        self.anim_target = Some(self.frame.width);
        self.step_frame(window, cx);
    }

    pub(crate) fn step_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.anim_target else {
            return;
        };
        let bounds = window.bounds();
        let current: f32 = bounds.size.width.into();
        let next = step_width(current, target);
        window.resize(size(px(next), px(bounds.size.height.into())));
        if (next - target).abs() < 0.5 {
            self.anim_target = None;
            cx.notify();
            return;
        }
        cx.notify();
        let entity = cx.entity().clone();
        window.on_next_frame(move |window, cx| {
            let _ = entity.update(cx, |desk, cx| desk.step_frame(window, cx));
        });
    }

    pub(crate) fn write_frame(&mut self) {
        self.settings.docked = !self.frame.expanded;
        self.settings.frame_x = self.frame.x.round() as i32;
        self.settings.frame_y = self.frame.y.round() as i32;
        self.settings.expanded_width = self.frame.expanded_width.round().max(DOCK_WIDTH) as u32;
        self.settings.frame_height = self.frame.height.round() as u32;
        if self.writable {
            let _ = self.store.save_settings(&self.settings);
        }
    }

    pub(crate) fn ensure_close_hook(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.close_hooked {
            return;
        }
        self.close_hooked = true;
        let entity = cx.entity().clone();
        window.on_window_should_close(cx, move |window, cx| {
            let _ = entity.update(cx, |desk, _| {
                let bounds = window.bounds();
                desk.frame.x = bounds.origin.x.into();
                desk.frame.y = bounds.origin.y.into();
                desk.frame.height = bounds.size.height.into();
                if desk.frame.expanded {
                    desk.frame.width = bounds.size.width.into();
                    desk.frame.expanded_width = desk.frame.width;
                }
                desk.write_frame();
            });
            true
        });
    }
}
