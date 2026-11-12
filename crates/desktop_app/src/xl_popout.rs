//! Reply popout for Termy XL.
//!
//! It follows the Termy X reply box: context line, caret field, character
//! count, and Post/Cancel. Only the Send button posts. Esc and Cancel close
//! it and leave the draft on the card.

use gpui_kit::{
    Context, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px, rgb,
};

use crate::trends_desk::TrendsDesk;
use termy_x::trends_desk::{TakeProvider, TakeTone, char_count, wrap_token};

impl TrendsDesk {
    pub(crate) fn reply_popout(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let Some(pop) = self.popout.clone() else {
            return div().into_any_element();
        };
        let post = self
            .columns
            .iter()
            .flat_map(|column| column.view.posts.iter())
            .find(|post| post.id == pop.card_id);
        let context = post
            .map(|post| format!("@{} · {}", wrap_token(&post.author), wrap_token(&post.text)))
            .unwrap_or_else(|| "Reply".into());
        let count = char_count(&pop.draft);
        let caret = self.pop_caret.caret.min(pop.draft.len());
        let (head, tail) = pop.draft.split_at(caret);
        let over = count > 280;
        let mut box_el = div()
            .id("desk-reply-popout")
            .absolute()
            .top(px(88.0))
            .left(px(48.0))
            .right(px(48.0))
            .p(px(12.0))
            .rounded(px(12.0))
            .bg(rgb(0x2a0f1a))
            .border_2()
            .border_color(rgb(termy_x::trends_desk::SELECT_PINK))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| {
                    cx.stop_propagation();
                }),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgb(0x7af6ff))
                    .whitespace_normal()
                    .child(format!("Reply to {context}")),
            )
            .child(
                div()
                    .id("desk-reply-editor")
                    .h(px(88.0))
                    .overflow_y_scroll()
                    .p(px(8.0))
                    .rounded(px(8.0))
                    .bg(rgb(0x0c1422))
                    .whitespace_normal()
                    .text_size(px(14.0))
                    .child(format!("{head}|{tail}")),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(if over { rgb(0xff5d8f) } else { rgb(0x9ad7ff) })
                    .child({
                        let read = termy_x::trends_desk::read_composer(&pop.draft);
                        format!(
                            "{count} / 280{} · {} mentions · {} links · Send button posts",
                            if read.over { " over limit" } else { "" },
                            read.mentions,
                            read.links
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .flex_wrap()
                    .children(TakeTone::all().map(|tone| {
                        tone_button(
                            &format!("desk-tone-{}", tone.label()),
                            tone.label(),
                            &pop.tone,
                            tone,
                            cx,
                        )
                    })),
            )
            .child(self.agent_controls(cx));
        for (index, variant) in pop.variants.iter().enumerate() {
            let text = variant.clone();
            box_el = box_el.child(
                div()
                    .id(SharedString::from(format!("desk-variant-{index}")))
                    .p(px(8.0))
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(rgb(0xffd60a))
                    .bg(rgb(0x1a160c))
                    .whitespace_normal()
                    .cursor_pointer()
                    .child(format!("Variant {} · {}", index + 1, wrap_token(variant)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            if let Some(pop) = this.popout.as_mut() {
                                pop.draft = text.clone();
                                this.pop_caret =
                                    termy_x::draft_edit::DraftCaret::at_end(text.len());
                            }
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            );
        }
        box_el
            .child(div().text_size(px(12.0)).child(format!(
                "Provider · {}",
                match pop.provider {
                    TakeProvider::Treg => "Treg",
                    TakeProvider::Hermes => "Hermes",
                    TakeProvider::Grok => "Grok",
                    TakeProvider::Free => "Free",
                }
            )))
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(pop_button("desk-pop-grok", "Grok", cx, |this, cx| {
                        this.set_take_provider("grok", cx);
                        this.regenerate_open_variants(cx);
                    }))
                    .child(pop_button("desk-pop-free", "Free", cx, |this, cx| {
                        this.set_take_provider("free", cx);
                        this.regenerate_open_variants(cx);
                    }))
                    .child(pop_button("desk-pop-treg", "Treg", cx, |this, cx| {
                        this.set_take_provider("treg", cx);
                        this.regenerate_open_variants(cx);
                    }))
                    .child(pop_button("desk-pop-hermes", "Hermes", cx, |this, cx| {
                        this.set_take_provider("hermes", cx);
                        this.regenerate_open_variants(cx);
                    }))
                    .child(pop_button(
                        "desk-variant-next",
                        "Next variant",
                        cx,
                        |this, cx| {
                            if let Some(pop) = this.popout.as_mut() {
                                let index = pop
                                    .variants
                                    .iter()
                                    .position(|item| item == &pop.draft)
                                    .unwrap_or(0);
                                let next = termy_x::trends_desk::flip_variant(
                                    index,
                                    1,
                                    pop.variants.len(),
                                );
                                if let Some(text) = pop.variants.get(next).cloned() {
                                    pop.draft = text.clone();
                                    this.pop_caret =
                                        termy_x::draft_edit::DraftCaret::at_end(text.len());
                                }
                            }
                            cx.notify();
                        },
                    ))
                    .child(pop_button("desk-pop-send", "Send", cx, |this, cx| {
                        this.send_from_popout(cx);
                    }))
                    .child(pop_button("desk-pop-cancel", "Cancel", cx, |this, cx| {
                        this.close_popout();
                        cx.notify();
                    })),
            )
            .into_any_element()
    }
}

fn tone_button(
    id: &str,
    label: &str,
    current: &str,
    tone: TakeTone,
    cx: &mut Context<TrendsDesk>,
) -> gpui_kit::AnyElement {
    let active = current.eq_ignore_ascii_case(tone.label());
    let name = tone.label().to_string();
    pop_button(id, label, cx, move |this, cx| {
        if active {
            return;
        }
        this.set_pop_tone(&name);
        this.regenerate_open_variants(cx);
        cx.notify();
    })
}

fn pop_button(
    id: &str,
    label: &str,
    cx: &mut Context<TrendsDesk>,
    handler: impl Fn(&mut TrendsDesk, &mut Context<TrendsDesk>) + 'static,
) -> gpui_kit::AnyElement {
    div()
        .id(SharedString::from(id.to_string()))
        .px(px(10.0))
        .py(px(6.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(rgb(0x3a3a3c))
        .bg(rgb(0x2c2c2e))
        .text_size(px(12.0))
        .cursor_pointer()
        .child(label.to_string())
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                handler(this, cx);
                cx.stop_propagation();
            }),
        )
        .into_any_element()
}
