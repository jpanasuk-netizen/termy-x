//! Header, navigation, and settings for the Termy XL window.

use gpui_kit::{
    App, AppContext, Context, InteractiveElement, IntoElement, MouseButton, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px, rgb,
};

use super::{Field, Surface, TrendsDesk};
use termy_x::trends_desk::{
    GPU_NOTE, IMAGE_QUOTE_NOTE, Nav, TakeProvider, VISIBLE_NAME, turn_page,
};

impl TrendsDesk {
    pub(crate) fn header(&self, _per_row: usize, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(12.0))
            .px(px(16.0))
            .py(px(12.0))
            .bg(rgb(0x10141c))
            .border_b_1()
            .border_color(rgb(0x2c2c2e))
            .child(self.xl_mark(cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        div()
                            .text_size(px(22.0))
                            .text_color(rgb(0x7af6ff))
                            .child(VISIBLE_NAME),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0xc7c7cc))
                            .child("Trends Desk"),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0x8e8e93))
                            .child(self.session.label.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(0x8e8e93))
                            .child(self.hud()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(0x9ad7ff))
                            .child(self.hermes_pill.clone()),
                    )
                    .children((!self.model_notice.is_empty()).then(|| {
                        let notice = self.model_notice.clone();
                        let full = self.model_detail.clone();
                        div()
                            .id("desk-model-notice")
                            .text_size(px(11.0))
                            .text_color(rgb(0xffd60a))
                            .child(notice)
                            .tooltip({
                                let full = full.clone();
                                move |_, cx| {
                                    cx.new(|_| HintView {
                                        text: full.clone().into(),
                                    })
                                    .into()
                                }
                            })
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .flex_shrink_0()
                    .gap(px(8.0))
                    .child(button(
                        "desk-refresh",
                        "Refresh",
                        false,
                        cx.listener(|this, _, _, cx| {
                            this.pack.last_refresh = Some(TrendsDesk::now_unix());
                            this.save_pack_file();
                            let ids: Vec<String> = this
                                .columns
                                .iter()
                                .map(|column| column.topic.id.clone())
                                .collect();
                            this.start_run(&ids, cx);
                        }),
                    ))
                    .child(button(
                        "desk-run-all",
                        "Run all",
                        false,
                        cx.listener(|this, _, _, cx| {
                            let ids: Vec<String> = this
                                .columns
                                .iter()
                                .map(|column| column.topic.id.clone())
                                .collect();
                            this.start_run(&ids, cx);
                        }),
                    ))
                    .child(button(
                        "desk-cancel",
                        "Cancel",
                        false,
                        cx.listener(|this, _, _, cx| this.cancel_run(cx)),
                    ))
                    .child(button(
                        "desk-take-free",
                        "Free takes",
                        TakeProvider::parse(&self.settings.take_provider) == TakeProvider::Free,
                        cx.listener(|this, _, _, cx| {
                            this.settings.take_provider = "free".into();
                            if this.writable {
                                let _ = this.store.save_settings(&this.settings);
                            }
                            this.begin_take_stream(cx);
                            cx.notify();
                        }),
                    ))
                    .child(button(
                        "desk-take-treg",
                        "Treg takes",
                        TakeProvider::parse(&self.settings.take_provider) == TakeProvider::Treg,
                        cx.listener(|this, _, _, cx| {
                            this.settings.take_provider = "treg".into();
                            if this.writable {
                                let _ = this.store.save_settings(&this.settings);
                            }
                            this.begin_take_stream(cx);
                            cx.notify();
                        }),
                    )),
            )
            .into_any_element()
    }

    pub(crate) fn nav_bar(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let current = self.surface;
        div()
            .flex()
            .gap(px(8.0))
            .px(px(16.0))
            .py(px(8.0))
            .child(nav_button(
                "desk-nav-columns",
                "Columns",
                current == Surface::Columns,
                cx,
                Nav::Columns,
            ))
            .child(nav_button(
                "desk-nav-history",
                "History",
                current == Surface::History,
                cx,
                Nav::History,
            ))
            .child(nav_button(
                "desk-nav-replies",
                "Replies",
                current == Surface::Replies,
                cx,
                Nav::Replies,
            ))
            .child(nav_button(
                "desk-nav-draft",
                "Draft",
                current == Surface::Draft,
                cx,
                Nav::Draft,
            ))
            .child(nav_button(
                "desk-nav-settings",
                "Settings",
                current == Surface::Settings,
                cx,
                Nav::Settings,
            ))
            .child(self.field_chip(
                "desk-filter",
                "Search",
                &self.filter_text,
                Field::Filter,
                cx,
            ))
            .into_any_element()
    }

    pub(crate) fn ensure_scrolls(&mut self) {
        for column in &self.columns {
            for post in &column.view.posts {
                self.scroll_handles
                    .entry(format!("take:{}", post.id))
                    .or_default();
            }
        }
    }

    pub(crate) fn turn_subject(&mut self, delta: i32) {
        let count = self.columns.len();
        let index = self
            .columns
            .iter()
            .position(|column| column.topic.id == self.settings.subject_id)
            .unwrap_or(0);
        let next = turn_page(index, count, delta);
        if let Some(column) = self.columns.get(next) {
            self.settings.subject_id = column.topic.id.clone();
            if self.writable {
                let _ = self.store.save_settings(&self.settings);
            }
        }
    }

    pub(crate) fn history_view(&self) -> gpui_kit::AnyElement {
        let mut list = div().flex().flex_col().gap(px(8.0));
        if self.history.is_empty() {
            list = list.child("No saved reports yet. A failed run does not write one.");
        }
        for report in &self.history {
            list = list.child(
                div()
                    .p(px(10.0))
                    .rounded(px(10.0))
                    .bg(rgb(0x16161e))
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(format!(
                        "{} · {} · cached file {}",
                        report.topic_id, report.query, report.file
                    ))
                    .child(report.markdown.clone()),
            );
        }
        list.into_any_element()
    }

    pub(crate) fn replies_view(&self, _cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let mut list = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(self.reply_note.clone());
        for post in &self.replies {
            let mut card = div()
                .p(px(10.0))
                .rounded(px(10.0))
                .bg(rgb(0x16161e))
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(format!(
                    "{} · {}",
                    post.author,
                    post.created_at.clone().unwrap_or_default()
                ))
                .child(post.text.clone())
                .child(post.url.clone());
            for reply in &post.replies {
                card = card.child(format!(
                    "reply {} · {} · {}",
                    reply.author, reply.url, reply.text
                ));
            }
            list = list.child(card);
        }
        list.into_any_element()
    }

    pub(crate) fn draft_view(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(self.draft_note.clone())
            .child(self.field_chip(
                "desk-draft-text",
                "Draft",
                &self.draft_text,
                Field::Draft,
                cx,
            ))
            .child(
                div().text_size(px(12.0)).text_color(rgb(0x8e8e93)).child(
                    self.draft_reply_to
                        .clone()
                        .unwrap_or_else(|| "No reply target yet.".into()),
                ),
            )
            .child(
                div().text_size(px(12.0)).child(
                    "Sending stays in the existing X compose confirm. This desk does not post.",
                ),
            )
            .into_any_element()
    }

    pub(crate) fn settings_view(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child({
                let (base, model) = self.config.ai_endpoint();
                div()
                    .text_size(px(12.0))
                    .child(format!("Free model: {model} @ {base}"))
            })
            .child(self.field_chip(
                "desk-ai-base",
                "Model URL",
                &self.ai_base_text,
                Field::AiBase,
                cx,
            ))
            .child(self.field_chip(
                "desk-ai-model",
                "Model name",
                &self.ai_model_text,
                Field::AiModel,
                cx,
            ))
            .child(self.field_chip(
                "desk-ai-key",
                "Model key",
                &self.ai_key_text,
                Field::AiKey,
                cx,
            ))
            .child(button(
                "desk-save-ai",
                "Save model",
                false,
                cx.listener(|this, _, _, cx| this.save_model(cx)),
            ))
            .child(div().text_size(px(12.0)).child(self.hermes_pill.clone()))
            .child(self.field_chip(
                "desk-hermes-base",
                "Hermes URL",
                &self.hermes_base_text,
                Field::HermesBase,
                cx,
            ))
            .child(self.field_chip(
                "desk-hermes-model",
                "Hermes model",
                &self.hermes_model_text,
                Field::HermesModel,
                cx,
            ))
            .child(self.field_chip(
                "desk-hermes-key",
                "Hermes key",
                &hermes_key_label(&self.hermes_key_text, self.field == Some(Field::HermesKey)),
                Field::HermesKey,
                cx,
            ))
            .child(button(
                "desk-save-hermes",
                "Save Hermes",
                false,
                cx.listener(|this, _, _, cx| this.save_hermes(cx)),
            ))
            .child(self.field_chip("desk-focus", "Focus", &self.focus_text, Field::Focus, cx))
            .child(self.field_chip(
                "desk-daily-at",
                "Daily at (America/Chicago)",
                &self.settings.daily_at,
                Field::DailyAt,
                cx,
            ))
            .child(self.field_chip(
                "desk-topic-name",
                "New column",
                &self.topic_name,
                Field::TopicName,
                cx,
            ))
            .child(self.field_chip(
                "desk-topic-desc",
                "Column description",
                &self.topic_desc,
                Field::TopicDesc,
                cx,
            ))
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(button(
                        "desk-save-settings",
                        "Save focus",
                        false,
                        cx.listener(|this, _, _, cx| this.save_settings(cx)),
                    ))
                    .child(button(
                        "desk-add-topic",
                        "Add column",
                        false,
                        cx.listener(|this, _, _, cx| this.add_topic(cx)),
                    ))
                    .child(button(
                        "desk-topic-earlier",
                        "Earlier",
                        false,
                        cx.listener(|this, _, _, cx| this.shift_subject(-1, cx)),
                    ))
                    .child(button(
                        "desk-topic-later",
                        "Later",
                        false,
                        cx.listener(|this, _, _, cx| this.shift_subject(1, cx)),
                    ))
                    .child(button(
                        "desk-topic-remove",
                        "Remove column",
                        false,
                        cx.listener(|this, _, _, cx| this.remove_subject(cx)),
                    ))
                    .child(button(
                        "desk-toggle-daily",
                        if self.settings.daily {
                            "Daily on"
                        } else {
                            "Daily off"
                        },
                        self.settings.daily,
                        cx.listener(|this, _, _, cx| {
                            this.settings.daily = !this.settings.daily;
                            let _ = this.store.save_settings(&this.settings);
                            cx.notify();
                        }),
                    )),
            )
            .child(self.session.detail.clone())
            .into_any_element()
    }

    fn field_chip(
        &self,
        id: &'static str,
        label: &str,
        value: &str,
        field: Field,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let active = self.field == Some(field);
        let shown = if value.is_empty() {
            format!("{label}: click to edit")
        } else {
            format!("{label}: {value}")
        };
        button(
            id,
            shown,
            active,
            cx.listener(move |this, _, window, cx| {
                this.field = Some(field);
                this.focus_handle.focus(window, cx);
                cx.notify();
            }),
        )
    }

    pub(crate) fn footer(&self) -> gpui_kit::AnyElement {
        div()
            .px(px(16.0))
            .py(px(8.0))
            .text_size(px(11.0))
            .text_color(rgb(0x8e8e93))
            .bg(rgb(0x10141c))
            .child(format!("{GPU_NOTE} {IMAGE_QUOTE_NOTE}"))
            .into_any_element()
    }
}

