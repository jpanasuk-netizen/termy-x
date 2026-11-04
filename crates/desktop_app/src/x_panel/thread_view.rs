//! Thread overlay: click a tweet, pin it, nest replies, Back restores scroll.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui_kit::{
    Animation, AnimationExt, Context, InteractiveElement, IntoElement, MouseButton, ParentElement,
    SharedString, Styled, div, point, px, rgb,
};

use termy_x::bulk::{
    BulkKind, BulkPlan, BulkStop, BulkTarget, DelayRng, WallRng, cap_stop_note, plan_bulk,
};
use termy_x::model::Post;
use termy_x::providers::{MockProvider, XProvider};
use termy_x::publish::Publisher;
use termy_x::sort::ResultSort;
use termy_x::thread::{build_reply_tree, walk_thread};

use super::*;

const THREAD_MOTION_MS: u64 = 180;

#[derive(Clone)]
pub(super) struct ThreadView {
    pub focus_id: String,
    pub root: Post,
    pub load: ThreadLoad,
    pub sort: ResultSort,
    pub seq: u64,
}

#[derive(Clone)]
pub(super) enum ThreadLoad {
    Loading,
    Ready { replies: Vec<Post> },
    Error(String),
}

pub(super) enum BulkUi {
    Confirm(BulkPlan),
    Running {
        plan: BulkPlan,
        done: usize,
        stop: Arc<AtomicBool>,
    },
    Note(String),
}

pub(super) fn restore_scroll_y(saved: f32, max_y: f32) -> f32 {
    saved.clamp(-max_y.max(0.0), 0.0)
}

fn thread_motion(
    element: impl Styled + IntoElement + 'static,
    leaving: bool,
) -> gpui_kit::AnyElement {
    let id = if leaving {
        "x-thread-leave"
    } else {
        "x-thread-enter"
    };
    element
        .with_animation(
            id,
            Animation::new(Duration::from_millis(THREAD_MOTION_MS))
                .with_easing(gpui_kit::ease_out_quint()),
            move |element, delta| {
                if leaving {
                    element.opacity(1.0 - delta)
                } else {
                    element.opacity(delta).mt(px(8.0 * (1.0 - delta)))
                }
            },
        )
        .into_any_element()
}

impl XPanel {
    fn lookup_post(&self, id: &str) -> Option<Post> {
        let from_lists = self
            .posts
            .iter()
            .chain(self.following_posts.iter())
            .chain(self.search_posts.iter())
            .chain(self.mentions_posts.iter())
            .chain(self.inbox_posts.iter())
            .find(|post| post.id == id || post.engage_target().id == id)
            .cloned();
        if from_lists.is_some() {
            return from_lists;
        }
        if let Some(profile) = &self.profile {
            let found = profile
                .pinned
                .iter()
                .chain(profile.posts.iter())
                .find(|post| post.id == id || post.engage_target().id == id)
                .cloned();
            if found.is_some() {
                return found;
            }
        }
        if let Some(thread) = &self.thread {
            if thread.root.id == id || thread.root.engage_target().id == id {
                return Some(thread.root.clone());
            }
            if let ThreadLoad::Ready { replies, .. } = &thread.load {
                return replies
                    .iter()
                    .find(|post| post.id == id || post.engage_target().id == id)
                    .cloned();
            }
        }
        None
    }

