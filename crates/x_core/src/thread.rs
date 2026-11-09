//! Nested reply trees from `in_reply_to` ids, sorted per sibling level.

use crate::model::Post;
use crate::sort::{ResultSort, cmp_posts};

/// Visual indent levels. Depth 1 is a direct reply to the focused tweet;
/// depth 5+ still nests in the tree but renders at this cap.
pub const MAX_DISPLAY_DEPTH: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadNode {
    pub post: Post,
    /// 1 = direct reply to the focused tweet.
    pub depth: usize,
    pub children: Vec<ThreadNode>,
}

impl ThreadNode {
    pub fn display_depth(&self) -> usize {
        self.depth.min(MAX_DISPLAY_DEPTH)
    }
}

/// Forest of replies under `focus_id`. The focused tweet itself is not
/// included — callers pin it separately. Unknown / missing parents attach
/// as direct children of the focus. Cycles and self-replies are dropped.
pub fn build_reply_tree(focus_id: &str, replies: &[Post], sort: ResultSort) -> Vec<ThreadNode> {
    use std::collections::{HashMap, HashSet};

    let mut by_id: HashMap<&str, &Post> = HashMap::new();
    for post in replies {
        if post.id.is_empty() || post.id == focus_id {
            continue;
        }
        if post.in_reply_to.as_deref() == Some(post.id.as_str()) {
            continue;
        }
        by_id.entry(post.id.as_str()).or_insert(post);
    }

    let mut children_of: HashMap<&str, Vec<&str>> = HashMap::new();
    for post in replies {
        if !by_id.contains_key(post.id.as_str()) {
            continue;
        }
        let parent = match post
            .in_reply_to
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty() && *id != "0" && *id != post.id)
        {
            Some(parent) if by_id.contains_key(parent) || parent == focus_id => parent,
            _ => focus_id,
        };
        children_of
            .entry(parent)
            .or_default()
            .push(post.id.as_str());
    }

    fn sort_ids(ids: &mut [&str], by_id: &HashMap<&str, &Post>, sort: ResultSort) {
        ids.sort_by(|a, b| cmp_posts(by_id[a], by_id[b], sort));
    }

    fn build(
        parent_id: &str,
        depth: usize,
        children_of: &HashMap<&str, Vec<&str>>,
        by_id: &HashMap<&str, &Post>,
        sort: ResultSort,
        visiting: &mut HashSet<String>,
    ) -> Vec<ThreadNode> {
        let Some(ids) = children_of.get(parent_id) else {
            return Vec::new();
        };
        let mut ids: Vec<&str> = ids
            .iter()
            .copied()
            .filter(|id| visiting.insert((*id).to_string()))
            .collect();
        sort_ids(&mut ids, by_id, sort);
        ids.into_iter()
            .filter_map(|id| {
                let post = by_id.get(id).copied()?.clone();
                let children = build(id, depth + 1, children_of, by_id, sort, visiting);
                Some(ThreadNode {
                    post,
                    depth,
                    children,
                })
            })
            .collect()
    }

    let mut visiting = HashSet::new();
    visiting.insert(focus_id.to_string());
    let mut nodes = build(focus_id, 1, &children_of, &by_id, sort, &mut visiting);
    // Disconnected cycles (A replies to B, B to A) never reach the
    // focus walk; show them as depth-1 orphans so no reply is dropped.
    let mut leftover: Vec<&str> = by_id
        .keys()
        .copied()
        .filter(|id| !visiting.contains(*id))
        .collect();
    sort_ids(&mut leftover, &by_id, sort);
    for id in leftover {
        if !visiting.insert(id.to_string()) {
            continue;
        }
        nodes.push(ThreadNode {
            post: by_id[id].clone(),
            depth: 1,
            children: Vec::new(),
        });
    }
    nodes.sort_by(|a, b| cmp_posts(&a.post, &b.post, sort));
    nodes
}

