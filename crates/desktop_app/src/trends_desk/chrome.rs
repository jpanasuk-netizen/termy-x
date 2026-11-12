//! Header, navigation, and settings for the Termy XL window.

use gpui_kit::{
    App, AppContext, Context, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window, div, px, rgb,
};

use super::{Field, Surface, TrendsDesk};
use termy_x::trends_desk::{
    GPU_NOTE, IMAGE_QUOTE_NOTE, Nav, TakeProvider, VISIBLE_NAME, turn_page,
};

impl TrendsDesk {
    pub(crate) fn post_saved(&mut self, id: String, cx: &mut Context<Self>) {
        let rows =
            termy_x::drafts::load_drafts(&termy_x::trends_desk::drafts_file(&self.data_root));
        let Some(row) = rows.into_iter().find(|row| row.id == id) else {
            self.action_note = "That draft is not in the list.".into();
            cx.notify();
            return;
        };
        let url = row.reply_to.unwrap_or_default();
        let text = termy_x::trends_desk::text_to_post(
            if self.list_pick.as_deref() == Some(&format!("draft:{}", row.id))
                && !self.draft_text.trim().is_empty()
            {
                &self.draft_text
            } else {
                &row.text
            },
        );
        if !url.contains("/status/") {
            self.draft_errors.insert(
                id,
                "That draft has no status URL. Nothing was posted.".into(),
            );
            self.action_note = "That draft has no status URL. Nothing was posted.".into();
            cx.notify();
            return;
        }
        if self.draft_busy.contains(&id) {
            self.action_note = "Posting…".into();
            cx.notify();
            return;
        }
        self.draft_busy.insert(id.clone());
        self.draft_errors.remove(&id);
        self.action_note = "Posting…".into();
        let bin = self.config.opencli_bin.clone();
        let posted_url = url.clone();
        let posted_text = text.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    termy_x::trends_desk::post_reply_opencli(
                        &bin,
                        &termy_x::io::SystemRunner,
                        &posted_url,
                        &posted_text,
                    )
                })
                .await;
            let _ = this.update(cx, |desk, cx| {
                desk.draft_busy.remove(&id);
                match result {
                    Ok(receipt) => {
                        let parent = termy_x::trends_desk::status_id(&url).unwrap_or_default();
                        let mut mark = termy_x::trends_desk::reply_receipt(&parent, &url, &receipt);
                        mark.at_unix = TrendsDesk::now_unix();
                        mark.status = "confirmed".into();
                        mark.text = text.clone();
                        desk.pack.receipts.push(mark);
                        desk.save_pack_file();
                        if !parent.is_empty() {
                            desk.card_replies.insert(
                                parent,
                                termy_x::trends_desk::ReplyPhase::Confirmed {
                                    text: text.clone(),
                                    receipt: receipt.clone(),
                                },
                            );
                        }
                        desk.action_note = termy_x::trends_desk::reply_link(&receipt)
                            .map(|link| format!("Posted ✓ {link}"))
                            .unwrap_or_else(|| "Posted ✓".into());
                    }
                    Err(error) => {
                        desk.draft_errors.insert(id, error.clone());
                        desk.action_note = error;
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

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
                    .children((!self.action_note.is_empty()).then(|| {
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0xffd60a))
                            .whitespace_normal()
                            .child(self.action_note.clone())
                    }))
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
                        if self.busy {
                            "Refreshing…"
                        } else {
                            "Refresh"
                        },
                        self.busy,
                        cx.listener(|this, _, _, cx| {
                            if this.busy {
                                this.action_note = "Already loading.".into();
                                cx.notify();
                                return;
                            }
                            this.pack.last_refresh = Some(TrendsDesk::now_unix());
                            this.save_pack_file();
                            let id = this.settings.subject_id.clone();
                            this.action_note = "Refreshing this page…".into();
                            this.start_run(std::slice::from_ref(&id), cx);
                        }),
                    ))
                    .child(button(
                        "desk-run-all",
                        if self.busy { "Running…" } else { "Run all" },
                        self.busy,
                        cx.listener(|this, _, _, cx| {
                            if this.busy {
                                this.action_note = "Already loading.".into();
                                cx.notify();
                                return;
                            }
                            let ids: Vec<String> = this
                                .columns
                                .iter()
                                .map(|column| column.topic.id.clone())
                                .collect();
                            this.action_note = "Running every subject…".into();
                            this.start_run(&ids, cx);
                        }),
                    ))
                    .child(button(
                        "desk-open-grok",
                        "Open Grok",
                        false,
                        cx.listener(|this, _, _, cx| {
                            this.action_note = "Opening Grok in a terminal…".into();
                            this.open_agent(termy_x::agents::Agent::Grok, cx);
                            cx.notify();
                        }),
                    ))
                    .child(button(
                        "desk-cancel",
                        "Cancel",
                        false,
                        cx.listener(|this, _, _, cx| {
                            if !this.busy {
                                this.action_note = "Nothing is running.".into();
                                cx.notify();
                                return;
                            }
                            this.cancel_run(cx);
                        }),
                    ))
                    .child(div().text_size(px(13.0)).whitespace_normal().child(
                        termy_x::trends_desk::provider_label(&self.settings.take_provider),
                    )),
            )
            .into_any_element()
    }

    pub(crate) fn nav_bar(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let current = self.surface;
        div()
            .flex()
            .flex_wrap()
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
                format!("Drafts ({})", self.draft_count),
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
            .child(self.search_box(cx))
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

    pub(crate) fn history_view(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let all = termy_x::trends_desk::copy_all(
            &self
                .history
                .iter()
                .map(|report| report.markdown.clone())
                .collect::<Vec<_>>(),
        );
        let mut list = div().flex().flex_col().gap(px(8.0)).child(button(
            "desk-history-copy-all",
            "Copy all",
            false,
            cx.listener(move |this, _, _, cx| {
                this.copy_text(&all, cx);
                this.action_note = termy_x::trends_desk::copied_mark().into();
                cx.notify();
            }),
        ));
        if self.history.is_empty() {
            list = list.child("No saved reports yet. A failed run does not write one.");
        }
        for (index, report) in self.history.iter().enumerate() {
            let picked = self.list_pick.as_deref() == Some(&format!("history:{index}"));
            let markdown = report.markdown.clone();
            let copy_text = markdown.clone();
            list = list.child(
                div()
                    .id(SharedString::from(format!("desk-history-{index}")))
                    .p(px(10.0))
                    .rounded(px(10.0))
                    .bg(rgb(if picked { 0x2a1020 } else { 0x16161e }))
                    .border_1()
                    .border_color(rgb(if picked { 0xff4fa3 } else { 0x3a3a3c }))
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.list_pick = Some(format!("history:{index}"));
                            this.action_note = "Selected a history entry. Ctrl+C copies it.".into();
                            cx.notify();
                        }),
                    )
                    .child(format!(
                        "{} · {} · cached file {}",
                        report.topic_id, report.query, report.file
                    ))
                    .child(
                        div()
                            .whitespace_normal()
                            .text_size(px(13.0))
                            .child(markdown),
                    )
                    .child(button(
                        SharedString::from(format!("desk-history-copy-{index}")),
                        "Copy",
                        false,
                        cx.listener(move |this, _, _, cx| {
                            this.copy_text(&copy_text, cx);
                            this.list_pick = Some(format!("history:{index}"));
                            this.action_note = termy_x::trends_desk::copied_mark().into();
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )),
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
        let rows =
            termy_x::drafts::load_drafts(&termy_x::trends_desk::drafts_file(&self.data_root));
        let all = termy_x::trends_desk::copy_all(
            &rows
                .iter()
                .map(|row| {
                    termy_x::trends_desk::copy_entry(
                        &row.text,
                        row.reply_to.as_deref().unwrap_or(""),
                    )
                })
                .collect::<Vec<_>>(),
        );
        let mut list = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(self.draft_note.clone())
            .child(
                div()
                    .text_size(px(13.0))
                    .child(termy_x::trends_desk::saved_drafts_label(self.draft_count)),
            )
            .child(button(
                "desk-drafts-copy-all",
                "Copy all",
                false,
                cx.listener(move |this, _, _, cx| {
                    this.copy_text(&all, cx);
                    this.action_note = termy_x::trends_desk::copied_mark().into();
                    cx.notify();
                }),
            ))
            .child(self.field_chip(
                "desk-draft-text",
                "Edit selected draft",
                &self.draft_text,
                Field::Draft,
                cx,
            ))
            .child(button(
                "desk-draft-save-edit",
                "Save edit",
                false,
                cx.listener(|this, _, _, cx| {
                    this.save_picked_draft();
                    cx.notify();
                }),
            ));
        if rows.is_empty() {
            list = list.child("No drafts yet. Draft take saves one here.");
        }
        for row in rows.iter().rev() {
            let id = row.id.clone();
            let tweet = row.tone.clone().unwrap_or_else(|| "Original tweet".into());
            let url = row.reply_to.clone().unwrap_or_default();
            let text = row.text.clone();
            let picked = self.list_pick.as_deref() == Some(&format!("draft:{id}"));
            let shown = if picked && !self.draft_text.is_empty() {
                self.draft_text.clone()
            } else {
                text.clone()
            };
            let copy_id = id.clone();
            let delete_id = id.clone();
            let select_id = id.clone();
            let select_text = text.clone();
            let select_url = url.clone();
            list = list.child(
                div()
                    .id(SharedString::from(format!("desk-draft-row-{id}")))
                    .p(px(10.0))
                    .rounded(px(10.0))
                    .bg(rgb(if picked { 0x2a1020 } else { 0x16161e }))
                    .border_1()
                    .border_color(rgb(if picked { 0xff4fa3 } else { 0x3a3a3c }))
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.list_pick = Some(format!("draft:{select_id}"));
                            this.draft_text = select_text.clone();
                            this.reply_edit =
                                Some(termy_x::draft_edit::ReplyEditor::new(&select_text));
                            this.draft_reply_to = Some(select_url.clone());
                            this.action_note = "Selected a draft. Ctrl+C copies it.".into();
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0xc7c7cc))
                            .whitespace_normal()
                            .child(format!("Original · {tweet}")),
                    )
                    .child(div().text_size(px(14.0)).whitespace_normal().child(shown))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .whitespace_normal()
                            .text_color(rgb(0x7af6ff))
                            .child(url.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(8.0))
                            .child(button(
                                SharedString::from(format!("desk-draft-copy-{copy_id}")),
                                "Copy",
                                false,
                                cx.listener({
                                    let text = text.clone();
                                    let url = url.clone();
                                    move |this, _, _, cx| {
                                        let payload = termy_x::trends_desk::copy_entry(&text, &url);
                                        this.copy_text(&payload, cx);
                                        this.action_note =
                                            termy_x::trends_desk::copied_mark().into();
                                        cx.stop_propagation();
                                        cx.notify();
                                    }
                                }),
                            ))
                            .child(draft_post_control(&id, &url, self, cx))
                            .child(button(
                                SharedString::from(format!("desk-draft-delete-{delete_id}")),
                                "Delete",
                                false,
                                cx.listener(move |this, _, _, cx| {
                                    this.delete_saved(&delete_id);
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            )),
                    ),
            );
        }
        list.into_any_element()
    }

    pub(crate) fn reader_bar(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let Some(reader) = self.reader.clone() else {
            return div().into_any_element();
        };
        let body = reader.body.clone();
        let url = reader.url.clone();
        div()
            .id("desk-reader")
            .mb(px(8.0))
            .p(px(10.0))
            .rounded(px(10.0))
            .bg(rgb(0x16161e))
            .border_1()
            .border_color(rgb(0x7af6ff))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(reader.title.clone())
            .child(div().whitespace_normal().text_size(px(14.0)).child(body))
            .child(
                div()
                    .whitespace_normal()
                    .text_size(px(12.0))
                    .text_color(rgb(0x7af6ff))
                    .child(url.clone()),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(button(
                        "desk-reader-copy",
                        "Copy",
                        false,
                        cx.listener(move |this, _, _, cx| {
                            this.copy_text(
                                &termy_x::trends_desk::copy_entry(&reader.body, &reader.url),
                                cx,
                            );
                            this.action_note = termy_x::trends_desk::copied_mark().into();
                            cx.notify();
                        }),
                    ))
                    .child(button(
                        "desk-reader-close",
                        "Close",
                        false,
                        cx.listener(|this, _, _, cx| {
                            this.reader = None;
                            this.action_note = "Closed the inline view.".into();
                            cx.notify();
                        }),
                    )),
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
                    .whitespace_normal()
                    .child(format!(
                        "Takes start at Vyce · glm-5.3-flash. A saved model URL overrides it. Now: {model} @ {base}"
                    ))
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
            .child(button(
                "desk-auto-fallback",
                if self.settings.auto_fallback {
                    "Auto-fallback: On"
                } else {
                    "Auto-fallback: Off"
                },
                self.settings.auto_fallback,
                cx.listener(|this, _, _, cx| {
                    this.settings.auto_fallback = !this.settings.auto_fallback;
                    if this.writable {
                        let _ = this.store.save_settings(&this.settings);
                    }
                    this.action_note = if this.settings.auto_fallback {
                        "Auto-fallback is on. Grok still does not switch by itself.".into()
                    } else {
                        "Auto-fallback is off.".into()
                    };
                    cx.notify();
                }),
            ))
            .child(
                div()
                    .text_size(px(13.0))
                    .whitespace_normal()
                    .child("Default take provider"),
            )
            .child(self.provider_dropdown(cx))
            .child(self.field_chip(
                "desk-custom-base",
                "Custom base URL",
                &self.settings.custom_base,
                Field::CustomBase,
                cx,
            ))
            .child(self.field_chip(
                "desk-custom-model",
                "Custom model",
                &self.settings.custom_model,
                Field::CustomModel,
                cx,
            ))
            .child(self.field_chip(
                "desk-custom-key-env",
                "Custom key env name",
                &self.settings.custom_key_env,
                Field::CustomKeyEnv,
                cx,
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

    fn search_box(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let editing = self.field == Some(Field::Filter);
        div()
            .id("desk-filter")
            .track_focus(&self.search_focus)
            .px(px(10.0))
            .py(px(6.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(if editing {
                rgb(0x0a84ff)
            } else {
                rgb(0x3a3a3c)
            })
            .bg(if editing {
                rgb(0x1a2a44)
            } else {
                rgb(0x2c2c2e)
            })
            .text_size(px(12.0))
            .min_w(px(160.0))
            .min_h(px(36.0))
            .flex_shrink_0()
            .cursor_text()
            .hover(|style| style.border_color(rgb(0xff4fa3)))
            .child(termy_x::trends_desk::search_label(
                editing,
                &self.filter_text,
            ))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.field = Some(Field::Filter);
                    this.search_focus.focus(window, cx);
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                this.on_search_key(event, cx);
            }))
            .into_any_element()
    }

    pub(crate) fn on_search_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        self.field = Some(Field::Filter);
        let key = event.keystroke.key.as_str();
        let ch = event.keystroke.key_char.as_deref();
        match termy_x::trends_desk::search_edit(&self.filter_text, key, ch) {
            termy_x::trends_desk::SearchEdit::Submit(term) => {
                self.filter_text = term;
                self.run_live_search(cx);
                self.field = None;
            }
            termy_x::trends_desk::SearchEdit::Close => self.field = None,
            termy_x::trends_desk::SearchEdit::Text(text) => self.filter_text = text,
            termy_x::trends_desk::SearchEdit::Ignore => return,
        }
        cx.stop_propagation();
        cx.notify();
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
    label: impl Into<SharedString> + 'static,
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
    let label = label.into();
    let tip = label.clone();
    div()
        .id(id.into())
        .flex_shrink_0()
        .min_h(px(36.0))
        .px(px(12.0))
        .py(px(8.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(if active { rgb(0xff4fa3) } else { rgb(0x3a3a3c) })
        .bg(if active { rgb(0x2a1020) } else { rgb(0x2c2c2e) })
        .text_size(px(13.0))
        .text_color(if active { rgb(0x7af6ff) } else { rgb(0xe5e5ea) })
        .cursor_pointer()
        .hover(|style| {
            style
                .bg(rgb(0x3a2430))
                .border_color(rgb(0xff4fa3))
                .text_color(rgb(0xffe56a))
        })
        .active(|style| {
            style
                .bg(rgb(0xff4fa3))
                .border_color(rgb(0xff4fa3))
                .text_color(rgb(0x070b14))
        })
        .tooltip(move |_, cx| cx.new(|_| HintView { text: tip.clone() }).into())
        .child(label)
        .on_mouse_down(MouseButton::Left, handler)
        .into_any_element()
}

fn parent_author(desk: &TrendsDesk, url: &str) -> String {
    if let Some(name) = desk.pack.authors.get(url) {
        let name = name.trim().trim_start_matches('@');
        if !name.is_empty() {
            return name.to_string();
        }
    }
    let wanted = termy_x::trends_desk::status_id(url);
    for column in &desk.columns {
        for post in &column.view.posts {
            let same = post.url == url
                || wanted
                    .as_ref()
                    .is_some_and(|id| post.id == *id || post.url.contains(id));
            if !same {
                continue;
            }
            let name = post.author.trim().trim_start_matches('@');
            if !name.is_empty() {
                return name.to_string();
            }
        }
    }
    String::new()
}

fn draft_post_control(
    id: &str,
    url: &str,
    desk: &TrendsDesk,
    cx: &mut Context<TrendsDesk>,
) -> gpui_kit::AnyElement {
    if let Some(mark) = desk.pack.receipts.iter().rev().find(|mark| {
        mark.status == "confirmed" && (mark.parent_url == url || mark.reply_url == url)
    }) {
        let when = termy_x::trends_desk::format_clock(
            mark.at_unix,
            termy_x::trends_desk::local_offset_secs(mark.at_unix),
        );
        let link = if mark.reply_url.is_empty() {
            mark.parent_url.clone()
        } else {
            mark.reply_url.clone()
        };
        return div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .text_size(px(28.0))
                    .text_color(rgb(0xffe14a))
                    .child("✓"),
            )
            .child(format!("Posted ✓ {when}"))
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(rgb(0x7af6ff))
                    .whitespace_normal()
                    .child(link),
            )
            .into_any_element();
    }
    let busy = desk.draft_busy.contains(id);
    let error = desk.draft_errors.get(id).cloned();
    let label = if busy {
        "Posting…".to_string()
    } else if error.is_some() {
        "Retry".to_string()
    } else {
        termy_x::trends_desk::draft_reply_label(url, &parent_author(desk, url))
    };
    let post_id = id.to_string();
    let mut column = div().flex().flex_col().gap(px(6.0));
    if let Some(error) = error {
        column = column.child(
            div()
                .text_color(rgb(0xff6b6b))
                .whitespace_normal()
                .child(error),
        );
    }
    column
        .child(
            div()
                .id(SharedString::from(format!("desk-draft-post-{post_id}")))
                .w_full()
                .min_h(px(56.0))
                .px(px(16.0))
                .py(px(12.0))
                .rounded(px(10.0))
                .flex()
                .items_center()
                .justify_center()
                .bg(rgb(0x7af6ff))
                .text_size(px(16.0))
                .text_color(rgb(0x070b14))
                .cursor_pointer()
                .child(label)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        if !busy {
                            this.post_saved(post_id.clone(), cx);
                        }
                        cx.stop_propagation();
                        cx.notify();
                    }),
                ),
        )
        .into_any_element()
}

