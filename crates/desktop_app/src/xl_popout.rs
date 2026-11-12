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
use termy_x::trends_desk::{TakeTone, panel_tone_name, panel_tones};

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
                        drafting_mark()
                    } else {
                        reply_surface(&shown, &caret)
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
            let menu = termy_x::trends_desk::menu_with_models(
                &hermes_models,
                &vyce_models,
                &custom_models,
            );
            let extras: Vec<_> = menu
                .into_iter()
                .filter(|(id, _)| id.contains(':'))
                .collect();
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
                    let http =
                        termy_x::io::UreqHttp::with_timeout(std::time::Duration::from_secs(8));
                    match kind {
                        termy_x::trends_desk::TakeProvider::Hermes => {
                            termy_x::trends_desk::probe_openai(&http, &hermes, &model, &hermes_key)
                        }
                        termy_x::trends_desk::TakeProvider::Vyce => {
                            let key = std::env::var("VYCE_API_KEY").unwrap_or_default();
                            termy_x::trends_desk::probe_openai(
                                &http,
                                termy_x::trends_desk::VYCE_DIRECT_BASE,
                                &model,
                                &key,
                            )
                        }
                        termy_x::trends_desk::TakeProvider::Custom => {
                            termy_x::trends_desk::probe_openai(&http, &custom, &model, "")
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
                    Ok(()) => (
                        true,
                        termy_x::trends_desk::verify_status(&label, true, millis, ""),
                    ),
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
        this.regenerate_open_variants(cx);
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

fn probe_cli(program: &str) -> Result<(), String> {
    let mut command = std::process::Command::new(program);
    command.args(termy_x::trends_desk::probe_cli_args(program));
    termy_x::trends_desk::apply_hidden_command(&mut command);
    match command.output() {
        Err(error) => Err(format!("{program} not found ({error})")),
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => {
            let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let out = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let detail = if !err.is_empty() { err } else { out };
            if detail.is_empty() {
                Err(format!("{program} --version failed"))
            } else {
                Err(detail)
            }
        }
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

fn breath_animation() -> Animation {
    Animation::new(std::time::Duration::from_millis(1600))
        .repeat()
        .with_easing(|t| 0.5 - 0.5 * (t * std::f32::consts::TAU).cos())
}

fn drafting_mark() -> gpui_kit::AnyElement {
    div()
        .text_size(px(28.0))
        .text_color(rgb(0xffe56a))
        .child("…")
        .with_animation("desk-drafting-breath", breath_animation(), |el, value| {
            el.opacity(0.2 + 0.8 * value)
        })
        .into_any_element()
}

fn reply_surface(text: &str, caret: &termy_x::draft_edit::DraftCaret) -> gpui_kit::AnyElement {
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

fn remember_editor_box(x: f32, y: f32, width: f32) {
    EDITOR_BOX.with(|cell| cell.set((x, y, width)));
}

fn editor_box() -> (f32, f32, f32) {
    EDITOR_BOX.with(|cell| cell.get())
}

thread_local! {
    static EDITOR_BOX: std::cell::Cell<(f32, f32, f32)> = const { std::cell::Cell::new((0.0, 0.0, 360.0)) };
}