fn hermes_key_label(key: &str, editing: bool) -> String {
    if editing {
        return key.to_string();
    }
    if key.trim().is_empty() {
        String::new()
    } else {
        "saved".into()
    }
}

fn nav_button(
    id: &'static str,
    label: &'static str,
    active: bool,
    cx: &mut Context<TrendsDesk>,
    nav: Nav,
) -> gpui_kit::AnyElement {
    button(
        id,
        label,
        active,
        cx.listener(move |this, _, _, cx| this.nav(nav, cx)),
    )
}

pub(crate) fn button(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    active: bool,
    handler: impl Fn(&gpui_kit::MouseDownEvent, &mut Window, &mut App) + 'static,
) -> gpui_kit::AnyElement {
    div()
        .id(id.into())
        .px(px(10.0))
        .py(px(6.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(if active { rgb(0x0a84ff) } else { rgb(0x3a3a3c) })
        .bg(if active { rgb(0x1a2a44) } else { rgb(0x2c2c2e) })
        .text_size(px(12.0))
        .cursor_pointer()
        .child(label.into())
        .on_mouse_down(MouseButton::Left, handler)
        .into_any_element()
}

pub(crate) fn open_status(url: &str) {
    if url.starts_with("https://x.com/") && url.contains("/status/") {
        let _ = webbrowser::open(url);
    }
}

pub(crate) struct HintView {
    pub(crate) text: SharedString,
}

impl Render for HintView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .max_w(px(360.0))
            .p(px(8.0))
            .rounded(px(8.0))
            .bg(rgb(0x1c1c1e))
            .text_size(px(12.0))
            .text_color(rgb(0xe5e5ea))
            .child(self.text.clone())
    }
}
