//! Shared Popular/Newest chips, key `s`, and display ordering.

use gpui_kit::{
    Context, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString, Styled,
    div, px, rgb,
};

use termy_x::model::Post;
use termy_x::sort::{LIST_SORT_KEY, ResultSort, cmp_posts};

use super::*;

impl XPanel {
    pub(super) fn shows_list_sort(&self) -> bool {
        shows_sort_for(self.thread.is_some(), self.section)
    }

    pub(super) fn list_sort_bar(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let sort = self.list_sort;
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .flex_wrap()
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(rgb(0x7af6ff))
                    .child(format!("Sort \u{b7} {LIST_SORT_KEY}")),
            )
            .child(self.list_sort_chip(
                ResultSort::Popular.label(),
                sort == ResultSort::Popular,
                ResultSort::Popular,
                cx,
            ))
            .child(self.list_sort_chip(
                ResultSort::Newest.label(),
                sort == ResultSort::Newest,
                ResultSort::Newest,
                cx,
            ))
            .into_any_element()
    }

    fn list_sort_chip(
        &self,
        label: &'static str,
        active: bool,
        sort: ResultSort,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        div()
            .id(SharedString::from(format!("x-list-sort-{label}")))
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
                    this.set_list_sort(sort, cx);
                    cx.stop_propagation();
                }),
            )
            .into_any_element()
    }

    pub(super) fn set_list_sort(&mut self, sort: ResultSort, cx: &mut Context<Self>) {
        if self.list_sort == sort {
            return;
        }
        self.list_sort = sort;
        if let Some(thread) = self.thread.as_mut() {
            thread.sort = sort;
        }
        self.status = format!("{} \u{b7} {}", self.section.label(), sort.label());
        self.persist_list_sort();
        // Search's provider query is Top vs Latest. Re-ask once, on the
        // existing single-flight path. Other lists re-rank what is loaded.
        if self.live && self.section == XSection::Search && !self.searched_query.is_empty() {
            self.loading = true;
            self.schedule_fetch(XSection::Search, cx);
        }
        cx.notify();
    }

    pub(super) fn toggle_list_sort(&mut self, cx: &mut Context<Self>) {
        self.set_list_sort(self.list_sort.toggle(), cx);
    }

    fn persist_list_sort(&self) {
        if self.fill_window {
            return;
        }
        let mut config = XConfig::load(&termy_x::config::default_config_dir());
        config.list_sort = self.list_sort;
        let _ = config.save_list_sort();
    }

    pub(super) fn sorted_posts<'a>(&self, posts: &'a [Post]) -> Vec<&'a Post> {
        let mut refs: Vec<&Post> = posts.iter().collect();
        refs.sort_by(|left, right| cmp_posts(left, right, self.list_sort));
        refs
    }

    #[allow(dead_code)]
    pub(super) fn named_post_list(
        &self,
        title: &str,
        empty_detail: &str,
        posts: &[Post],
        cx: &mut Context<Self>,
    ) -> Vec<gpui_kit::AnyElement> {
        let mut rows = Vec::new();
        if posts.is_empty() {
            if self.loading {
                rows.push(empty_state(
                    &format!("Loading {title}..."),
                    "Using your free OpenCLI session. Nothing burns X API credits.",
                ));
            } else {
                rows.push(empty_state(&format!("No {title} yet"), empty_detail));
            }
            return rows;
        }
        if self.loading {
            rows.push(hint(format!("Refreshing {title}...")));
        } else {
            rows.push(hint(format!(
                "{title} \u{b7} {} shown \u{b7} {}",
                posts.len(),
                self.list_sort.label()
            )));
        }
        for post in self.sorted_posts(posts) {
            rows.push(self.tweet_card(post, cx));
        }
        rows
    }
}

pub(super) fn shows_sort_for(thread_open: bool, section: XSection) -> bool {
    thread_open
        || matches!(
            section,
            XSection::Timeline
                | XSection::Search
                | XSection::Mentions
                | XSection::Inbox
                | XSection::Lookup
                | XSection::Trends
                | XSection::Feeds
                | XSection::Stats
                | XSection::Analyze
        )
}

#[cfg(test)]
mod tests {
    use super::shows_sort_for;
    use crate::x_panel::XSection;
    use termy_x::sort::{LIST_SORT_KEY, ResultSort};

    #[test]
    fn sort_key_is_s_and_toggle_flips() {
        assert_eq!(LIST_SORT_KEY, "s");
        assert_eq!(ResultSort::Popular.toggle(), ResultSort::Newest);
        assert_eq!(ResultSort::Newest.toggle(), ResultSort::Popular);
    }

    #[test]
    fn list_modes_show_the_toggle() {
        assert!(shows_sort_for(true, XSection::Compose));
        assert!(shows_sort_for(false, XSection::Search));
        assert!(shows_sort_for(false, XSection::Mentions));
        assert!(shows_sort_for(false, XSection::Inbox));
        assert!(shows_sort_for(false, XSection::Lookup));
        assert!(shows_sort_for(false, XSection::Trends));
        assert!(shows_sort_for(false, XSection::Feeds));
        assert!(shows_sort_for(false, XSection::Stats));
        assert!(shows_sort_for(false, XSection::Analyze));
        assert!(!shows_sort_for(false, XSection::Compose));
        assert!(!shows_sort_for(false, XSection::Help));
        assert!(!shows_sort_for(false, XSection::Queue));
    }
}
