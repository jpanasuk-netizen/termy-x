//! Heat, filters, hit list, receipts, palette, and keys for Termy XL.
//!
//! Keys never send. Refresh only starts a search.

use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::{
    AppContext, ClipboardItem, Context, InteractiveElement, IntoElement, Keystroke, MouseButton,
    ParentElement, StatefulInteractiveElement, Styled, Window, div, px, rgb,
};

use crate::trends_desk::{ColumnUi, TrendsDesk, button, open_status};
use termy_x::trends_desk::{
    DeskKey, PackFile, Phase, SessionState, SourcePost, TakeTone, WINDOW_PAD, apply_filters,
    desk_key, hud_line, layout_takes, load_pack, move_selection, new_ids, page_label,
    replied_badge, save_pack,
};

impl TrendsDesk {
    pub(crate) fn load_pack_file(&self) -> PackFile {
        load_pack(&self.data_root)
    }

    pub(crate) fn save_pack_file(&self) {
        if self.writable {
            let _ = save_pack(&self.data_root, &self.pack);
        }
    }

    pub(crate) fn now_unix() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    pub(crate) fn visible_posts<'a>(&self, column: &'a ColumnUi) -> Vec<&'a SourcePost> {
        let matched = termy_x::trends_desk::filter_posts(&column.view.posts, &self.filter_text);
        let mut rows = apply_filters(&matched, &self.pack.filters, Self::now_unix());
        if self.pack.sort_heat {
            let order = termy_x::trends_desk::sort_by_heat(&rows, Self::now_unix());
            rows.sort_by_key(|post| {
                order
                    .iter()
                    .position(|id| id == &post.id)
                    .unwrap_or(usize::MAX)
            });
        }
        rows
    }

    pub(crate) fn posts_for_takes(&self) -> Vec<termy_x::trends_desk::SourcePost> {
        let index = self
            .columns
            .iter()
            .position(|column| column.topic.id == self.settings.subject_id)
            .unwrap_or(0);
        let Some(column) = self.columns.get(index) else {
            return Vec::new();
        };
        self.visible_posts(column)
            .into_iter()
            .take(self.card_window)
            .filter(|post| {
                !self.takes.get(&post.id).is_some_and(|take| {
                    !take.text.trim().is_empty()
                        && take.source != "offline"
                        && take.source != "none"
                })
            })
            .cloned()
            .collect()
    }

    pub(crate) fn hud(&self) -> String {
        let sent = self
            .pack
            .receipts
            .iter()
            .filter(|mark| mark.status == "confirmed")
            .count();
        let drafts =
            termy_x::drafts::load_drafts(&termy_x::trends_desk::drafts_file(&self.data_root)).len();
        let line = hud_line(
            sent,
            drafts,
            &self.settings.take_provider,
            self.pack.last_refresh,
            termy_x::trends_desk::local_offset_secs(Self::now_unix()),
        );
        let offset = termy_x::trends_desk::local_offset_secs(Self::now_unix());
        let next = termy_x::trends_desk::next_refresh_text(
            self.pack.last_refresh,
            Self::now_unix(),
            self.pack.refresh_on,
            self.pack.refresh_minutes,
            offset,
        );
        format!("{line} · {next}")
    }

    pub(crate) fn heat_for(&self, post: &SourcePost) -> Option<String> {
        termy_x::trends_desk::heat_badge(post, Self::now_unix())
    }

    pub(crate) fn badge_for(&self, id: &str) -> Option<String> {
        replied_badge(&self.pack.receipts, id)
    }

    pub(crate) fn new_count(&self, topic: &str, ids: &[String]) -> usize {
        match self.pack.seen.get(topic) {
            Some(seen) if !seen.is_empty() => new_ids(seen, ids).len(),
            _ => 0,
        }
    }

    pub(crate) fn toggle_star(&mut self, id: &str) {
        if let Some(index) = self.pack.hits.iter().position(|hit| hit.id == id) {
            self.pack.hits.remove(index);
        } else if let Some(post) = self
            .columns
            .iter()
            .flat_map(|column| column.view.posts.iter())
            .find(|post| post.id == id)
        {
            self.pack.hits.push(termy_x::trends_desk::HitItem {
                id: post.id.clone(),
                url: post.url.clone(),
                author: post.author.clone(),
                text: post.text.clone(),
                note: String::new(),
            });
        }
        self.save_pack_file();
    }

    pub(crate) fn pack_key(
        &mut self,
        key: &str,
        ctrl: bool,
        shift: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.palette {
            if key == "escape" || (ctrl && key == "k") {
                self.palette = false;
            } else if key == "backspace" {
                self.palette_query.pop();
            }
            cx.notify();
            return true;
        }
        if self.surface != termy_x::trends_desk::Surface::Columns || self.field.is_some() {
            return false;
        }
        match desk_key(key, ctrl, shift) {
            DeskKey::Palette => self.palette = true,
            DeskKey::Page(delta) => self.turn_subject(delta),
            DeskKey::Select(delta) => {
                let ids = self.page_ids();
                self.selected = move_selection(&ids, self.selected.as_deref(), delta);
            }
            DeskKey::Draft => {
                if let Some(id) = self.selected_take() {
                    self.draft_take_for(id, cx);
                }
            }
            DeskKey::Reply => {
                if let Some(id) = self.selected_take() {
                    self.open_popout(&id);
                }
            }
            DeskKey::Open => {
                if let Some(url) = self.selected_url() {
                    open_status(&url);
                }
            }
            DeskKey::Star => {
                if let Some(id) = self.selected_take() {
                    self.toggle_star(&id);
                }
            }
            DeskKey::Unbound => return false,
        }
        cx.notify();
        true
    }

    pub(crate) fn selected_take(&self) -> Option<String> {
        self.selected
            .as_deref()
            .and_then(|id| id.strip_prefix("take:"))
            .map(str::to_string)
    }

    fn selected_url(&self) -> Option<String> {
        let id = self.selected_take()?;
        self.columns
            .iter()
            .flat_map(|column| column.view.posts.iter())
            .find(|post| post.id == id)
            .map(|post| post.url.clone())
    }

    pub(crate) fn note_key(&mut self, keystroke: &Keystroke) {
        if !self.palette {
            return;
        }
        if let Some(ch) = keystroke.key_char.as_deref()
            && !keystroke.modifiers.control
            && !keystroke.modifiers.platform
            && !ch.is_empty()
        {
            self.palette_query.push_str(ch);
        }
    }

    pub(crate) fn palette_view(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let query = self.palette_query.to_lowercase();
        let mut list = div()
            .id("desk-palette")
            .absolute()
            .top(px(64.0))
            .left(px(80.0))
            .right(px(80.0))
            .p(px(12.0))
            .rounded(px(12.0))
            .bg(rgb(0x10141c))
            .border_1()
            .border_color(rgb(0xffd60a))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.stop_propagation()),
            )
            .child(format!(
                "Command · {}",
                if query.is_empty() {
                    "type to filter".into()
                } else {
                    self.palette_query.clone()
                }
            ));
        let mut rows = vec![
            ("hits".into(), "Hit List".into()),
            ("receipts".into(), "Receipts".into()),
            ("heat".into(), "Sort by heat".into()),
            ("termy".into(), "Termy areas".into()),
            ("free".into(), "Provider Free".into()),
            ("treg".into(), "Provider Treg".into()),
        ];
        for column in &self.columns {
            rows.push((
                format!("subject:{}", column.topic.id),
                column.topic.name.clone(),
            ));
        }
        for tone in TakeTone::all() {
            rows.push((
                format!("tone:{}", tone.label()),
                format!("Tone {}", tone.label()),
            ));
        }
        for (id, label) in rows {
            if !query.is_empty() && !label.to_lowercase().contains(&query) {
                continue;
            }
            let action = id.clone();
            list = list.child(button(
                format!("desk-pal-{id}"),
                label,
                false,
                cx.listener(move |this, _, _, cx| {
                    this.run_palette(&action);
                    cx.stop_propagation();
                    cx.notify();
                }),
            ));
        }
        list.into_any_element()
    }

    pub(crate) fn run_palette(&mut self, action: &str) {
        self.palette = false;
        self.palette_query.clear();
        if let Some(id) = action.strip_prefix("subject:") {
            self.settings.subject_id = id.to_string();
            self.board = "subject".into();
            self.save_pack_file();
            if self.writable {
                let _ = self.store.save_settings(&self.settings);
            }
        } else if let Some(name) = action.strip_prefix("tone:") {
            self.pack.tone = name.to_lowercase();
            self.save_pack_file();
        } else {
            match action {
                "hits" => self.board = "hits".into(),
                "receipts" => self.board = "receipts".into(),
                "termy" => self.board = "termy".into(),
                "heat" => {
                    self.pack.sort_heat = !self.pack.sort_heat;
                    self.save_pack_file();
                }
                "free" => self.remember_provider("free"),
                "treg" => self.remember_provider("treg"),
                "hermes" => self.remember_provider("hermes"),
                "grok" => self.remember_provider("grok"),
                _ => {}
            }
        }
    }

    pub(crate) fn special_page(&self, cx: &mut Context<Self>) -> Option<gpui_kit::AnyElement> {
        if self.board == "hits" {
            let mut list = div().flex().flex_col().gap(px(8.0)).child("Hit List");
            for hit in &self.pack.hits {
                list = list.child(format!("★ @{} · {} · {}", hit.author, hit.note, hit.url));
            }
            if self.pack.hits.is_empty() {
                list = list.child("No starred cards yet.");
            }
            list = list.child(button(
                "desk-board-back",
                "Back",
                false,
                cx.listener(|this, _, _, cx| {
                    this.board = "subject".into();
                    cx.notify();
                }),
            ));
            return Some(list.into_any_element());
        }
        if self.board == "termy" {
            let mut list = div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child("Termy areas use the existing panel, writing only in the Termy XL data folder. Sends still need a click. Image quotes stay unsupported.");
            let areas = [
                (crate::x_panel::XSection::Timeline, "Timeline"),
                (crate::x_panel::XSection::Search, "Search"),
                (crate::x_panel::XSection::Trends, "Trends"),
                (crate::x_panel::XSection::Compose, "Compose"),
                (crate::x_panel::XSection::ReplyGuy, "Reply"),
                (crate::x_panel::XSection::Followers, "Follow"),
                (crate::x_panel::XSection::Queue, "Receipts"),
            ];
            for (section, label) in areas {
                list = list.child(button(
                    format!("desk-x-{label}"),
                    label,
                    false,
                    cx.listener(move |this, _, _, cx| {
                        this.open_x_section(section, cx);
                        cx.stop_propagation();
                        cx.notify();
                    }),
                ));
            }
            list = list.child(button(
                "desk-board-back-3",
                "Back",
                false,
                cx.listener(|this, _, _, cx| {
                    this.board = "subject".into();
                    cx.notify();
                }),
            ));
            return Some(list.into_any_element());
        }
        if self.board == "receipts" {
            let mut list = div().flex().flex_col().gap(px(8.0)).child("Receipts");
            let now = Self::now_unix();
            let offset = termy_x::trends_desk::local_offset_secs(now);
            for mark in &self.pack.receipts {
                let when = termy_x::trends_desk::format_clock(mark.at_unix, offset);
                list = list.child(format!("{} · {when} · {}", mark.status, mark.url));
            }
            if self.pack.receipts.is_empty() {
                list = list.child("No replies sent from Termy XL yet.");
            }
            list = list.child(button(
                "desk-board-back-2",
                "Back",
                false,
                cx.listener(|this, _, _, cx| {
                    this.board = "subject".into();
                    cx.notify();
                }),
            ));
            return Some(list.into_any_element());
        }
        None
    }

    pub(crate) fn copy_text(&self, text: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
    }

    pub(crate) fn maybe_refresh(&mut self, cx: &mut Context<Self>) {
        if self.options.fixture {
            return;
        }
        let now = Self::now_unix();
        if !termy_x::trends_desk::should_auto_refresh(
            self.pack.last_refresh,
            now,
            self.pack.refresh_on,
            self.pack.refresh_minutes,
            self.busy,
            self.free_halted,
        ) {
            return;
        }
        self.pack.last_refresh = Some(now);
        self.save_pack_file();
        let id = self
            .columns
            .iter()
            .find(|column| column.topic.id == self.settings.subject_id)
            .map(|column| column.topic.id.clone())
            .or_else(|| self.columns.first().map(|column| column.topic.id.clone()));
        if let Some(id) = id {
            self.start_run(std::slice::from_ref(&id), cx);
        }
    }
    pub(crate) fn open_x_section(
        &mut self,
        section: crate::x_panel::XSection,
        cx: &mut Context<Self>,
    ) {
        let root = self.data_root.clone();
        if self.x_panel.is_none() {
            self.x_panel = Some(cx.new(|cx| crate::x_panel::XPanel::new_in(cx, false, root)));
        }
        if let Some(panel) = &self.x_panel {
            panel.update(cx, |panel, cx| panel.open_section(section, cx));
        }
        self.board = "xpanel".into();
    }

    pub(crate) fn columns_view(
        &self,
        width: f32,
        _per_row: usize,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        if self.board == "xpanel" {
            if let Some(panel) = &self.x_panel {
                return div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(button(
                        "desk-x-back",
                        "Back to XL",
                        false,
                        cx.listener(|this, _, _, cx| {
                            this.board = "subject".into();
                            cx.notify();
                        }),
                    ))
                    .child(div().flex_1().min_h(px(0.0)).child(panel.clone()))
                    .into_any_element();
            }
        }
        let index = self
            .columns
            .iter()
            .position(|column| column.topic.id == self.settings.subject_id)
            .unwrap_or(0);
        if let Some(page) = self.special_page(cx) {
            return page;
        }
        let Some(column) = self.columns.get(index) else {
            return div().child("No subjects yet.").into_any_element();
        };
        let subject_key = format!("subject:{}", column.topic.id);
        let subject_selected = self.selected.as_deref() == Some(subject_key.as_str());
        let all_visible = self.visible_posts(column);
        let hidden = all_visible.len().saturating_sub(self.card_window);
        let visible: Vec<_> = all_visible.into_iter().take(self.card_window).collect();
        let fresh = self.new_count(
            &column.topic.id,
            &visible
                .iter()
                .map(|post| post.id.clone())
                .collect::<Vec<_>>(),
        );
        let label = format!(
            "{} · {fresh} new",
            page_label(index, self.columns.len(), &column.topic.name)
        );
        let ids: Vec<String> = visible.iter().map(|post| post.id.clone()).collect();
        let frames = layout_takes(width, &ids);
        let page = div()
            .size_full()
            .flex()
            .flex_col()
            .p(px(WINDOW_PAD))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.selected = None;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .id("desk-subject-page")
                    .flex_1()
                    .min_h(px(0.0))
                    .w_full()
                    .rounded(px(16.0))
                    .border_1()
                    .border_color(if subject_selected {
                        rgb(0xffe56a)
                    } else {
                        rgb(0xffd60a)
                    })
                    .bg(if subject_selected {
                        rgb(0x2a2410)
                    } else {
                        rgb(0x12100a)
                    })
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .p(px(12.0))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.selected = None;
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(button(
                                "desk-page-prev",
                                "Prev",
                                false,
                                cx.listener(|this, _, _, cx| {
                                    this.turn_subject(-1);
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            ))
                            .child(div().text_size(px(16.0)).child(label))
                            .child(button(
                                "desk-page-next",
                                "Next",
                                false,
                                cx.listener(|this, _, _, cx| {
                                    this.turn_subject(1);
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            )),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0x8e8e93))
                            .child(column.topic.desc.clone()),
                    )
                    .child({
                        let topic_id = column.topic.id.clone();
                        let mut list = div()
                            .id("desk-take-list")
                            .flex_1()
                            .min_h(px(0.0))
                            .overflow_y_scroll()
                            .flex()
                            .flex_row()
                            .flex_wrap()
                            .items_start()
                            .gap(px(12.0));
                        if column.view.phase == Phase::Loading && visible.is_empty() {
                            list = list.child(status_row(
                                "Loading takes…",
                                refresh_button(topic_id.clone(), cx),
                            ));
                        } else if column.view.phase == Phase::Error && visible.is_empty() {
                            list = list.child(status_row(
                                &column.view.banner,
                                refresh_button(topic_id.clone(), cx),
                            ));
                        } else if visible.is_empty() {
                            list = list.child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(8.0))
                                    .h(px(36.0))
                                    .child("Not run yet.")
                                    .child(refresh_button(topic_id.clone(), cx)),
                            );
                        }
                        if hidden > 0 {
                            list = list.child(button(
                                "desk-show-more",
                                format!("Show {hidden} more"),
                                false,
                                cx.listener(|this, _, _, cx| {
                                    this.card_window = this.card_window.saturating_add(12);
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            ));
                        }
                        for (post, frame) in visible.iter().zip(frames.iter()) {
                            list = list.child(
                                div()
                                    .w(px(frame.width))
                                    .min_w(px(0.0))
                                    .child(self.tweet_card(post, column, frame.width, cx)),
                            );
                        }
                        list
                    }),
            );
        let mut rows: Vec<gpui_kit::AnyElement> = vec![page.into_any_element()];
        if self.session.state == SessionState::Missing {
            rows.insert(
                0,
                div()
                    .mb(px(12.0))
                    .p(px(12.0))
                    .rounded(px(12.0))
                    .bg(rgb(0x3a1d1d))
                    .text_size(px(13.0))
                    .child(self.session.detail.clone())
                    .into_any_element(),
            );
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .children(rows)
            .into_any_element()
    }
}

fn status_row(label: &str, action: gpui_kit::AnyElement) -> gpui_kit::AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .h(px(36.0))
        .child(label.to_string())
        .child(action)
        .into_any_element()
}

fn refresh_button(topic_id: String, cx: &mut Context<TrendsDesk>) -> gpui_kit::AnyElement {
    button(
        format!("desk-refresh-{topic_id}"),
        "Refresh",
        false,
        cx.listener(move |this, _, _, cx| {
            let pressed = termy_x::trends_desk::press_refresh(
                crate::trends_desk::TrendsDesk::now_unix(),
                this.busy,
                Some(topic_id.as_str()),
                this.pack.last_refresh,
            );
            if pressed.started {
                this.pack.last_refresh = pressed.last_refresh;
                this.save_pack_file();
                this.start_run(std::slice::from_ref(&topic_id), cx);
            }
            cx.stop_propagation();
        }),
    )
}
