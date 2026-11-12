//! Pink draft box inside a Termy XL gold card.
//!
//! Draft take opens it on the click. Funny, Smart, and Spicy swap the reply.
//! Copy copies it. Send is the only control that posts. Esc and Close keep the text.

use gpui_kit::{
    Context, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px, rgb,
};

use crate::trends_desk::TrendsDesk;
use termy_x::trends_desk::{
    TakeTone, apply_variant, panel_tone_name, panel_tones, pink_box_controls,
};

impl TrendsDesk {
    pub(crate) fn inline_editor(
        &self,
        card_id: &str,
        cx: &mut Context<Self>,
    ) -> Option<gpui_kit::AnyElement> {
        let pop = self.popout.clone()?;
        if !pop.open || pop.card_id != card_id {
            return None;
        }
        let controls = pink_box_controls();
        let drafting = pop.draft == termy_x::trends_desk::DRAFTING_PLACEHOLDER;
        let caret = self.pop_caret.caret.min(pop.draft.len());
        let (head, tail) = pop.draft.split_at(caret);
        let field = if drafting {
            termy_x::trends_desk::DRAFTING_PLACEHOLDER.to_string()
        } else {
            format!("{head}|{tail}")
        };
        let sending = self
            .card_replies
            .get(&pop.card_id)
            .is_some_and(termy_x::trends_desk::send_is_busy);
        let mut box_el = div()
            .id("desk-inline-editor")
            .w_full()
            .min_w(px(0.0))
            .p(px(10.0))
            .rounded(px(12.0))
            .bg(rgb(termy_x::trends_desk::SELECT_PINK))
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
                    .id("desk-reply-editor")
                    .min_h(px(72.0))
                    .p(px(8.0))
                    .rounded(px(8.0))
                    .bg(rgb(0x2a1020))
                    .text_size(px(14.0))
                    .text_color(rgb(0xffe56a))
                    .whitespace_normal()
                    .child(field),
            );
        let mut chips = div().flex().flex_wrap().gap(px(8.0));
        for (index, tone) in panel_tones().into_iter().enumerate() {
            chips = chips.child(variant_chip(panel_tone_name(tone), index, cx));
        }
        box_el = box_el.child(chips);
        if !self.action_note.is_empty() {
            box_el = box_el.child(
                div()
                    .text_size(px(12.0))
                    .text_color(rgb(0x070b14))
                    .whitespace_normal()
                    .child(self.action_note.clone()),
            );
        }
        let send_label = if sending { "Sending…" } else { controls[4] };
        Some(
            box_el
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(8.0))
                        .child(box_button(
                            "desk-reply-copy",
                            controls[3],
                            cx,
                            |this, cx| {
                                if let Some(pop) = this.popout.clone() {
                                    this.copy_text(&pop.draft, cx);
                                    this.action_note = "Copied the reply.".into();
                                }
                                cx.notify();
                            },
                        ))
                        .child(box_button("desk-pop-send", send_label, cx, |this, cx| {
                            this.send_from_popout(cx);
                            cx.notify();
                        }))
                        .child(box_button("desk-inline-close", "Close", cx, |this, cx| {
                            this.close_popout();
                            this.action_note =
                                "Closed the draft. The text is still on the card.".into();
                            cx.notify();
                        })),
                )
                .into_any_element(),
        )
    }

    pub(crate) fn show_inline_editor(&mut self, id: &str, text: &str) {
        let provider = termy_x::trends_desk::TakeProvider::parse(&self.settings.take_provider);
        let tone = panel_tone_name(TakeTone::parse(&self.pack.tone)).to_string();
        let stored = self
            .takes
            .get(id)
            .map(|take| take.variants.clone())
            .unwrap_or_default();
        let mut editor = if text == termy_x::trends_desk::DRAFTING_PLACEHOLDER {
            termy_x::trends_desk::editor_on_draft_click(id, provider)
        } else {
            termy_x::trends_desk::open_editor(id, text, provider)
        };
        editor.tone = tone;
        if !stored.is_empty() {
            editor.variants = termy_x::trends_desk::distinct_variants(text, &stored);
        }
        self.pop_caret = termy_x::draft_edit::DraftCaret::at_end(text.len());
        self.selected = Some(termy_x::trends_desk::select_take(id));
        self.card_replies.insert(
            id.to_string(),
            termy_x::trends_desk::ReplyPhase::Editing {
                text: text.to_string(),
            },
        );
        self.popout = Some(editor);
    }

    pub(crate) fn open_inline_for(&mut self, id: String, cx: &mut Context<Self>) {
        let existing = self
            .card_replies
            .get(&id)
            .map(|phase| phase.text().to_string())
            .filter(|text| !termy_x::trends_desk::reply_needs_a_take(text))
            .or_else(|| {
                self.takes
                    .get(&id)
                    .map(|take| take.text.clone())
                    .filter(|text| !termy_x::trends_desk::reply_needs_a_take(text))
            })
            .unwrap_or_default();
        if termy_x::trends_desk::reply_needs_a_take(&existing) {
            self.draft_take_for(id, cx);
        } else {
            self.show_inline_editor(&id, &existing);
            self.action_note = "Draft is open in the card.".into();
            cx.notify();
        }
    }
}

fn variant_chip(
    label: &'static str,
    index: usize,
    cx: &mut Context<TrendsDesk>,
) -> gpui_kit::AnyElement {
    box_button(&format!("desk-tone-{label}"), label, cx, move |this, cx| {
        let missing = format!("No {label} line yet.");
        if let Some(pop) = this.popout.as_mut() {
            let variants = pop.variants.clone();
            if apply_variant(&mut pop.draft, &variants, index) {
                let text = pop.draft.clone();
                let id = pop.card_id.clone();
                this.pop_caret = termy_x::draft_edit::DraftCaret::at_end(text.len());
                this.card_replies
                    .insert(id, termy_x::trends_desk::ReplyPhase::Editing { text });
                this.action_note = format!("Showing {label}.");
            } else {
                this.action_note = missing;
            }
        }
        cx.notify();
    })
}

fn box_button(
    id: &str,
    label: &str,
    cx: &mut Context<TrendsDesk>,
    handler: impl Fn(&mut TrendsDesk, &mut Context<TrendsDesk>) + 'static,
) -> gpui_kit::AnyElement {
    div()
        .id(SharedString::from(id.to_string()))
        .flex_shrink_0()
        .min_h(px(36.0))
        .px(px(12.0))
        .py(px(8.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(rgb(0xffe56a))
        .bg(rgb(0x2a1020))
        .text_size(px(13.0))
        .text_color(rgb(0xffe56a))
        .cursor_pointer()
        .hover(|style| {
            style
                .bg(rgb(0xffe56a))
                .text_color(rgb(0x2a1020))
                .border_color(rgb(0xffe56a))
        })
        .active(|style| {
            style
                .bg(rgb(0x070b14))
                .text_color(rgb(0x7af6ff))
                .border_color(rgb(0x7af6ff))
        })
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