/// Depth-first walk, parents before children, sibling order preserved.
pub fn walk_thread(nodes: &[ThreadNode]) -> Vec<&ThreadNode> {
    let mut out = Vec::new();
    fn rec<'a>(nodes: &'a [ThreadNode], out: &mut Vec<&'a ThreadNode>) {
        for node in nodes {
            out.push(node);
            rec(&node.children, out);
        }
    }
    rec(nodes, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post(id: &str, likes: u64, created: Option<&str>, parent: Option<&str>) -> Post {
        Post {
            id: id.into(),
            url: format!("https://x.com/a/status/{id}"),
            author_handle: "a".into(),
            author_name: "A".into(),
            text: format!("t{id}"),
            created_at: created.map(str::to_string),
            likes,
            in_reply_to: parent.map(str::to_string),
            ..Post::default()
        }
    }

    #[test]
    fn nests_from_in_reply_to_and_caps_display_depth() {
        let focus = "root";
        let replies = vec![
            post("a", 1, Some("2026-01-01T00:00:00Z"), Some(focus)),
            post("b", 1, Some("2026-01-01T01:00:00Z"), Some("a")),
            post("c", 1, Some("2026-01-01T02:00:00Z"), Some("b")),
            post("d", 1, Some("2026-01-01T03:00:00Z"), Some("c")),
            post("e", 1, Some("2026-01-01T04:00:00Z"), Some("d")),
        ];
        let tree = build_reply_tree(focus, &replies, ResultSort::Newest);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].post.id, "a");
        assert_eq!(tree[0].depth, 1);
        let flat = walk_thread(&tree);
        let ids: Vec<&str> = flat.iter().map(|n| n.post.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c", "d", "e"]);
        assert_eq!(flat[0].display_depth(), 1);
        assert_eq!(flat[3].depth, 4);
        assert_eq!(flat[3].display_depth(), 4);
        assert_eq!(flat[4].depth, 5);
        assert_eq!(flat[4].display_depth(), 4);
    }

    #[test]
    fn popular_sorts_siblings_by_likes_newest_by_created_at() {
        let focus = "root";
        let replies = vec![
            post("low", 2, Some("2026-10-06T16:00:00Z"), Some(focus)),
            post("high", 90, Some("2026-10-06T12:00:00Z"), Some(focus)),
            post("mid", 20, Some("2026-10-06T14:00:00Z"), Some(focus)),
            post("child-low", 1, Some("2026-10-06T18:00:00Z"), Some("high")),
            post("child-high", 8, Some("2026-10-06T13:00:00Z"), Some("high")),
        ];
        let popular = build_reply_tree(focus, &replies, ResultSort::Popular);
        let popular_ids: Vec<&str> = popular.iter().map(|n| n.post.id.as_str()).collect();
        assert_eq!(popular_ids, ["high", "mid", "low"]);
        let nested: Vec<&str> = popular[0]
            .children
            .iter()
            .map(|n| n.post.id.as_str())
            .collect();
        assert_eq!(
            nested,
            ["child-high", "child-low"],
            "sort is per sibling level"
        );

        let newest = build_reply_tree(focus, &replies, ResultSort::Newest);
        let newest_ids: Vec<&str> = newest.iter().map(|n| n.post.id.as_str()).collect();
        assert_eq!(newest_ids, ["low", "mid", "high"]);
        let nested_new: Vec<&str> = newest
            .iter()
            .find(|n| n.post.id == "high")
            .unwrap()
            .children
            .iter()
            .map(|n| n.post.id.as_str())
            .collect();
        assert_eq!(nested_new, ["child-low", "child-high"]);
    }

    #[test]
    fn unknown_parent_and_missing_in_reply_to_attach_under_focus() {
        let replies = vec![
            post("orphan", 3, None, Some("gone")),
            post("bare", 4, None, None),
            post("self", 1, None, Some("self")),
        ];
        let tree = build_reply_tree("root", &replies, ResultSort::Popular);
        let ids: Vec<&str> = tree.iter().map(|n| n.post.id.as_str()).collect();
        assert_eq!(ids, ["bare", "orphan"]);
        assert!(tree.iter().all(|n| n.depth == 1 && n.children.is_empty()));
    }

    #[test]
    fn skips_focus_row_and_breaks_cycles() {
        let replies = vec![
            post("root", 9, None, None),
            post("a", 1, None, Some("b")),
            post("b", 1, None, Some("a")),
            post("c", 5, None, Some("root")),
        ];
        let tree = build_reply_tree("root", &replies, ResultSort::Popular);
        let ids: Vec<&str> = walk_thread(&tree)
            .iter()
            .map(|n| n.post.id.as_str())
            .collect();
        assert!(!ids.contains(&"root"));
        assert_eq!(tree[0].post.id, "c");
        assert!(ids.contains(&"a") && ids.contains(&"b"));
        assert_eq!(
            ids.len(),
            ids.iter()
                .copied()
                .collect::<std::collections::HashSet<_>>()
                .len()
        );
    }

    #[test]
    fn newest_puts_missing_timestamps_last() {
        let replies = vec![
            post("dated", 1, Some("2026-10-01T00:00:00Z"), Some("root")),
            post("undated", 99, None, Some("root")),
        ];
        let tree = build_reply_tree("root", &replies, ResultSort::Newest);
        assert_eq!(tree[0].post.id, "dated");
        assert_eq!(tree[1].post.id, "undated");
    }
}
