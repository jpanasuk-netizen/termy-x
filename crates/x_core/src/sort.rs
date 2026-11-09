//! Popular / Newest ordering for tweet lists and reply sibling groups.

use std::cmp::Ordering;

use crate::model::Post;

/// Key that toggles Popular / Newest in every list mode.
pub const LIST_SORT_KEY: &str = "s";

/// How tweet result lists and reply sibling groups are ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ResultSort {
    /// Likes descending, then engagement. Ties keep input order.
    #[default]
    Popular,
    /// `created_at` descending. Missing timestamps sort last.
    Newest,
}

impl ResultSort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Popular => "popular",
            Self::Newest => "newest",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Popular => "Popular",
            Self::Newest => "Newest",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "popular" | "top" | "hot" => Some(Self::Popular),
            "newest" | "new" | "recent" | "upcoming" | "live" => Some(Self::Newest),
            _ => None,
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            Self::Popular => Self::Newest,
            Self::Newest => Self::Popular,
        }
    }
}

/// Popular: likes, then engagement, then recency. Newest: `created_at` desc.
pub fn cmp_posts(left: &Post, right: &Post, sort: ResultSort) -> Ordering {
    match sort {
        ResultSort::Popular => right
            .likes
            .cmp(&left.likes)
            .then(right.engagement().cmp(&left.engagement()))
            .then(cmp_created_desc(left, right)),
        ResultSort::Newest => cmp_created_desc(left, right),
    }
}

fn created_key(post: &Post) -> Option<i64> {
    post.created_at
        .as_deref()
        .and_then(crate::time::parse_created_at)
}

fn cmp_created_desc(left: &Post, right: &Post) -> Ordering {
    match (created_key(left), created_key(right)) {
        (Some(a), Some(b)) => b.cmp(&a).then_with(|| left.id.cmp(&right.id)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => left.id.cmp(&right.id),
    }
}

/// Stable in-place sort of a tweet list.
pub fn sort_posts(posts: &mut [Post], sort: ResultSort) {
    posts.sort_by(|left, right| cmp_posts(left, right, sort));
}

/// Sorted copy of a tweet list.
pub fn sorted_posts(posts: &[Post], sort: ResultSort) -> Vec<Post> {
    let mut out = posts.to_vec();
    sort_posts(&mut out, sort);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post(id: &str, likes: u64, reposts: u64, created: Option<&str>) -> Post {
        Post {
            id: id.into(),
            url: format!("https://x.com/a/status/{id}"),
            author_handle: "a".into(),
            author_name: "A".into(),
            text: format!("t{id}"),
            created_at: created.map(str::to_string),
            likes,
            reposts,
            replies: 0,
            bookmarks: 0,
            ..Post::default()
        }
    }

    #[test]
    fn parse_accepts_popular_newest_and_legacy_aliases() {
        assert_eq!(ResultSort::parse("popular"), Some(ResultSort::Popular));
        assert_eq!(ResultSort::parse("Top"), Some(ResultSort::Popular));
        assert_eq!(ResultSort::parse("newest"), Some(ResultSort::Newest));
        assert_eq!(ResultSort::parse("recent"), Some(ResultSort::Newest));
        assert_eq!(ResultSort::parse("nope"), None);
        assert_eq!(ResultSort::Popular.as_str(), "popular");
        assert_eq!(ResultSort::Newest.label(), "Newest");
        assert_eq!(ResultSort::Popular.toggle(), ResultSort::Newest);
        assert_eq!(LIST_SORT_KEY, "s");
    }

    #[test]
    fn popular_orders_likes_then_engagement() {
        let posts = vec![
            post("low-like-high-eng", 10, 40, Some("2026-10-01T00:00:00Z")),
            post("high-like", 90, 0, Some("2026-10-02T00:00:00Z")),
            post("mid-like-newer", 10, 1, Some("2026-10-08T00:00:00Z")),
        ];
        let ids: Vec<String> = sorted_posts(&posts, ResultSort::Popular)
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(
            ids,
            ["high-like", "low-like-high-eng", "mid-like-newer"],
            "likes win; equal likes fall through to engagement"
        );
    }

    #[test]
    fn newest_orders_created_at_desc_and_puts_missing_last() {
        let posts = vec![
            post("old", 99, 0, Some("2026-10-01T00:00:00Z")),
            post("undated", 1, 0, None),
            post("new", 2, 0, Some("2026-10-09T12:00:00Z")),
        ];
        let ids: Vec<String> = sorted_posts(&posts, ResultSort::Newest)
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(ids, ["new", "old", "undated"]);
    }

    #[test]
    fn newest_parses_twitter_dates_and_unix_instead_of_raw_text() {
        // Raw text order is the opposite of time: "Mon..." < "Wed...",
        // and "99" > "100".
        let posts = vec![
            post(
                "old-twitter",
                50,
                0,
                Some("Wed Oct 01 00:00:00 +0000 2020"),
            ),
            post(
                "new-twitter",
                1,
                0,
                Some("Mon Jan 15 00:00:00 +0000 2026"),
            ),
            post("unix-small", 40, 0, Some("99")),
            post("unix-big", 2, 0, Some("100")),
        ];
        let ids: Vec<String> = sorted_posts(&posts, ResultSort::Newest)
            .into_iter()
            .map(|item| item.id)
            .collect();
        assert_eq!(
            ids,
            ["new-twitter", "old-twitter", "unix-big", "unix-small"]
        );
        let popular: Vec<String> = sorted_posts(&posts, ResultSort::Popular)
            .into_iter()
            .map(|item| item.id)
            .collect();
        assert_ne!(popular, ids);
    }

    #[test]
    fn popular_keeps_input_order_on_a_full_tie() {
        let posts = vec![
            post("first", 5, 0, Some("2026-10-01T00:00:00Z")),
            post("second", 5, 0, Some("2026-10-01T00:00:00Z")),
        ];
        let ids: Vec<String> = sorted_posts(&posts, ResultSort::Popular)
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(ids, ["first", "second"]);
    }
}