/// Prev and Next. Tall enough to hit, and they do not shrink when the title is long.
pub(crate) fn page_button(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    handler: impl Fn(&gpui_kit::MouseDownEvent, &mut Window, &mut App) + 'static,
) -> gpui_kit::AnyElement {
    let label = label.into();
    let tip = label.clone();
    let hit = termy_x::trends_desk::PAGE_BAR_HIT;
    div()
        .id(id.into())
        .flex_shrink_0()
        .min_w(px(72.0))
        .h(px(hit))
        .px(px(16.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(8.0))
        .border_1()
        .border_color(rgb(0x3a3a3c))
        .bg(rgb(0x2c2c2e))
        .text_size(px(14.0))
        .text_color(rgb(0xe5e5ea))
        .cursor_pointer()
        .hover(|style| {
            style
                .bg(rgb(0x3a2430))
                .border_color(rgb(0xff4fa3))
                .text_color(rgb(0xffe56a))
        })
        .active(|style| style.bg(rgb(0xff4fa3)).text_color(rgb(0x070b14)))
        .tooltip(move |_, cx| cx.new(|_| HintView { text: tip.clone() }).into())
        .child(label)
        .on_mouse_down(MouseButton::Left, handler)
        .into_any_element()
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

pub(crate) fn follow_loop(cx: &mut Context<super::TrendsDesk>) {
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(250))
                .await;
            let _ = this.update(cx, |_desk, cx| cx.notify());
        }
    })
    .detach();
}
