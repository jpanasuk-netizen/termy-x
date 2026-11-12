//! Inline reply panel inside a Termy XL gold card.
//!
//! Draft take, Reply, and Edit open this panel in the card. Only Send posts.
//! Esc and Close collapse it and leave the draft on the card.

use gpui_kit::{
    Context, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px, rgb,
};

use crate::trends_desk::TrendsDesk;
use termy_x::trends_desk::{
    TakeTone, apply_variant, char_count, panel_tone_name, panel_tones, wrap_token,
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
        let drafting = pop.draft == termy_x::trends_desk::DRAFTING_PLACEHOLDER;
        let field = if drafting {
            termy_x::trends_desk::DRAFTING_PLACEHOLDER.to_string()
        } else {
            format!("{head}|{tail}")
        };
        let dial = format!("Tone · {}", panel_tone_name(TakeTone::parse(&pop.tone)));
        let mut box_el = div()
            .id("desk-inline-editor")
            .w_full()
            .p(px(8.0))
            .rounded(px(10.0))
            .bg(rgb(0x2a0f1a))
            .border_1()
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
                    .child(field),
            )
            .child({
                let mut tones = div().flex().gap(px(8.0)).flex_wrap();
                for tone in panel_tones() {
                    tones = tones.child(tone_button(
                        &format!("desk-tone-{}", panel_tone_name(tone)),
                        panel_tone_name(tone),
                        &pop.tone,
                        tone,
                        cx,
                    ));
                }
                tones.child(pop_button("desk-tone-dial", &dial, cx, |this, cx| {
                    let current = this
                        .popout
                        .as_ref()
                        .map(|pop| pop.tone.clone())
                        .unwrap_or_default();
                    let next = termy_x::trends_desk::next_tone(&current);
                    this.set_pop_tone(panel_tone_name(next));
                    this.regenerate_open_variants(cx);
                }))
            });
        for (index, variant) in pop.variants.iter().enumerate() {
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
                                let variants = pop.variants.clone();
                                if !apply_variant(&mut pop.draft, &variants, index) {
                                    cx.stop_propagation();
                                    return;
                                }
                                let text = pop.draft.clone();
                                let id = pop.card_id.clone();
                                this.pop_caret =
                                    termy_x::draft_edit::DraftCaret::at_end(text.len());
                                this.card_replies
                                    .insert(id, termy_x::trends_desk::ReplyPhase::Editing { text });
                            }
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            );
        }
        Some(
            box_el
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
                        .id("desk-provider-label")
                        .flex_shrink_0()
                        .text_size(px(12.0))
                        .child(pop.provider.caption()),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
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
                        .child(pop_button("desk-open-grok", "Open Grok", cx, |this, cx| {
                            this.open_agent(termy_x::agents::Agent::Grok, cx);
                        })),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(8.0))
                        .child(pop_button("desk-inline-close", "Close", cx, |this, cx| {
                            this.close_popout();
                            cx.notify();
                        }))
                        .child(pop_button("desk-pop-send", "Send", cx, |this, cx| {
                            this.send_from_popout(cx);
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
            cx.notify();
        }
    }
}

fn tone_button(
    id: &str,
    label: &str,
    current: &str,
    tone: TakeTone,
    cx: &mut Context<TrendsDesk>,
) -> gpui_kit::AnyElement {
    let active = TakeTone::parse(current) == tone;
    let name = panel_tone_name(tone).to_string();
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
