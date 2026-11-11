//! Gold take cards for a Termy XL subject page.

use gpui_kit::{
    Context, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px, rgb,
};

use crate::trends_desk::{ColumnUi, TrendsDesk, button, open_status};
use termy_x::trends_desk::{ChangeTag, ReplyPhase, card_from_post, scroll_by, wrap_token};

impl TrendsDesk {
    pub(crate) fn tweet_card(
        &self,
        post: &termy_x::trends_desk::SourcePost,
        column: &ColumnUi,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let card = card_from_post(post);
        let phase = self
            .card_replies
            .get(&card.id)
            .cloned()
            .unwrap_or(ReplyPhase::Idle);
        let tag = column
            .tags
            .iter()
            .find(|(url, _)| url == &card.url)
            .map(|(_, tag)| match tag {
                ChangeTag::New => "NEW",
                ChangeTag::Rising => "RISING",
                ChangeTag::Fading => "FADING",
            });
        let url = card.url.clone();
        let id = card.id.clone();
        let take_id = id.clone();
        let edit_id = id.clone();
        let send_id = id.clone();
        let created = card
            .created_at
            .as_deref()
            .map(|raw| {
                termy_x::trends_desk::format_post_time(
                    raw,
                    crate::trends_desk::TrendsDesk::now_unix(),
                    termy_x::trends_desk::local_offset_secs(
                        crate::trends_desk::TrendsDesk::now_unix(),
                    ),
                )
            })
            .unwrap_or_else(|| "time not provided".into());
        let media = if card.media.is_empty() {
            "no media".to_string()
        } else {
            card.media.join(" ")
        };
        let reply_text = wrap_token(phase.text());
        let shown = self.takes.get(&card.id);
        let (take_line, take_tag) = match shown {
            Some(take) if take.source == "pending" => ("Drafting take…".to_string(), None),
            Some(take) if take.source == "offline" => (
                take.text.clone(),
                Some(if take.note.starts_with("free model") {
                    take.note.clone()
                } else {
                    "offline take".into()
                }),
            ),
            Some(take) => (take.text.clone(), None),
            None => (
                termy_x::trends_desk::idea_from(post),
                Some("offline take".to_string()),
            ),
        };
        let editing = self
            .popout
            .as_ref()
            .is_some_and(|pop| pop.open && pop.card_id == card.id);
        let take_key = format!("take:{}", card.id);
        let selected = self.selected.as_deref() == Some(take_key.as_str());
        let handle = self
            .scroll_handles
            .get(&take_key)
            .cloned()
            .unwrap_or_default();
        let status = if editing {
            "Editing reply".to_string()
        } else {
            match &phase {
                ReplyPhase::Pending { .. } => "Pending send".to_string(),
                ReplyPhase::Confirmed { receipt, .. } => format!("Confirmed · {receipt}"),
                ReplyPhase::Failed { error, .. } => format!("Failed · {error}"),
                ReplyPhase::Editing { .. } | ReplyPhase::Idle => String::new(),
            }
        };
        div()
            .id(SharedString::from(format!("desk-card-{id}")))
            .w_full()
            .min_w(px(0.0))
            .max_h(px(280.0))
            .p(px(10.0))
            .rounded(px(10.0))
            .overflow_y_scroll()
            .scrollbar_width(px(4.0))
            .track_scroll(&handle)
            .bg(if selected {
                rgb(0x2a2410)
            } else {
                rgb(0x10141c)
            })
            .border_1()
            .border_color(if selected {
                rgb(0xffe56a)
            } else {
                rgb(0xffd60a)
            })
            .flex()
            .flex_col()
            .gap(px(4.0))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener({
                    let take_key = take_key.clone();
                    move |this, _, _, cx| {
                        this.selected = Some(take_key.clone());
                        cx.stop_propagation();
                        cx.notify();
                    }
                }),
            )
            .on_scroll_wheel(cx.listener({
                let take_key = take_key.clone();
                move |this, event: &gpui_kit::ScrollWheelEvent, _, cx| {
                    let delta = f32::from(event.delta.pixel_delta(px(16.0)).y);
                    let handle = this.scroll_handles.get(&take_key).cloned();
                    let max = handle
                        .as_ref()
                        .map(|item| f32::from(item.max_offset().y))
                        .unwrap_or(400.0);
                    let at = handle
                        .as_ref()
                        .map(|item| f32::from(item.offset().y))
                        .unwrap_or(0.0);
                    scroll_by(
                        &mut this.scroll_offsets,
                        &take_key,
                        delta.abs(),
                        max.max(at),
                    );
                    let at_limit = (delta > 0.0 && at <= 1.0) || (delta < 0.0 && at >= max - 1.0);
                    if !at_limit {
                        cx.stop_propagation();
                    }
                }
            }))
            .child(
                div()
                    .text_size(px(14.0))
                    .whitespace_normal()
                    .text_color(rgb(0xffe56a))
                    .child(take_line),
            )
            .children(take_tag.map(|tag| {
                div()
                    .text_size(px(11.0))
                    .text_color(rgb(0x8e8e93))
                    .child(tag)
            }))
            .child(div().text_size(px(14.0)).whitespace_normal().child(format!(
                "{}{} · @{}",
                tag.map(|tag| format!("{tag} · ")).unwrap_or_default(),
                wrap_token(&card.author_name),
                wrap_token(&card.handle)
            )))
            .child(
                div()
                    .text_size(px(13.0))
                    .whitespace_normal()
                    .child(wrap_token(&card.text)),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(rgb(0xc7c7cc))
                    .child(format!(
                        "{created} · {} likes · {} reposts · {} replies · {} views",
                        card.likes, card.reposts, card.replies, card.views
                    )),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .whitespace_normal()
                    .text_color(rgb(0x8e8e93))
                    .child(wrap_token(&media)),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .whitespace_normal()
                    .text_color(rgb(0x7af6ff))
                    .child(wrap_token(&card.url)),
            )
            .children(self.heat_for(post).map(|heat| {
                div()
                    .text_size(px(12.0))
                    .text_color(rgb(0xffd60a))
                    .child(heat)
            }))
            .children((!status.is_empty()).then(|| div().text_size(px(12.0)).child(status)))
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(rgb(0x30d158))
                    .child(self.badge_for(&card.id).unwrap_or_default()),
            )
            .children((!reply_text.trim().is_empty() && editing).then(|| {
                div()
                    .text_size(px(12.0))
                    .whitespace_normal()
                    .child(reply_text)
            }))
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(button(
                        SharedString::from(format!("desk-open-{id}")),
                        "Open source",
                        false,
                        cx.listener(move |_, _, _, _| open_status(&url)),
                    ))
                    .child(button(
                        SharedString::from(format!("desk-take-{id}")),
                        "Draft take",
                        false,
                        cx.listener(move |this, _, _, cx| {
                            this.draft_take_for(take_id.clone(), cx);
                            cx.stop_propagation();
                        }),
                    ))
                    .child(button(
                        SharedString::from(format!("desk-reply-{id}")),
                        "Reply",
                        false,
                        cx.listener(move |this, _, _, cx| {
                            this.open_popout(&edit_id);
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ))
                    .child(button(
                        SharedString::from(format!("desk-star-{id}")),
                        "Star",
                        false,
                        cx.listener({
                            let star_id = id.clone();
                            move |this, _, _, cx| {
                                this.toggle_star(&star_id);
                                cx.stop_propagation();
                                cx.notify();
                            }
                        }),
                    ))
                    .child(button(
                        SharedString::from(format!("desk-copy-link-{id}")),
                        "Copy link",
                        false,
                        cx.listener({
                            let link = card.url.clone();
                            move |this, _, _, cx| {
                                this.copy_text(&link, cx);
                                cx.stop_propagation();
                            }
                        }),
                    ))
                    .child(button(
                        SharedString::from(format!("desk-copy-text-{id}")),
                        "Copy text",
                        false,
                        cx.listener({
                            let body = card.text.clone();
                            move |this, _, _, cx| {
                                this.copy_text(&body, cx);
                                cx.stop_propagation();
                            }
                        }),
                    ))
                    .child(button(
                        SharedString::from(format!("desk-edit-{id}")),
                        "Edit",
                        false,
                        cx.listener(move |this, _, _, cx| {
                            this.open_popout(&send_id);
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )),
            )
            .into_any_element()
    }
}
