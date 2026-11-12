//! Pink draft box inside a Termy XL gold card.
//!
//! Draft take opens it on the click. Funny, Smart, and Spicy swap the reply.
//! Copy copies it. Send is the only control that posts. Esc and Close keep the text.

use gpui_kit::{
    Animation, AnimationExt, AppContext, Context, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, ParentElement, SharedString, StatefulInteractiveElement,
    Styled, canvas, div, px, rgb,
};
use termy_x::io::Http;

use crate::trends_desk::TrendsDesk;
use termy_x::trends_desk::{
    ReplyPhase, TakeDraft, TakeProvider, TakeTone, apply_send_result, edit_reply, panel_tone_name,
    panel_tones, post_reply_opencli, request_send,
};

impl TrendsDesk {
    pub(crate) fn inline_editor(
        &self,
        card_id: &str,
        handle: &str,
        cx: &mut Context<Self>,
    ) -> Option<gpui_kit::AnyElement> {
        let pop = self.popout.clone()?;
        if !pop.open || pop.card_id != card_id {
            return None;
        }
        let drafting = pop.draft == termy_x::trends_desk::DRAFTING_PLACEHOLDER;
        let shown = self
            .reply_edit
            .as_ref()
            .map(|editor| editor.text.clone())
            .unwrap_or_else(|| pop.draft.clone());
        let caret = self
            .reply_edit
            .as_ref()
            .map(|editor| editor.caret)
            .unwrap_or(self.pop_caret);
        let phase = self.card_replies.get(&pop.card_id).cloned().unwrap_or(
            termy_x::trends_desk::ReplyPhase::Editing {
                text: pop.draft.clone(),
            },
        );
        let face = match &phase {
            termy_x::trends_desk::ReplyPhase::Pending { .. } => {
                termy_x::trends_desk::PostFace::Posting
            }
            termy_x::trends_desk::ReplyPhase::Confirmed { .. } => {
                termy_x::trends_desk::PostFace::Posted
            }
            termy_x::trends_desk::ReplyPhase::Failed { .. } => {
                termy_x::trends_desk::PostFace::Retry
            }
            _ => termy_x::trends_desk::PostFace::Ready,
        };
        let post_label = termy_x::trends_desk::post_button_text(face, handle);
        let posted_url = match &phase {
            termy_x::trends_desk::ReplyPhase::Confirmed { receipt, .. } => {
                termy_x::trends_desk::reply_link(receipt)
            }
            _ => None,
        };
        let failure = match &phase {
            termy_x::trends_desk::ReplyPhase::Failed { error, .. } => Some(error.clone()),
            _ => None,
        };
        let status = if let Some(error) = failure.clone() {
            error
        } else {
            self.action_note.clone()
        };
        let count = termy_x::trends_desk::char_count(&shown);
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
                    .id("desk-reply-status")
                    .text_size(px(13.0))
                    .text_color(rgb(0x070b14))
                    .whitespace_normal()
                    .child(if status.is_empty() {
                        "Status".into()
                    } else {
                        status.clone()
                    }),
            )
            .child(self.provider_dropdown(cx))
            .child(self.take_carousel(cx))
            .children(
                (self.take_generating || drafting)
                    .then(|| crate::xl_mark::drafting_mark(self.elapsed_secs())),
            )
            .children((!self.take_banner.is_empty()).then(|| self.failure_banner(cx)))
            .children((drafting || self.take_generating).then(|| self.cancel_row(cx)))
            .child(
                div()
                    .id("desk-reply-editor")
                    .relative()
                    .min_h(px(96.0))
                    .p(px(8.0))
                    .rounded(px(8.0))
                    .bg(rgb(0x2a1020))
                    .text_size(px(14.0))
                    .text_color(rgb(0xffe56a))
                    .whitespace_normal()
                    .child(
                        canvas(
                            |bounds, _, _| {
                                remember_editor_box(
                                    bounds.origin.x.into(),
                                    bounds.origin.y.into(),
                                    bounds.size.width.into(),
                                );
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
                    .child(if drafting {
                        crate::xl_mark::drafting_mark(self.elapsed_secs())
                    } else {
                        crate::xl_mark::reply_surface(&shown, &caret)
                    })
                    .children((!drafting).then(|| {
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(0xc7c7cc))
                            .child(format!("{count} characters"))
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, cx| {
                            this.place_reply_cursor(
                                event.position.x.into(),
                                event.position.y.into(),
                                event.modifiers.shift,
                            );
                            this.reply_drag = !event.modifiers.shift;
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                        if this.reply_drag && event.pressed_button == Some(MouseButton::Left) {
                            this.place_reply_cursor(
                                event.position.x.into(),
                                event.position.y.into(),
                                true,
                            );
                            cx.stop_propagation();
                            cx.notify();
                        }
                    })),
            );
        let mut chips = div().flex().flex_wrap().gap(px(8.0));
        for (index, tone) in panel_tones().into_iter().enumerate() {
            chips = chips.child(variant_chip(panel_tone_name(tone), index, cx));
        }
        box_el = box_el.child(chips);
        if status.contains("Vyce direct") {
            box_el = box_el.child(box_button(
                "desk-vyce-direct",
                "Vyce direct",
                cx,
                |this, cx| this.choose_provider("vyce", cx),
            ));
        }
        if status.contains("pick another provider") {
            box_el = box_el.child(box_button("desk-switch-grok", "Grok", cx, |this, cx| {
                this.choose_provider("grok", cx);
            }));
            box_el = box_el.child(box_button("desk-switch-muse", "Muse", cx, |this, cx| {
                this.choose_provider("muse", cx);
            }));
        }
        let copy_label = if self.action_note == termy_x::trends_desk::copied_mark() {
            termy_x::trends_desk::copied_mark()
        } else {
            "Copy"
        };
        box_el = box_el.child(box_button("desk-reply-copy", copy_label, cx, |this, cx| {
            if let Some(pop) = this.popout.clone() {
                this.copy_text(&pop.draft, cx);
                this.action_note = termy_x::trends_desk::copied_mark().into();
            }
            cx.notify();
        }));
        if let Some(url) = posted_url.clone() {
            let shown = url.clone();
            box_el = box_el.child(
                div()
                    .id("desk-posted-link")
                    .text_size(px(13.0))
                    .whitespace_normal()
                    .text_color(rgb(0x070b14))
                    .cursor_pointer()
                    .child(format!("Posted ✓ {shown}"))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.reader = Some(crate::trends_desk::InlineRead {
                                title: "Reply".into(),
                                body: "Posted from this window.".into(),
                                url: url.clone(),
                            });
                            this.list_pick = Some("reader".into());
                            this.action_note = format!("Posted ✓ {shown}");
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            );
        }
        let _ = post_label;
        Some(box_el.into_any_element())
    }

    pub(crate) fn pinned_post_button(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let Some(pop) = self.popout.clone().filter(|pop| pop.open) else {
            return div().into_any_element();
        };
        let handle = self
            .columns
            .iter()
            .flat_map(|column| column.view.posts.iter())
            .find(|post| post.id == pop.card_id)
            .map(|post| post.author.clone())
            .unwrap_or_else(|| "author".into());
        let phase = self.card_replies.get(&pop.card_id).cloned().unwrap_or(
            termy_x::trends_desk::ReplyPhase::Editing {
                text: pop.draft.clone(),
            },
        );
        let face = match &phase {
            termy_x::trends_desk::ReplyPhase::Pending { .. } => {
                termy_x::trends_desk::PostFace::Posting
            }
            termy_x::trends_desk::ReplyPhase::Confirmed { .. } => {
                termy_x::trends_desk::PostFace::Posted
            }
            termy_x::trends_desk::ReplyPhase::Failed { .. } => {
                termy_x::trends_desk::PostFace::Retry
            }
            _ => termy_x::trends_desk::PostFace::Ready,
        };
        let label = termy_x::trends_desk::post_button_text(face, &handle);
        post_button("desk-post-reply", &label, cx, |this, cx| {
            this.action_note = "Posting…".into();
            this.send_from_popout(cx);
            cx.notify();
        })
    }

    pub(crate) fn load_provider_models(&mut self, cx: &mut Context<Self>) {
        let (hermes, hermes_key, custom) = self.provider_probe();
        cx.spawn(async move |this, cx| {
            let http = termy_x::io::UreqHttp::with_timeout(std::time::Duration::from_secs(5));
            let hermes_models = model_ids(&http, &hermes, &hermes_key);
            let custom_models = if custom.trim().is_empty() {
                Vec::new()
            } else {
                model_ids(&http, &custom, "")
            };
            let vyce_key = std::env::var("VYCE_API_KEY").unwrap_or_default();
            let vyce_models = if vyce_key.is_empty() {
                Vec::new()
            } else {
                model_ids(&http, termy_x::trends_desk::VYCE_DIRECT_BASE, &vyce_key)
            };
            let grok_models = termy_x::trends_desk::refresh_grok_models(
                &termy_x::trends_desk::termy_xl_data_root(),
            );
            let menu = termy_x::trends_desk::menu_with_models(
                &hermes_models,
                &vyce_models,
                &custom_models,
            );
            let mut extras: Vec<_> = menu
                .into_iter()
                .filter(|(id, _)| id.contains(':'))
                .collect();
            for model in grok_models {
                let id = format!("grok:{model}");
                let label = format!("Grok · {model}");
                if !extras.iter().any(|(have, _)| have == &id) {
                    extras.push((id, label));
                }
            }
            let _ = this.update(cx, |desk, cx| {
                desk.extra_models = extras;
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn provider_dropdown(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let current = termy_x::trends_desk::provider_label(&self.settings.take_provider);
        let mut menu = termy_x::trends_desk::base_provider_menu();
        for extra in &self.extra_models {
            if !menu.iter().any(|(id, _)| id == &extra.0) {
                menu.push(extra.clone());
            }
        }
        let open = self.provider_menu;
        let verify_ok = self.verify_ok;
        let verify_line = self.verify_line.clone();
        let mut column = div().flex().flex_col().gap(px(4.0)).child(box_button(
            "desk-provider-combo",
            &format!("{current} ▾"),
            cx,
            |this, cx| {
                this.provider_menu = !this.provider_menu;
                if this.provider_menu {
                    this.load_provider_models(cx);
                }
                cx.notify();
            },
        ));
        if open {
            for (id, label) in menu {
                let chosen = id.clone();
                column = column.child(box_button(
                    &format!("desk-provider-{id}"),
                    &label,
                    cx,
                    move |this, cx| {
                        this.provider_menu = false;
                        this.choose_provider(&chosen, cx);
                    },
                ));
            }
        }
        if !verify_line.is_empty() {
            let color = if verify_ok == Some(true) {
                rgb(0x30d158)
            } else {
                rgb(0xff453a)
            };
            column = column.child(
                div()
                    .text_size(px(12.0))
                    .text_color(color)
                    .whitespace_normal()
                    .child(verify_line),
            );
        }
        column.into_any_element()
    }

    fn place_reply_cursor(&mut self, x: f32, y: f32, extend: bool) {
        self.carousel_keys = false;
        let Some(mut editor) = self.reply_edit.clone() else {
            return;
        };
        let (origin_x, origin_y, width) = editor_box();
        if extend {
            editor.shift_click(x, y, origin_x, origin_y, width);
        } else {
            editor.click_at(x, y, origin_x, origin_y, width);
        }
        if let Some(pop) = self.popout.as_mut() {
            pop.draft = editor.text.clone();
        }
        self.pop_caret = editor.caret;
        self.reply_edit = Some(editor);
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
        self.reply_edit = Some(termy_x::draft_edit::ReplyEditor::new(text));
        self.reply_drag = false;
        self.popout = Some(editor);
    }

    pub(crate) fn edit_reply_key(
        &mut self,
        key: &str,
        event: &gpui_kit::KeyDownEvent,
        cx: &mut Context<Self>,
    ) {
        use termy_x::draft_edit::Motion;
        let mods = &event.keystroke.modifiers;
        let command = mods.control || mods.platform;
        let Some(mut editor) = self.reply_edit.clone() else {
            return;
        };
        if command && key == "a" {
            editor.select_all();
        } else if command && key == "c" {
            self.copy_text(&editor.copy_text(), cx);
            self.action_note = termy_x::trends_desk::copied_mark().into();
        } else if command && key == "x" {
            let cut = editor.cut();
            if !cut.is_empty() {
                self.copy_text(&cut, cx);
                self.action_note = termy_x::trends_desk::copied_mark().into();
            }
        } else if command && key == "v" {
            if let Some(item) = cx.read_from_clipboard()
                && let Some(text) = item.text()
            {
                editor.paste(text.as_ref());
            }
        } else if command && key == "z" {
            editor.undo();
        } else if key == "backspace" {
            editor.backspace();
        } else if key == "delete" {
            editor.delete();
        } else if key == "left" {
            editor.move_key(Motion::Left, mods.shift);
        } else if key == "right" {
            editor.move_key(Motion::Right, mods.shift);
        } else if key == "up" {
            editor.move_key(Motion::Up, mods.shift);
        } else if key == "down" {
            editor.move_key(Motion::Down, mods.shift);
        } else if key == "home" {
            editor.move_key(Motion::Home, mods.shift);
        } else if key == "end" {
            editor.move_key(Motion::End, mods.shift);
        } else if key == "enter" {
            editor.insert("\n");
        } else if let Some(ch) = event.keystroke.key_char.as_deref()
            && !command
            && !ch.is_empty()
        {
            editor.insert(ch);
        } else {
            cx.notify();
            return;
        }
        self.finish_reply_edit(editor, cx);
    }

    fn finish_reply_edit(
        &mut self,
        editor: termy_x::draft_edit::ReplyEditor,
        cx: &mut Context<Self>,
    ) {
        let text = editor.text.clone();
        self.pop_caret = editor.caret;
        if let Some(pop) = self.popout.as_mut() {
            pop.draft = text.clone();
            let id = pop.card_id.clone();
            self.card_replies.insert(
                id,
                termy_x::trends_desk::ReplyPhase::Editing { text: text.clone() },
            );
        }
        self.reply_edit = Some(editor);
        self.persist_open_reply();
        cx.notify();
    }

    fn persist_open_reply(&mut self) {
        let Some(pop) = self.popout.clone() else {
            return;
        };
        let Some(post) = self
            .columns
            .iter()
            .flat_map(|column| column.view.posts.iter())
            .find(|post| post.id == pop.card_id)
            .cloned()
        else {
            return;
        };
        if pop.draft.trim().is_empty() || pop.draft == termy_x::trends_desk::DRAFTING_PLACEHOLDER {
            return;
        }
        let path = termy_x::trends_desk::drafts_file(&self.data_root);
        let author = post.author.trim().trim_start_matches('@');
        if !author.is_empty() && !post.url.is_empty() {
            self.pack
                .authors
                .insert(post.url.clone(), author.to_string());
            self.save_pack_file();
        }
        if let Ok(count) = termy_x::trends_desk::remember_take(
            &path,
            &pop.card_id,
            &post.text,
            &post.url,
            &pop.draft,
        ) {
            self.saved_urls.insert(post.url);
            self.draft_count = count;
        }
    }

    pub(crate) fn edit_saved_draft_key(
        &mut self,
        key: &str,
        event: &gpui_kit::KeyDownEvent,
        cx: &mut Context<Self>,
    ) {
        use termy_x::draft_edit::Motion;
        let Some(id) = self
            .list_pick
            .as_deref()
            .and_then(|pick| pick.strip_prefix("draft:"))
            .map(str::to_string)
        else {
            return;
        };
        if self.reply_edit.is_none() {
            self.reply_edit = Some(termy_x::draft_edit::ReplyEditor::new(&self.draft_text));
        }
        let mods = &event.keystroke.modifiers;
        let command = mods.control || mods.platform;
        let Some(mut editor) = self.reply_edit.clone() else {
            return;
        };
        if command && key == "a" {
            editor.select_all();
        } else if command && key == "c" {
            self.copy_text(&editor.copy_text(), cx);
            self.action_note = termy_x::trends_desk::copied_mark().into();
            self.reply_edit = Some(editor);
            cx.notify();
            return;
        } else if command && key == "x" {
            let cut = editor.cut();
            if !cut.is_empty() {
                self.copy_text(&cut, cx);
            }
        } else if command && key == "v" {
            if let Some(item) = cx.read_from_clipboard()
                && let Some(text) = item.text()
            {
                editor.paste(text.as_ref());
            }
        } else if command && key == "z" {
            editor.undo();
        } else if key == "backspace" {
            editor.backspace();
        } else if key == "delete" {
            editor.delete();
        } else if key == "left" {
            editor.move_key(Motion::Left, mods.shift);
        } else if key == "right" {
            editor.move_key(Motion::Right, mods.shift);
        } else if key == "home" {
            editor.move_key(Motion::Home, mods.shift);
        } else if key == "end" {
            editor.move_key(Motion::End, mods.shift);
        } else if key == "enter" {
            editor.insert("\n");
        } else if let Some(ch) = event.keystroke.key_char.as_deref()
            && !command
            && !ch.is_empty()
        {
            editor.insert(ch);
        } else {
            cx.notify();
            return;
        }
        self.draft_text = editor.text.clone();
        self.reply_edit = Some(editor);
        let path = termy_x::trends_desk::drafts_file(&self.data_root);
        if let Err(error) = termy_x::trends_desk::rewrite_draft(&path, &id, &self.draft_text) {
            self.action_note = error;
        }
        cx.notify();
    }

    pub(crate) fn choose_provider(&mut self, name: &str, cx: &mut Context<Self>) {
        let name = termy_x::trends_desk::canonical_provider(name);
        self.remember_provider(&name);
        let label = termy_x::trends_desk::provider_label(&self.settings.take_provider);
        self.action_note = format!("Next take uses {label}.");
        self.verify_line = format!("Checking {label}…");
        self.verify_ok = None;
        self.verify_gen = self.verify_gen.saturating_add(1);
        let generation = self.verify_gen;
        self.spawn_verify(generation, cx);
        cx.notify();
    }

    fn spawn_verify(&mut self, generation: u64, cx: &mut Context<Self>) {
        let saved = self.settings.take_provider.clone();
        let kind = termy_x::trends_desk::provider_kind(&saved);
        let label = termy_x::trends_desk::provider_label(&saved);
        let (hermes, hermes_key, custom) = self.provider_probe();
        let model = termy_x::trends_desk::provider_model(&saved)
            .unwrap_or_else(|| termy_x::trends_desk::VYCE_MODEL.to_string());
        cx.spawn(async move |this, cx| {
            let started = std::time::Instant::now();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let limit = if kind == termy_x::trends_desk::TakeProvider::Hermes {
                        termy_x::trends_desk::HERMES_TAKE_TIMEOUT
                    } else {
                        termy_x::trends_desk::PROBE_TIMEOUT
                    };
                    let http = termy_x::io::UreqHttp::with_timeout(limit);
                    match kind {
                        termy_x::trends_desk::TakeProvider::Hermes => {
                            termy_x::trends_desk::probe_openai(
                                &http,
                                &hermes,
                                termy_x::trends_desk::HERMES_MODEL,
                                &hermes_key,
                            )
                            .map(|()| String::new())
                        }
                        termy_x::trends_desk::TakeProvider::Vyce => {
                            let key = std::env::var("VYCE_API_KEY").unwrap_or_default();
                            termy_x::trends_desk::probe_openai(
                                &http,
                                termy_x::trends_desk::VYCE_DIRECT_BASE,
                                &model,
                                &key,
                            )
                            .map(|()| String::new())
                        }
                        termy_x::trends_desk::TakeProvider::Custom => {
                            termy_x::trends_desk::probe_openai(&http, &custom, &model, "")
                                .map(|()| String::new())
                        }
                        termy_x::trends_desk::TakeProvider::Grok => probe_cli("grok"),
                        termy_x::trends_desk::TakeProvider::Muse => probe_cli("muse"),
                        termy_x::trends_desk::TakeProvider::Agy => probe_cli("agy"),
                        _ => Err("that provider is not available".into()),
                    }
                })
                .await;
            let millis = started.elapsed().as_millis();
            let _ = this.update(cx, |desk, cx| {
                if desk.verify_gen != generation {
                    return;
                }
                if desk.settings.take_provider != saved {
                    return;
                }
                let (ok, line) = match result {
                    Ok(reply) => {
                        let mut line =
                            termy_x::trends_desk::verify_status(&label, true, millis, "");
                        let reply = reply.trim();
                        if !reply.is_empty() {
                            let short: String = reply.chars().take(80).collect();
                            line = format!("{line} · {short}");
                        }
                        (true, line)
                    }
                    Err(error) => (
                        false,
                        termy_x::trends_desk::verify_status(&label, false, millis, &error),
                    ),
                };
                desk.verify_ok = Some(ok);
                desk.verify_line = line.clone();
                desk.action_note = line;
                cx.notify();
            });
        })
        .detach();
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

    pub(crate) fn spawn_one_take(
        &mut self,
        id: String,
        open_compose: bool,
        cx: &mut Context<Self>,
    ) {
        self.takes_halted = false;
        termy_x::trends_desk::resume_takes();
        let Some(post) = self
            .columns
            .iter()
            .flat_map(|column| column.view.posts.iter())
            .find(|post| post.id == id)
            .cloned()
        else {
            if open_compose {
                let message = termy_x::trends_desk::text_after_take(
                    termy_x::trends_desk::DRAFTING_PLACEHOLDER,
                    "",
                    "this card has no post",
                );
                self.show_inline_editor(&id, &message);
                cx.notify();
            }
            return;
        };
        self.takes.insert(
            id.clone(),
            TakeDraft {
                text: String::new(),
                source: "pending",
                note: String::new(),
                variants: Vec::new(),
            },
        );
        let saved = self.settings.take_provider.clone();
        let provider = termy_x::trends_desk::provider_kind(&saved);
        let mut config = self.config.clone();
        if provider == TakeProvider::Hermes {
            config.hermes_model = termy_x::trends_desk::HERMES_MODEL.to_string();
            config.hermes_api_key =
                termy_x::trends_desk::hermes_key_resolved(&config.hermes_api_key);
        }
        if provider == TakeProvider::Vyce
            && let Some(model) = termy_x::trends_desk::provider_model(&saved)
        {
            config.ai_model = model;
            config.ai_explicit = true;
        }
        if provider == TakeProvider::Custom {
            config.ai_base_url = self.settings.custom_base.clone();
            config.ai_model = self.settings.custom_model.clone();
            config.ai_key_env = self.settings.custom_key_env.clone();
            config.ai_api_key.clear();
        }
        let tone = termy_x::trends_desk::panel_tone_name(termy_x::trends_desk::TakeTone::parse(
            &self.pack.tone,
        ))
        .to_string();
        let fallback = self.settings.auto_fallback && provider != TakeProvider::Grok;
        let grok_model = if provider == TakeProvider::Grok {
            termy_x::trends_desk::provider_model(&saved)
        } else {
            None
        };
        self.take_epoch = self.take_epoch.wrapping_add(1);
        let epoch = self.take_epoch;
        if self.take_started.is_none() {
            self.take_started = Some(std::time::Instant::now());
        }
        if open_compose && !self.more_take {
            self.load_take_history(&id);
            self.show_inline_editor(&id, termy_x::trends_desk::DRAFTING_PLACEHOLDER);
        }
        self.more_take = false;
        let posted = post.clone();
        cx.spawn(async move |this, cx| {
            let (note_tx, note_rx) = std::sync::mpsc::channel();
            let (part_tx, part_rx) = std::sync::mpsc::channel();
            let (done_tx, done_rx) = std::sync::mpsc::channel();
            let tone_done = tone.clone();
            std::thread::spawn(move || {
                let _guard = termy_x::trends_desk::bind_retry_notice(Some(note_tx));
                let _stream = termy_x::trends_desk::bind_grok_stream(Some(part_tx));
                let _model = termy_x::trends_desk::bind_grok_model(grok_model);
                let limit = if provider == TakeProvider::Grok {
                    termy_x::trends_desk::GROK_TAKE_TIMEOUT
                } else if provider == TakeProvider::Hermes {
                    termy_x::trends_desk::HERMES_TAKE_TIMEOUT
                } else {
                    termy_x::trends_desk::TAKE_TIMEOUT
                };
                let http = termy_x::io::UreqHttp::with_timeout(limit);
                let result = if fallback {
                    termy_x::trends_desk::run_take_fallback(
                        provider, &config, &post, &http, &tone, true,
                    )
                } else {
                    termy_x::trends_desk::run_take_chain(provider, &config, &post, &http, &tone)
                };
                let _ = done_tx.send(result);
                drop(_stream);
                drop(_guard);
            });
            let result = loop {
                match done_rx.try_recv() {
                    Ok(result) => break result,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        break TakeDraft {
                            text: String::new(),
                            source: "offline",
                            note: "take stopped".into(),
                            variants: Vec::new(),
                        };
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        while let Ok(msg) = note_rx.try_recv() {
                            let _ = this.update(cx, |desk, cx| {
                                desk.action_note = msg;
                                cx.notify();
                            });
                        }
                        while let Ok(chunk) = part_rx.try_recv() {
                            let _ = this.update(cx, |desk, cx| {
                                desk.stream_take = chunk;
                                cx.notify();
                            });
                        }
                        let _ = this.update(cx, |_, cx| cx.notify());
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(200))
                            .await;
                    }
                }
            };
            let _ = this.update(cx, |desk, cx| {
                if desk.take_epoch != epoch {
                    cx.notify();
                    return;
                }
                desk.land_take(&id, &posted, &tone_done, result);
                cx.notify();
            });
        })
        .detach();
    }

    fn land_take(
        &mut self,
        id: &str,
        post: &termy_x::trends_desk::SourcePost,
        tone: &str,
        result: TakeDraft,
    ) {
        self.takes.insert(id.to_string(), result.clone());
        self.take_generating = false;
        self.take_started = None;
        self.stream_take.clear();
        let status = if result.text.trim().is_empty() {
            result.note.clone()
        } else {
            "ok".into()
        };
        termy_x::trends_desk::append_desk_log(
            &self.data_root,
            &termy_x::trends_desk::take_log_line(result.source, &status),
        );
        let next = termy_x::trends_desk::text_after_take(
            termy_x::trends_desk::DRAFTING_PLACEHOLDER,
            &result.text,
            &result.note,
        );
        let cleared =
            termy_x::trends_desk::take_cleared_grok_model().filter(|_| result.source == "grok");
        if cleared.is_some() && self.settings.take_provider.starts_with("grok:") {
            self.settings.take_provider = "grok".into();
            self.config.take_provider = "grok".into();
            if self.writable {
                let _ = self.store.save_settings(&self.settings);
                let _ = self.config.save_take_provider("grok");
            }
        }
        if next.trim().is_empty() {
            let mut banner = termy_x::trends_desk::failure_banner(result.source, &result.note);
            if let Some(id) = &cleared
                && !banner.contains("Cleared unknown")
            {
                banner =
                    format!("Cleared unknown Grok model '{id}'. Used the CLI default. {banner}");
            }
            self.take_banner = banner.clone();
            self.action_note = banner;
            if let Some(pop) = self.popout.as_mut().filter(|pop| pop.card_id == id) {
                if pop.draft == termy_x::trends_desk::DRAFTING_PLACEHOLDER {
                    pop.draft.clear();
                }
            }
            self.reply_edit = Some(termy_x::draft_edit::ReplyEditor::new(""));
            return;
        }
        let author = post.author.trim().trim_start_matches('@');
        if !author.is_empty() && !post.url.is_empty() {
            self.pack
                .authors
                .insert(post.url.clone(), author.to_string());
        }
        self.take_banner = cleared
            .map(|id| format!("Cleared unknown Grok model '{id}'. Used the CLI default."))
            .unwrap_or_default();
        let label = if result.note.trim().is_empty() {
            result.source.to_string()
        } else {
            result.note.clone()
        };
        termy_x::trends_desk::remember_saved_take(&mut self.pack, id, &next, &label, tone);
        self.save_pack_file();
        self.take_cards = self.pack.takes.get(id).cloned().unwrap_or_default();
        self.take_index = self.take_cards.len().saturating_sub(1);
        if let Some(pop) = self.popout.as_mut().filter(|pop| pop.card_id == id) {
            pop.draft = next.clone();
            pop.variants = termy_x::trends_desk::distinct_variants(&next, &result.variants);
        }
        self.pop_caret = termy_x::draft_edit::DraftCaret::at_end(next.len());
        self.reply_edit = Some(termy_x::draft_edit::ReplyEditor::new(&next));
        self.card_replies
            .insert(id.to_string(), edit_reply(&ReplyPhase::Idle, next.clone()));
        self.draft_note = result.note.clone();
        self.action_note = label;
        let path = termy_x::trends_desk::drafts_file(&self.data_root);
        if let Ok(count) =
            termy_x::trends_desk::append_take_draft(&path, id, &post.text, &post.url, &next)
        {
            self.saved_urls.insert(post.url.clone());
            self.draft_count = count;
        }
        self.selected = Some(format!("take:{id}"));
    }

    fn load_take_history(&mut self, id: &str) {
        self.take_cards = self.pack.takes.get(id).cloned().unwrap_or_default();
        self.take_index = self.take_cards.len().saturating_sub(1);
        self.carousel_keys = true;
    }

    pub(crate) fn nudge_carousel(&mut self, delta: i32, cx: &mut Context<Self>) {
        match termy_x::trends_desk::slide(
            self.take_cards.len(),
            self.take_index,
            self.take_generating,
            delta,
        ) {
            termy_x::trends_desk::Slide::Stay => {}
            termy_x::trends_desk::Slide::Move(index) => {
                self.take_index = index;
                self.use_current_take();
                cx.notify();
            }
            termy_x::trends_desk::Slide::More => self.request_another_take(cx),
        }
    }

    fn use_current_take(&mut self) {
        let Some(card) = self.take_cards.get(self.take_index).cloned() else {
            return;
        };
        let Some(pop) = self.popout.as_mut() else {
            return;
        };
        pop.draft = card.text.clone();
        self.pop_caret = termy_x::draft_edit::DraftCaret::at_end(card.text.len());
        self.reply_edit = Some(termy_x::draft_edit::ReplyEditor::new(&card.text));
        self.card_replies.insert(
            pop.card_id.clone(),
            edit_reply(&ReplyPhase::Idle, card.text),
        );
        self.take_banner.clear();
    }

    pub(crate) fn request_another_take(&mut self, cx: &mut Context<Self>) {
        if self.take_generating {
            return;
        }
        let Some(id) = self.popout.as_ref().map(|pop| pop.card_id.clone()) else {
            return;
        };
        self.more_take = true;
        self.take_generating = true;
        self.take_started = Some(std::time::Instant::now());
        self.take_banner.clear();
        self.stream_take.clear();
        self.spawn_one_take(id, false, cx);
    }

    pub(crate) fn cancel_take(&mut self, cx: &mut Context<Self>) {
        self.take_epoch = self.take_epoch.wrapping_add(1);
        termy_x::trends_desk::halt_takes();
        self.take_generating = false;
        self.take_started = None;
        self.stream_take.clear();
        self.take_banner = "Cancelled".into();
        self.action_note = "Cancelled".into();
        cx.notify();
    }

    fn try_provider(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(id) = self.popout.as_ref().map(|pop| pop.card_id.clone()) else {
            return;
        };
        self.choose_provider(name, cx);
        self.take_banner.clear();
        self.more_take = true;
        self.take_generating = true;
        self.take_started = Some(std::time::Instant::now());
        self.spawn_one_take(id, false, cx);
    }

    fn elapsed_secs(&self) -> u64 {
        self.take_started
            .map(|start| start.elapsed().as_secs())
            .unwrap_or(0)
    }

    fn take_carousel(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let index = self.take_index;
        let generating = self.take_generating;
        let count = termy_x::trends_desk::indicator(self.take_cards.len(), index, generating);
        let shown = if generating && !self.stream_take.is_empty() {
            self.stream_take.clone()
        } else if generating {
            format!("… {}s", self.elapsed_secs())
        } else if let Some(card) = self.take_cards.get(index) {
            format!("{}\n{}", card.provider, card.text)
        } else {
            "Press → for a take.".into()
        };
        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(box_button("desk-take-left", "←", cx, |this, cx| {
                this.carousel_keys = true;
                this.nudge_carousel(-1, cx);
            }))
            .child(
                div()
                    .id("desk-take-card")
                    .w(px(240.0))
                    .min_h(px(72.0))
                    .p(px(8.0))
                    .rounded(px(8.0))
                    .bg(rgb(0x2a1020))
                    .text_size(px(14.0))
                    .text_color(rgb(0xffe56a))
                    .whitespace_normal()
                    .cursor_pointer()
                    .child(shown)
                    .child(
                        div()
                            .mt(px(6.0))
                            .text_size(px(12.0))
                            .text_color(rgb(0x7af6ff))
                            .child(count),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.carousel_keys = true;
                            this.use_current_take();
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
                    .with_animation(
                        SharedString::from(format!("desk-slide-{index}-{generating}")),
                        Animation::new(std::time::Duration::from_millis(280)),
                        |card, t| card.ml(px((1.0 - t) * 32.0)),
                    ),
            )
            .child(box_button("desk-take-right", "→", cx, |this, cx| {
                this.carousel_keys = true;
                this.nudge_carousel(1, cx);
            }))
            .child(box_button("desk-take-use", "Use this", cx, |this, cx| {
                this.use_current_take();
                cx.notify();
            }))
            .into_any_element()
    }

    fn failure_banner(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let text = self.take_banner.clone();
        div()
            .p(px(10.0))
            .rounded(px(8.0))
            .bg(rgb(0x7a1020))
            .text_size(px(16.0))
            .text_color(rgb(0xffe56a))
            .whitespace_normal()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(text)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.0))
                    .child(post_button("desk-take-retry", "Retry", cx, |this, cx| {
                        this.request_another_take(cx);
                    }))
                    .child(post_button(
                        "desk-try-hermes",
                        "Try Hermes",
                        cx,
                        |this, cx| {
                            this.try_provider("hermes", cx);
                        },
                    ))
                    .child(post_button("desk-try-muse", "Try Muse", cx, |this, cx| {
                        this.try_provider("muse", cx);
                    })),
            )
            .into_any_element()
    }

    fn cancel_row(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        post_button("desk-cancel-take", "Cancel", cx, |this, cx| {
            this.cancel_take(cx);
        })
    }

    pub(crate) fn send_reply(&mut self, id: String, cx: &mut Context<Self>) {
        let phase = self
            .card_replies
            .get(&id)
            .cloned()
            .unwrap_or(ReplyPhase::Idle);
        let Some(post) = self
            .columns
            .iter()
            .flat_map(|column| column.view.posts.iter())
            .find(|post| post.id == id)
            .cloned()
        else {
            return;
        };
        match request_send(&phase, true) {
            Err(error) => {
                self.action_note = error.clone();
                let text = phase.text().to_string();
                if !text.is_empty() {
                    self.card_replies
                        .insert(id, ReplyPhase::Failed { text, error });
                }
                cx.notify();
            }
            Ok(pending) => {
                let text = termy_x::trends_desk::text_to_post(&pending.text());
                let pending = ReplyPhase::Pending { text: text.clone() };
                self.card_replies.insert(id.clone(), pending.clone());
                cx.notify();
                let bin = self.config.opencli_bin.clone();
                let url = post.url.clone();
                let parent_id = post.id.clone();
                let provider = self.settings.take_provider.clone();
                let receipt_url = url.clone();
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_executor()
                        .spawn(async move {
                            let runner = termy_x::io::SystemRunner;
                            post_reply_opencli(&bin, &runner, &url, &text)
                        })
                        .await;
                    let _ = this.update(cx, |desk, cx| {
                        let pending = desk.card_replies.get(&id).cloned().unwrap_or(pending);
                        let posted = termy_x::trends_desk::text_to_post(pending.text());
                        let outcome = if result.is_ok() { "ok" } else { "failed" };
                        termy_x::trends_desk::append_desk_log(
                            &desk.data_root,
                            &termy_x::trends_desk::post_log_line(
                                &provider, &parent_id, outcome, &posted,
                            ),
                        );
                        let mut mark = termy_x::trends_desk::reply_receipt(
                            &parent_id,
                            &receipt_url,
                            result.as_deref().unwrap_or(""),
                        );
                        mark.at_unix = TrendsDesk::now_unix();
                        mark.status = if result.is_ok() {
                            "confirmed".into()
                        } else {
                            "failed".into()
                        };
                        mark.text = posted;
                        desk.pack.receipts.push(mark);
                        desk.save_pack_file();
                        desk.action_note = match &result {
                            Ok(receipt) => {
                                if let Some(url) = termy_x::trends_desk::reply_link(receipt) {
                                    format!("Posted ✓ {url}")
                                } else {
                                    "Posted ✓".into()
                                }
                            }
                            Err(error) => error.clone(),
                        };
                        desk.card_replies
                            .insert(id.clone(), apply_send_result(&pending, result));
                        if desk
                            .card_replies
                            .get(&id)
                            .is_some_and(|phase| matches!(phase, ReplyPhase::Confirmed { .. }))
                        {
                            if desk.popout.as_ref().is_some_and(|pop| pop.card_id == id) {
                                desk.popout = None;
                                desk.reply_edit = None;
                            }
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
        }
    }
}

fn variant_chip(
    label: &'static str,
    index: usize,
    cx: &mut Context<TrendsDesk>,
) -> gpui_kit::AnyElement {
    box_button(&format!("desk-tone-{label}"), label, cx, move |this, cx| {
        let _ = index;
        this.action_note = format!("Drafting {label}…");
        this.set_pop_tone(label);
        this.request_another_take(cx);
        cx.notify();
    })
}

fn post_button(
    id: &str,
    label: &str,
    cx: &mut Context<TrendsDesk>,
    handler: impl Fn(&mut TrendsDesk, &mut Context<TrendsDesk>) + 'static,
) -> gpui_kit::AnyElement {
    div()
        .id(SharedString::from(id.to_string()))
        .w_full()
        .flex_shrink_0()
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
        .hover(|style| style.bg(rgb(0xffe56a)))
        .active(|style| style.bg(rgb(0xff4fa3)).text_color(rgb(0x070b14)))
        .child(label.to_string())
        .tooltip({
            let tip: SharedString = label.to_string().into();
            move |_, cx| {
                cx.new(|_| crate::trends_desk::HintView { text: tip.clone() })
                    .into()
            }
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                handler(this, cx);
                cx.stop_propagation();
            }),
        )
        .into_any_element()
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
        .tooltip({
            let tip: SharedString = label.to_string().into();
            move |_, cx| {
                cx.new(|_| crate::trends_desk::HintView { text: tip.clone() })
                    .into()
            }
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                handler(this, cx);
                cx.stop_propagation();
            }),
        )
        .into_any_element()
}

fn probe_cli(program: &str) -> Result<String, String> {
    let mut command = std::process::Command::new(program);
    command.args(termy_x::trends_desk::probe_cli_args(program));
    termy_x::trends_desk::apply_hidden_command(&mut command);
    let _ = command.stdin(std::process::Stdio::null());
    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return Err(format!("{program} not found ({error})")),
    };
    let pid = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    let output = match rx.recv_timeout(termy_x::trends_desk::PROBE_TIMEOUT) {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => return Err(error.to_string()),
        Err(_) => {
            stop_probe(pid);
            return Err(format!("{program} timed out after 30s"));
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if program == "agy" {
        if stdout.trim().is_empty() {
            if let Some(credit) = termy_x::trends_desk::agy_credit_error(&stderr) {
                return Err(credit);
            }
        } else {
            match termy_x::trends_desk::agy_print_text(&stdout) {
                Ok(text) => return Ok(text),
                Err(error) => {
                    return Err(termy_x::trends_desk::agy_credit_error(&error).unwrap_or(error));
                }
            }
        }
    }
    if !output.status.success() {
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        if let Some(credit) = termy_x::trends_desk::agy_credit_error(&detail) {
            return Err(credit);
        }
        return Err(if detail.is_empty() {
            format!("{program} exited {}", output.status)
        } else {
            detail
        });
    }
    let shown = if !stdout.is_empty() { stdout } else { stderr };
    if shown.is_empty() {
        Err(format!("{program} returned no text"))
    } else {
        Ok(shown)
    }
}

fn stop_probe(pid: u32) {
    if pid == 0 {
        return;
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .output();
    }
}

fn model_ids(http: &termy_x::io::UreqHttp, base: &str, key: &str) -> Vec<String> {
    let url = format!("{}/models", base.trim().trim_end_matches('/'));
    let request = termy_x::io::HttpRequest {
        method: "GET",
        url,
        body: None,
        bearer: {
            let key = key.trim();
            if key.is_empty() {
                None
            } else {
                Some(key.to_string())
            }
        },
        content_type: None,
        headers: Vec::new(),
    };
    match http.request(request) {
        Ok(body) => termy_x::trends_desk::models_from_openai(&body),
        Err(_) => Vec::new(),
    }
}

fn remember_editor_box(x: f32, y: f32, width: f32) {
    EDITOR_BOX.with(|cell| cell.set((x, y, width)));
}

fn editor_box() -> (f32, f32, f32) {
    EDITOR_BOX.with(|cell| cell.get())
}

thread_local! {
    static EDITOR_BOX: std::cell::Cell<(f32, f32, f32)> = const { std::cell::Cell::new((0.0, 0.0, 360.0)) };
}
