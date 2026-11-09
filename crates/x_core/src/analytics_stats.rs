//! Analytics over a tweet list: period compare, heatmap, top posts,
//! and a 14-day account read. Pure functions; callers fetch via OpenCLI.

use serde::Serialize;

use crate::model::{Post, Profile};
use crate::time::{parse_created_at, start_of_day, weekday_hour, ymd};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DayBucket {
    pub day: String,
    pub posts: u64,
    pub likes: u64,
    pub replies: u64,
    pub reposts: u64,
    pub views: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PeriodStats {
    pub days: Vec<DayBucket>,
    pub posts: u64,
    pub likes: u64,
    pub replies: u64,
    pub reposts: u64,
    pub views: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DeltaPct {
    pub posts: f64,
    pub likes: f64,
    pub replies: f64,
    pub reposts: f64,
    pub views: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PeriodCompare {
    pub days: u32,
    pub current: PeriodStats,
    pub prior: PeriodStats,
    pub delta_pct: DeltaPct,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Heatmap {
    /// Weekday (0 = Sunday) × hour (0-23) engagement sums.
    pub cells: [[u64; 24]; 7],
    pub best: Option<HeatCell>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HeatCell {
    pub weekday: u8,
    pub hour: u8,
    pub engagement: u64,
}

impl HeatCell {
    pub fn label(&self) -> String {
        const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
        let day = DAYS.get(self.weekday as usize).copied().unwrap_or("?");
        format!("{day} {hour:02}:00", hour = self.hour)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AccountReport {
    pub handle: String,
    pub name: String,
    pub days: u32,
    pub posts: u64,
    pub posts_per_day: f64,
    pub avg_engagement: f64,
    pub reply_ratio: f64,
    pub best_hours: Vec<HeatCell>,
    pub top_posts: Vec<Post>,
    pub compare: PeriodCompare,
    pub heatmap: Heatmap,
}

pub fn delta_pct(current: u64, prior: u64) -> f64 {
    if prior == 0 {
        if current == 0 { 0.0 } else { 100.0 }
    } else {
        (current as f64 - prior as f64) / prior as f64 * 100.0
    }
}

fn in_window(post: &Post, start: i64, end: i64) -> bool {
    match post.created_at.as_deref().and_then(parse_created_at) {
        Some(ts) => ts >= start && ts < end,
        None => false,
    }
}

/// Reposts carry the original author's engagement, so they are left out of
/// every engagement stat (posts, heatmap, top posts, averages).
pub fn is_repost(post: &Post) -> bool {
    post.retweeted.is_some() || post.text.trim_start().starts_with("RT @")
}

fn accumulate(posts: &[Post], start: i64, end: i64) -> PeriodStats {
    let span_days = ((end - start).max(0) / 86_400) as usize;
    let mut days: Vec<DayBucket> = (0..span_days)
        .map(|i| {
            let day_start = start + (i as i64) * 86_400;
            DayBucket {
                day: ymd(day_start),
                posts: 0,
                likes: 0,
                replies: 0,
                reposts: 0,
                views: 0,
            }
        })
        .collect();
    let mut totals = PeriodStats {
        days: Vec::new(),
        posts: 0,
        likes: 0,
        replies: 0,
        reposts: 0,
        views: 0,
    };
    for post in posts.iter().filter(|p| !is_repost(p)) {
        let Some(ts) = post.created_at.as_deref().and_then(parse_created_at) else {
            continue;
        };
        if ts < start || ts >= end {
            continue;
        }
        let idx = ((ts - start) / 86_400) as usize;
        if let Some(bucket) = days.get_mut(idx) {
            bucket.posts += 1;
            bucket.likes += post.likes;
            bucket.replies += post.replies;
            bucket.reposts += post.reposts;
            bucket.views += post.views;
        }
        totals.posts += 1;
        totals.likes += post.likes;
        totals.replies += post.replies;
        totals.reposts += post.reposts;
        totals.views += post.views;
    }
    totals.days = days;
    totals
}

pub fn compare_periods(posts: &[Post], now: i64, days: u32) -> PeriodCompare {
    let days = days.clamp(1, 90);
    let today = start_of_day(now) + 86_400;
    let current_start = today - i64::from(days) * 86_400;
    let prior_start = current_start - i64::from(days) * 86_400;
    let current = accumulate(posts, current_start, today);
    let prior = accumulate(posts, prior_start, current_start);
    PeriodCompare {
        days,
        delta_pct: DeltaPct {
            posts: delta_pct(current.posts, prior.posts),
            likes: delta_pct(current.likes, prior.likes),
            replies: delta_pct(current.replies, prior.replies),
            reposts: delta_pct(current.reposts, prior.reposts),
            views: delta_pct(current.views, prior.views),
        },
        current,
        prior,
    }
}

pub fn heatmap(posts: &[Post]) -> Heatmap {
    let mut cells = [[0u64; 24]; 7];
    for post in posts.iter().filter(|p| !is_repost(p)) {
        let Some(ts) = post.created_at.as_deref().and_then(parse_created_at) else {
            continue;
        };
        let (wd, hour) = weekday_hour(ts);
        if (wd as usize) < 7 && (hour as usize) < 24 {
            cells[wd as usize][hour as usize] =
                cells[wd as usize][hour as usize].saturating_add(post.engagement());
        }
    }
    let mut best: Option<HeatCell> = None;
    for (wd, row) in cells.iter().enumerate() {
        for (hour, value) in row.iter().enumerate() {
            if *value == 0 {
                continue;
            }
            let better = best.as_ref().is_none_or(|cell| *value > cell.engagement);
            if better {
                best = Some(HeatCell {
                    weekday: wd as u8,
                    hour: hour as u8,
                    engagement: *value,
                });
            }
        }
    }
    Heatmap { cells, best }
}

pub fn top_posts(posts: &[Post], n: usize) -> Vec<Post> {
    let mut ranked =
        crate::model::rank_by_engagement(posts.iter().filter(|p| !is_repost(p)).cloned().collect());
    ranked.truncate(n.max(1));
    ranked
}

/// Fraction of posts that look like replies (`@handle` prefix).
pub fn reply_ratio(posts: &[Post]) -> f64 {
    if posts.is_empty() {
        return 0.0;
    }
    let replies = posts.iter().filter(|p| looks_like_reply(p)).count();
    replies as f64 / posts.len() as f64
}

pub fn looks_like_reply(post: &Post) -> bool {
    post.card_text()
        .trim_start()
        .strip_prefix('@')
        .is_some_and(|rest| {
            rest.chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        })
}

pub fn avg_engagement(posts: &[Post]) -> f64 {
    if posts.is_empty() {
        return 0.0;
    }
    let own: Vec<&Post> = posts.iter().filter(|p| !is_repost(p)).collect();
    if own.is_empty() {
        return 0.0;
    }
    let sum: u64 = own.iter().map(|p| p.engagement()).sum();
    sum as f64 / own.len() as f64
}

pub fn posts_in_days(posts: &[Post], now: i64, days: u32) -> Vec<&Post> {
    let start = start_of_day(now) + 86_400 - i64::from(days.clamp(1, 90)) * 86_400;
    let end = start_of_day(now) + 86_400;
    posts.iter().filter(|p| in_window(p, start, end)).collect()
}

pub fn account_report(
    handle: &str,
    profile: Option<&Profile>,
    posts: &[Post],
    now: i64,
    days: u32,
) -> AccountReport {
    let days = days.clamp(1, 90);
    let window: Vec<Post> = posts_in_days(posts, now, days)
        .into_iter()
        .cloned()
        .collect();
    let heat = heatmap(&window);
    let mut best_hours: Vec<HeatCell> = Vec::new();
    if let Some(best) = heat.best.clone() {
        best_hours.push(best);
        // Second and third busiest hours, if any.
        let mut ranked: Vec<HeatCell> = Vec::new();
        for (wd, row) in heat.cells.iter().enumerate() {
            for (hour, value) in row.iter().enumerate() {
                if *value > 0 {
                    ranked.push(HeatCell {
                        weekday: wd as u8,
                        hour: hour as u8,
                        engagement: *value,
                    });
                }
            }
        }
        ranked.sort_by_key(|c| std::cmp::Reverse(c.engagement));
        best_hours = ranked.into_iter().take(3).collect();
    }
    AccountReport {
        handle: crate::model::strip_handle(handle).to_string(),
        name: profile
            .map(|p| p.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| crate::model::strip_handle(handle).to_string()),
        days,
        posts: window.len() as u64,
        posts_per_day: window.len() as f64 / f64::from(days),
        avg_engagement: avg_engagement(&window),
        reply_ratio: reply_ratio(&window),
        best_hours,
        top_posts: top_posts(&window, 5),
        compare: compare_periods(posts, now, days),
        heatmap: heat,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::sample_posts;

    fn dated(id: &str, created: &str, likes: u64, replies: u64, text: &str) -> Post {
        let mut post = sample_posts().remove(0);
        post.id = id.into();
        post.created_at = Some(created.into());
        post.likes = likes;
        post.replies = replies;
        post.reposts = 0;
        post.bookmarks = 0;
        post.views = likes * 10;
        post.text = text.into();
        post
    }

    #[test]
    fn compare_counts_current_vs_prior_and_delta() {
        let now = crate::time::parse_created_at("2026-10-10T12:00:00Z").unwrap();
        let posts = vec![
            dated("a", "2026-10-09T10:00:00Z", 10, 1, "now"),
            dated("b", "2026-10-08T10:00:00Z", 20, 2, "now2"),
            dated("c", "2026-10-02T10:00:00Z", 5, 0, "prior"),
        ];
        let cmp = compare_periods(&posts, now, 7);
        assert_eq!(cmp.current.posts, 2);
        assert_eq!(cmp.prior.posts, 1);
        assert!((cmp.delta_pct.posts - 100.0).abs() < f64::EPSILON);
        assert_eq!(cmp.current.likes, 30);
        assert_eq!(cmp.prior.likes, 5);
        assert_eq!(cmp.current.days.len(), 7);
    }

    #[test]
    fn heatmap_picks_the_busiest_hour() {
        let posts = vec![
            dated("a", "2026-10-06T13:00:00Z", 50, 0, "hot"), // Tue 13
            dated("b", "2026-10-06T13:30:00Z", 10, 0, "also"),
            dated("c", "2026-10-07T01:00:00Z", 5, 0, "quiet"),
        ];
        let heat = heatmap(&posts);
        let best = heat.best.expect("best hour");
        assert_eq!(best.weekday, 2);
        assert_eq!(best.hour, 13);
        assert!(best.engagement > 0);
        assert!(best.label().starts_with("Tue"));
    }

    #[test]
    fn reply_ratio_and_account_report() {
        let now = crate::time::parse_created_at("2026-10-10T12:00:00Z").unwrap();
        let posts = vec![
            dated("a", "2026-10-09T10:00:00Z", 8, 2, "@ada thanks"),
            dated("b", "2026-10-09T11:00:00Z", 4, 0, "original post"),
            dated("c", "2026-01-01T00:00:00Z", 99, 0, "too old"),
        ];
        assert!((reply_ratio(&posts[0..2]) - 0.5).abs() < f64::EPSILON);
        let report = account_report("Ada", None, &posts, now, 14);
        assert_eq!(report.handle, "Ada");
        assert_eq!(report.posts, 2);
        assert!((report.posts_per_day - (2.0 / 14.0)).abs() < 1e-9);
        assert_eq!(report.top_posts[0].id, "a");
        assert!(report.avg_engagement > 0.0);
        assert_eq!(delta_pct(0, 0), 0.0);
        assert_eq!(delta_pct(5, 0), 100.0);
    }

    #[test]
    fn reposts_are_excluded_from_engagement_stats() {
        let own = dated("1", "Tue Oct 20 12:00:00 +0000 2026", 3, 1, "my post");
        let rt = dated(
            "2",
            "Tue Oct 20 13:00:00 +0000 2026",
            9000,
            50,
            "RT @someone: big",
        );
        let posts = vec![own, rt];
        assert!(is_repost(&posts[1]));
        let top = top_posts(&posts, 5);
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].id, "1");
        assert!(avg_engagement(&posts) < 100.0);
        assert!(heatmap(&posts).best.is_some_and(|c| c.engagement < 100));
    }
}