    /// Open the thread overlay on the clicked tweet. Fetch runs on the
    /// background executor; the clicked card is pinned immediately.
    pub(super) fn open_thread(&mut self, id: &str, author: &str, cx: &mut Context<Self>) {
        if self
            .thread
            .as_ref()
            .is_some_and(|thread| thread.focus_id == id && !self.thread_leaving)
        {
            return;
        }
        self.autoscroll = None;
        self.close_viewer();
        if self.quote_post_id.is_some() {
            self.close_quote();
        }
        if self.reply_post_id.is_some() {
            self.close_reply();
        }
        let display = match self.lookup_post(id) {
            Some(post) => post.engage_target().clone(),
            None => Post {
                id: id.to_string(),
                url: format!("https://x.com/{author}/status/{id}"),
                author_handle: author.to_string(),
                author_name: author.to_string(),
                text: String::new(),
                ..Post::default()
            },
        };
        let display_id = display.id.clone();
        if self.thread.is_none() {
            self.thread_return_scroll = Some(f32::from(self.list_scroll.offset().y));
        }
        self.thread_leaving = false;
        self.thread_seq = self.thread_seq.wrapping_add(1);
        let seq = self.thread_seq;
        self.thread = Some(ThreadView {
            focus_id: display_id.clone(),
            root: display,
            load: ThreadLoad::Loading,
            sort: self.list_sort,
            seq,
        });
        self.list_scroll.set_offset(point(px(0.0), px(0.0)));
        self.status = format!("thread \u{b7} @{author} \u{b7} loading replies\u{2026}");
        let live = self.live;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    if live {
                        XPanel::service_free().thread(&display_id)
                    } else {
                        MockProvider.thread(&display_id)
                    }
                })
                .await;
            let _ = this.update(cx, |panel, cx| {
                {
                    let Some(thread) = panel.thread.as_mut() else {
                        return;
                    };
                    if thread.seq != seq {
                        return;
                    }
                    match result {
                        Ok(served) => {
                            thread.root = served.value.root;
                            thread.focus_id = thread.root.id.clone();
                            let replies = served.value.replies;
                            let n = replies.len();
                            let handle = thread.root.author_handle.clone();
                            thread.load = ThreadLoad::Ready { replies };
                            panel.status = if n == 0 {
                                format!("thread \u{b7} @{handle} \u{b7} no replies")
                            } else {
                                format!(
                                    "thread \u{b7} @{handle} \u{b7} {n} repl{}",
                                    if n == 1 { "y" } else { "ies" }
                                )
                            };
                        }
                        Err(error) => {
                            thread.load = ThreadLoad::Error(error.clone());
                            panel.status = format!("thread \u{b7} {error}");
                        }
                    }
                }
                panel.seed_thread_flags();
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn close_thread(&mut self, restore_scroll: bool, cx: &mut Context<Self>) {
        if self.thread.is_none() {
            return;
        }
        if self.thread_leaving {
            self.finish_close_thread(restore_scroll);
            cx.notify();
            return;
        }
        self.thread_leaving = true;
        cx.notify();
        let restore = restore_scroll;
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            executor
                .timer(Duration::from_millis(THREAD_MOTION_MS))
                .await;
            let _ = this.update(cx, |panel, cx| {
                panel.finish_close_thread(restore);
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_close_thread(&mut self, restore_scroll: bool) {
        if let Some(BulkUi::Running { stop, .. }) = &self.bulk {
            stop.store(true, Ordering::SeqCst);
        }
        self.bulk = None;
        self.thread = None;
        self.thread_leaving = false;
        if restore_scroll && let Some(saved) = self.thread_return_scroll.take() {
            let max_y: f32 = self.list_scroll.max_offset().y.into();
            let y = restore_scroll_y(saved, max_y);
            self.list_scroll.set_offset(point(px(0.0), px(y)));
        } else {
            self.thread_return_scroll = None;
        }
        self.status = format!("{} \u{b7} thread closed", self.section.label());
    }

    /// Drop the overlay immediately (tab / feed change). Scroll belongs
    /// to the destination surface, so it is not restored here.
    pub(super) fn abandon_thread(&mut self) {
        self.thread = None;
        self.thread_leaving = false;
        self.thread_return_scroll = None;
        if let Some(BulkUi::Running { stop, .. }) = &self.bulk {
            stop.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        self.bulk = None;
    }

    /// Preview-only: pin a mock thread on the first fixture tweet so
    /// screenshots can land on Popular / Newest / reply / bulk without a
    /// live OpenCLI call.
    pub(super) fn show_mock_thread(&mut self, sort: ResultSort) {
        self.load_mock_dashboard();
        let post = match self.posts.first() {
            Some(p) => p.engage_target().clone(),
            None => MockProvider
                .thread("1001")
                .map(|s| s.value.root)
                .unwrap_or_default(),
        };
        let served = MockProvider.thread(&post.id).expect("mock thread fixture");
        self.thread_leaving = false;
        self.thread_seq = self.thread_seq.wrapping_add(1);
        self.list_sort = sort;
        self.thread = Some(ThreadView {
            focus_id: served.value.root.id.clone(),
            root: served.value.root,
            load: ThreadLoad::Ready {
                replies: served.value.replies,
            },
            sort,
            seq: self.thread_seq,
        });
        self.seed_thread_flags();
        self.status = format!(
            "thread \u{b7} @{} \u{b7} mock",
            self.thread
                .as_ref()
                .map(|t| t.root.author_handle.clone())
                .unwrap_or_default()
        );
    }

    pub(super) fn dismiss_thread(&mut self, cx: &mut Context<Self>) {
        if self.reply_post_id.is_some() {
            self.close_reply();
            cx.notify();
            return;
        }
        if self.quote_post_id.is_some() {
            self.close_quote();
            cx.notify();
            return;
        }
        self.close_thread(true, cx);
    }

    fn seed_thread_flags(&mut self) {
        let (liked, follows) = {
            let Some(thread) = &self.thread else {
                return;
            };
            let mut liked = Vec::new();
            let mut follows = Vec::new();
            let mut consider = |post: &Post| {
                if post.viewer_liked == Some(true) {
                    liked.push(post.id.clone());
                }
                if post.viewer_follows_author == Some(true) {
                    follows.push(post.author_handle.clone());
                }
            };
            consider(&thread.root);
            if let ThreadLoad::Ready { replies } = &thread.load {
                for post in replies {
                    consider(post);
                }
            }
            (liked, follows)
        };
        for id in liked {
            if !self.liked_ids.iter().any(|p| p == &id) {
                self.liked_ids.push(id);
            }
        }
        for handle in follows {
            self.follow_overrides.entry(handle).or_insert(true);
        }
    }

    pub(super) fn thread_replies_for_bulk(&self) -> Vec<Post> {
        let Some(thread) = &self.thread else {
            return Vec::new();
        };
        let ThreadLoad::Ready { replies } = &thread.load else {
            return Vec::new();
        };
        replies
            .iter()
            .map(|post| {
                let mut post = post.clone();
                if self.liked_ids.iter().any(|id| id == &post.id) {
                    post.viewer_liked = Some(true);
                }
                if let Some(flag) = self.follow_overrides.iter().find_map(|(h, v)| {
                    h.trim_start_matches('@')
                        .eq_ignore_ascii_case(post.author_handle.trim_start_matches('@'))
                        .then_some(*v)
                }) {
                    post.viewer_follows_author = Some(flag);
                }
                post
            })
            .collect()
    }

    pub(super) fn own_handle(&self) -> Option<&str> {
        match &self.connection {
            ConnState::Connected { handle, .. } => Some(handle.as_str()),
            _ => None,
        }
    }

    pub(super) fn arm_bulk(&mut self, kind: BulkKind, cx: &mut Context<Self>) {
        if matches!(self.bulk, Some(BulkUi::Running { .. })) {
            return;
        }
        let replies = self.thread_replies_for_bulk();
        let plan = plan_bulk(kind, &replies, self.own_handle(), self.bulk_cap);
        if plan.action_count() == 0 {
            self.notice =
                Some("Nothing to do \u{2014} already liked, already followed, or skipped.".into());
            self.bulk = None;
            cx.notify();
            return;
        }
        self.bulk = Some(BulkUi::Confirm(plan));
        cx.notify();
    }

    fn cancel_bulk(&mut self, cx: &mut Context<Self>) {
        if let Some(BulkUi::Running { stop, .. }) = &self.bulk {
            stop.store(true, Ordering::SeqCst);
        }
        self.bulk = None;
        cx.notify();
    }

    fn confirm_bulk(&mut self, cx: &mut Context<Self>) {
        let Some(BulkUi::Confirm(plan)) = self.bulk.take() else {
            return;
        };
        self.start_bulk_run(plan, cx);
    }

    fn start_bulk_run(&mut self, plan: BulkPlan, cx: &mut Context<Self>) {
        let stop = Arc::new(AtomicBool::new(false));
        self.bulk_seq = self.bulk_seq.wrapping_add(1);
        let seq = self.bulk_seq;
        let total = plan.action_count();
        self.status = format!("bulk \u{b7} 0/{total}");
        self.bulk = Some(BulkUi::Running {
            plan: plan.clone(),
            done: 0,
            stop: stop.clone(),
        });
        cx.notify();
        let live = self.live;
        let mock = self.mock_writes.clone();
        let cap = plan.cap;
        let capped = plan.capped;
        let kind = plan.kind;
        let targets = plan.targets;
        cx.spawn(async move |this, cx| {
            let mut done = 0usize;
            let mut rng = WallRng;
            for (index, target) in targets.iter().enumerate() {
                if stop.load(Ordering::SeqCst) {
                    let _ = this.update(cx, |panel, cx| {
                        if panel.bulk_seq != seq {
                            return;
                        }
                        panel.finish_bulk(BulkStop::Stopped, done, total, cap, cx);
                    });
                    return;
                }
                if index > 0 {
                    let ms = rng.delay_ms();
                    cx.background_executor()
                        .timer(Duration::from_millis(ms))
                        .await;
                    if stop.load(Ordering::SeqCst) {
                        let _ = this.update(cx, |panel, cx| {
                            if panel.bulk_seq != seq {
                                return;
                            }
                            panel.finish_bulk(BulkStop::Stopped, done, total, cap, cx);
                        });
                        return;
                    }
                }
                let target = target.clone();
                let mock = mock.clone();
                let write_target = target.clone();
                let result = cx
                    .background_executor()
                    .spawn(async move { bulk_write(live, &mock, kind, &write_target) })
                    .await;
                match result {
                    Ok(_) => {
                        done += 1;
                        let _ = this.update(cx, |panel, cx| {
                            if panel.bulk_seq != seq {
                                return;
                            }
                            panel.apply_bulk_success(kind, &target);
                            if let Some(BulkUi::Running { done: d, .. }) = &mut panel.bulk {
                                *d = done;
                            }
                            panel.status = format!("bulk \u{b7} {done}/{total}");
                            cx.notify();
                        });
                    }
                    Err(error) => {
                        let _ = this.update(cx, |panel, cx| {
                            if panel.bulk_seq != seq {
                                return;
                            }
                            panel.finish_bulk(
                                BulkStop::Error(termy_x::publish::humanize_error(&error)),
                                done,
                                total,
                                cap,
                                cx,
                            );
                        });
                        return;
                    }
                }
            }
            let stop_kind = if capped {
                BulkStop::Cap
            } else {
                BulkStop::Completed
            };
            let _ = this.update(cx, |panel, cx| {
                if panel.bulk_seq != seq {
                    return;
                }
                panel.finish_bulk(stop_kind, done, total, cap, cx);
            });
        })
        .detach();
    }

    fn apply_bulk_success(&mut self, kind: BulkKind, target: &BulkTarget) {
        match kind {
            BulkKind::LikeReplies => {
                self.set_engage_state(&target.post_id, true, true);
            }
            BulkKind::FollowRepliers => {
                self.follow_overrides
                    .insert(target.author_handle.clone(), true);
                self.apply_follow_to_thread(&target.author_handle, true);
            }
        }
    }

    fn finish_bulk(
        &mut self,
        stop: BulkStop,
        done: usize,
        total: usize,
        cap: usize,
        cx: &mut Context<Self>,
    ) {
        let note = match &stop {
            BulkStop::Completed => format!("Done ({done}/{total})."),
            BulkStop::Stopped => format!("Stopped ({done}/{total})."),
            BulkStop::Cap => cap_stop_note(cap),
            BulkStop::Error(error) => {
                format!("Stopped on error ({done}/{total}): {error}")
            }
        };
        self.status = format!("bulk \u{b7} {note}");
        self.notice = Some(note.clone());
        self.bulk = Some(BulkUi::Note(note));
        cx.notify();
    }

    pub(super) fn bulk_overlay(
        &self,
        dialog_width: f32,
        cx: &mut Context<Self>,
    ) -> Option<gpui_kit::AnyElement> {
        let Some(BulkUi::Confirm(plan)) = &self.bulk else {
            return None;
        };
        let summary = plan.confirm_summary();
        let title = match plan.kind {
            BulkKind::LikeReplies => "Like all replies",
            BulkKind::FollowRepliers => "Follow all repliers",
        };
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(tint(0x03060c, 170))
                .child(
                    div()
                        .id("x-bulk-confirm")
                        .w(px(dialog_width))
                        .max_w(px(640.0))
                        .p(px(16.0))
                        .rounded(px(14.0))
                        .bg(rgb(0x101624))
                        .border_1()
                        .border_color(rgb(0xff4fa3))
                        .flex()
                        .flex_col()
                        .gap(px(10.0))
                        .child(
                            div()
                                .text_size(px(16.0))
                                .text_color(rgb(0xffd0ea))
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(px(13.0))
                                .text_color(rgb(0xe7eefc))
                                .child(summary),
                        )
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(rgb(0x93a4c3))
                                .child(format!(
                                    "Cap {} \u{b7} 3\u{2013}8s between actions \u{b7} Stop any time.",
                                    plan.cap
                                )),
                        )
                        .child(
                            div()
                                .flex()
                                .gap(px(8.0))
                                .child(self.thread_chip(
                                    "x-bulk-cancel",
                                    "Cancel",
                                    false,
                                    cx,
                                    |this, cx| {
                                        this.cancel_bulk(cx);
                                    },
                                ))
                                .child(self.thread_chip(
                                    "x-bulk-go",
                                    "Confirm",
                                    true,
                                    cx,
                                    |this, cx| {
                                        this.confirm_bulk(cx);
                                    },
                                )),
                        ),
                )
                .into_any_element(),
        )
    }

    fn thread_chip(
        &self,
        id: &'static str,
        label: &'static str,
        active: bool,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut XPanel, &mut Context<XPanel>) + Clone + 'static,
    ) -> gpui_kit::AnyElement {
        div()
            .id(SharedString::from(id))
            .px(px(10.0))
            .py(px(6.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(if active {
                rgb(0xff4fa3)
            } else {
                tint(0x7af6ff, 55)
            })
            .bg(if active { rgb(0x2a0f3a) } else { rgb(0x1636c9) })
            .text_size(px(12.0))
            .text_color(if active { rgb(0xffd0ea) } else { rgb(0xf8fbff) })
            .cursor_pointer()
            .hover(|style| {
                style
                    .bg(rgb(0x1f4fff))
                    .border_color(rgb(0x7af6ff))
                    .text_color(rgb(0xe9fdff))
            })
            .child(label)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    on_click(this, cx);
                    cx.stop_propagation();
                }),
            )
            .into_any_element()
    }

    pub(super) fn thread_rows(&self, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let Some(thread) = &self.thread else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        let handle = thread.root.author_handle.clone();
        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .flex_wrap()
            .child(
                div()
                    .id("x-thread-back")
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(tint(0x7af6ff, 55))
                    .bg(rgb(0x1636c9))
                    .text_size(px(12.0))
                    .text_color(rgb(0xf8fbff))
                    .cursor_pointer()
                    .hover(|style| {
                        style
                            .bg(rgb(0x1f4fff))
                            .border_color(rgb(0x7af6ff))
                            .text_color(rgb(0xe9fdff))
                    })
                    .child("Back")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.close_thread(true, cx);
                            cx.stop_propagation();
                        }),
                    ),
            )
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .text_color(rgb(0x7af6ff))
                    .child(format!("Thread \u{b7} @{handle}")),
            );
        let has_replies =
            matches!(&thread.load, ThreadLoad::Ready { replies } if !replies.is_empty());
        let bulk_running = matches!(self.bulk, Some(BulkUi::Running { .. }));
        let header = if has_replies && !bulk_running {
            header
                .child(self.thread_chip(
                    "x-bulk-like-all",
                    "Like all replies",
                    false,
                    cx,
                    |this, cx| this.arm_bulk(BulkKind::LikeReplies, cx),
                ))
                .child(self.thread_chip(
                    "x-bulk-follow-all",
                    "Follow all repliers",
                    false,
                    cx,
                    |this, cx| this.arm_bulk(BulkKind::FollowRepliers, cx),
                ))
        } else {
            header
        };
        let header = match &self.bulk {
            Some(BulkUi::Running { done, plan, .. }) => header
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(rgb(0xffd0ea))
                        .child(format!("{done}/{}", plan.action_count())),
                )
                .child(
                    self.thread_chip("x-bulk-stop", "Stop", true, cx, |this, cx| {
                        this.cancel_bulk(cx);
                    }),
                ),
            Some(BulkUi::Note(note)) => header.child(
                div()
                    .text_size(px(12.0))
                    .text_color(rgb(0xffd0ea))
                    .child(note.clone()),
            ),
            _ => header,
        };
        rows.push(header.into_any_element());
        rows.push(self.tweet_card_scoped(&thread.root, "thread-focus", cx));
        match &thread.load {
            ThreadLoad::Loading => {
                rows.push(empty_state(
                    "Loading replies...",
                    "Using your free OpenCLI session. Nothing burns X API credits.",
                ));
            }
            ThreadLoad::Error(error) => {
                rows.push(empty_state("Could not load this thread", error.clone()));
            }
            ThreadLoad::Ready { replies } if replies.is_empty() => {
                rows.push(empty_state(
                    "No replies yet",
                    "This tweet has no replies in the session payload.",
                ));
            }
            ThreadLoad::Ready { replies } => {
                let tree = build_reply_tree(&thread.focus_id, replies, thread.sort);
                for node in walk_thread(&tree) {
                    let indent = (node.display_depth() as f32) * 16.0;
                    rows.push(
                        div()
                            .pl(px(indent))
                            .child(self.tweet_card_scoped(&node.post, "thread", cx))
                            .into_any_element(),
                    );
                }
            }
        }
        vec![thread_motion(
            div().flex().flex_col().gap(px(8.0)).children(rows),
            self.thread_leaving,
        )]
    }
}

fn bulk_write(
    live: bool,
    mock: &termy_x::bulk::MockWriteLog,
    kind: BulkKind,
    target: &BulkTarget,
) -> Result<String, String> {
    if live {
        let service = XPanel::service_free();
        let publishers = service.publishers;
        let Some(publisher) = termy_x::publish::first_available(publishers.as_slice()) else {
            return Err("No in-app posting path found. Connect X first, then try again.".into());
        };
        match kind {
            BulkKind::LikeReplies => {
                publisher.engage(termy_x::publish::EngageAction::Like, &target.url, None, &[])
            }
            BulkKind::FollowRepliers => publisher.set_follow(&target.author_handle, true),
        }
    } else {
        match kind {
            BulkKind::LikeReplies => {
                mock.engage(termy_x::publish::EngageAction::Like, &target.url, None, &[])
            }
            BulkKind::FollowRepliers => mock.set_follow(&target.author_handle, true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::restore_scroll_y;

    #[test]
    fn restore_scroll_clamps_to_the_list_range() {
        assert_eq!(restore_scroll_y(-120.0, 400.0), -120.0);
        assert_eq!(restore_scroll_y(0.0, 400.0), 0.0);
        assert_eq!(restore_scroll_y(-900.0, 400.0), -400.0);
        assert_eq!(restore_scroll_y(40.0, 400.0), 0.0);
        assert_eq!(restore_scroll_y(-10.0, 0.0), 0.0);
    }
}
