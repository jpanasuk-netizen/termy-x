//! Account stats from the OpenCLI profile payload plus a tweet window.
//!
//! Follower / following / tweet / like totals come from `twitter profile`
//! fields. They are never `len()` of a capped `twitter followers` page
//! (that page is typically 200 and under-counts). Missing fields stay
//! `None` and render as `n/a`.

use serde::Serialize;
use serde_json::Value;

use crate::model::Post;

/// Cap used by OpenCLI `twitter tweets --limit` in profile lookup.
pub const TWEET_WINDOW_CAP: usize = 120;

/// Account-level totals from an OpenCLI / GraphQL / API profile row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct ProfileTotals {
    pub followers: Option<u64>,
    pub following: Option<u64>,
    pub tweets: Option<u64>,
    pub likes: Option<u64>,
    pub verified: Option<bool>,
    pub created_at: Option<String>,
}

/// Sums over the loaded tweet window. Every field is `None` (n/a) when
/// no loaded tweet carried that key.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct TweetWindowStats {
    /// How many tweets were summed.
    pub loaded: usize,
    /// Cap applied (`TWEET_WINDOW_CAP`).
    pub cap: usize,
    pub views: Option<u64>,
    pub likes: Option<u64>,
    pub replies: Option<u64>,
    pub reposts: Option<u64>,
    pub quotes: Option<u64>,
    pub bookmarks: Option<u64>,
    /// likes + replies + reposts + quotes + bookmarks, using only
    /// fields that were present.
    pub engagement: Option<u64>,
    /// engagement / views when views is present and greater than 0.
    pub engagement_rate: Option<f64>,
}

/// Combined profile totals + tweet window for the panel and `x stats`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AccountStats {
    pub handle: String,
    pub name: String,
    pub totals: ProfileTotals,
    pub window: TweetWindowStats,
}

impl TweetWindowStats {
    pub fn window_note(&self) -> String {
        format!(
            "summed over {} loaded tweets (OpenCLI twitter tweets, cap {})",
            self.loaded, self.cap
        )
    }
}

/// Format an optional count: the number, or `n/a`.
pub fn fmt_count(value: Option<u64>) -> String {
    match value {
        Some(n) => n.to_string(),
        None => "n/a".into(),
    }
}

/// Format verified: yes / no / n/a.
pub fn fmt_verified(value: Option<bool>) -> String {
    match value {
        Some(true) => "yes".into(),
        Some(false) => "no".into(),
        None => "n/a".into(),
    }
}

/// Format created_at, or `n/a`.
pub fn fmt_created(value: Option<&str>) -> String {
    match value.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => s.to_string(),
        None => "n/a".into(),
    }
}

/// Format engagement rate as a percent, or `n/a`.
pub fn fmt_rate(value: Option<f64>) -> String {
    match value {
        Some(rate) => format!("{:.2}%", rate * 100.0),
        None => "n/a".into(),
    }
}

/// Quote count from a tweet row. `None` when the payload omitted quotes.
pub fn quote_count_from_metrics(metrics: &Value, item: &Value) -> Option<u64> {
    optional_number(metrics, &["quote_count", "quotes", "quoteCount"])
        .or_else(|| optional_number(item, &["quote_count", "quotes", "quoteCount"]))
}

fn optional_number(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(json_u64))
}

fn json_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|n| u64::try_from(n).ok()))
        .or_else(|| {
            value.as_f64().and_then(|n| {
                if n.is_finite() && n >= 0.0 && n.fract() == 0.0 {
                    Some(n as u64)
                } else {
                    None
                }
            })
        })
        .or_else(|| {
            value.as_str().and_then(|s| {
                s.chars()
                    .filter(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .parse()
                    .ok()
            })
        })
}

fn optional_bool(value: &Value, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_bool))
}

fn first_str<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn profile_row(value: &Value) -> &Value {
    let node = value
        .get("data")
        .or_else(|| value.get("user"))
        .unwrap_or(value);
    node.as_array()
        .and_then(|rows| rows.first())
        .unwrap_or(node)
}

/// Totals from a profile JSON object or one-row array (OpenCLI shape).
pub fn profile_totals_from_value(value: &Value) -> ProfileTotals {
    let node = profile_row(value);
    let metrics = node.get("public_metrics").unwrap_or(node);
    let from =
        |keys: &[&str]| optional_number(metrics, keys).or_else(|| optional_number(node, keys));
    ProfileTotals {
        followers: from(&["followers_count", "followers"]),
        following: from(&["following_count", "friends_count", "following"]),
        tweets: from(&["tweets", "statuses_count", "tweet_count", "statusesCount"]),
        likes: from(&[
            "likes",
            "favourites_count",
            "favorites_count",
            "favourite_count",
            "favorites",
        ]),
        verified: optional_bool(
            node,
            &["verified", "is_blue_verified", "isVerified", "is_verified"],
        )
        .or_else(|| {
            node.get("legacy")
                .and_then(|legacy| optional_bool(legacy, &["verified", "is_blue_verified"]))
        }),
        created_at: first_str(node, &["created_at", "createdAt"]).map(str::to_string),
    }
}

pub fn profile_totals_from_json(text: &str) -> Result<ProfileTotals, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    Ok(profile_totals_from_value(&value))
}

fn sum_optional<I>(values: I) -> Option<u64>
where
    I: IntoIterator<Item = Option<u64>>,
{
    let mut any = false;
    let mut sum = 0u64;
    for n in values.into_iter().flatten() {
        any = true;
        sum = sum.saturating_add(n);
    }
    any.then_some(sum)
}

fn engagement_of(window: &TweetWindowStats) -> Option<u64> {
    sum_optional([
        window.likes,
        window.replies,
        window.reposts,
        window.quotes,
        window.bookmarks,
    ])
}

fn rate_of(engagement: Option<u64>, views: Option<u64>) -> Option<f64> {
    match (engagement, views) {
        (Some(eng), Some(views)) if views > 0 => Some(eng as f64 / views as f64),
        _ => None,
    }
}

/// Window sums from already-parsed posts (profile lookup tweets).
///
/// `views` / likes / replies / reposts / bookmarks on [`Post`] default
/// to 0 when the parser did not see a key, so those sums are `Some`
/// whenever the window is non-empty. `quotes` stays `None` (n/a) unless
/// at least one post carried the field.
pub fn tweet_window_from_posts(posts: &[Post], cap: usize) -> TweetWindowStats {
    let cap = cap.max(1);
    let loaded: Vec<&Post> = posts.iter().take(cap).collect();
    if loaded.is_empty() {
        return TweetWindowStats {
            loaded: 0,
            cap,
            ..TweetWindowStats::default()
        };
    }
    let quotes = sum_optional(loaded.iter().map(|p| p.quotes));
    let mut window = TweetWindowStats {
        loaded: loaded.len(),
        cap,
        views: Some(
            loaded
                .iter()
                .map(|p| p.views)
                .fold(0u64, u64::saturating_add),
        ),
        likes: Some(
            loaded
                .iter()
                .map(|p| p.likes)
                .fold(0u64, u64::saturating_add),
        ),
        replies: Some(
            loaded
                .iter()
                .map(|p| p.replies)
                .fold(0u64, u64::saturating_add),
        ),
        reposts: Some(
            loaded
                .iter()
                .map(|p| p.reposts)
                .fold(0u64, u64::saturating_add),
        ),
        quotes,
        bookmarks: Some(
            loaded
                .iter()
                .map(|p| p.bookmarks)
                .fold(0u64, u64::saturating_add),
        ),
        engagement: None,
        engagement_rate: None,
    };
    window.engagement = engagement_of(&window);
    window.engagement_rate = rate_of(window.engagement, window.views);
    window
}

/// Per-tweet metric row parsed from OpenCLI `twitter tweets -f json`.
/// Missing keys stay `None` so the window can show n/a.
#[derive(Debug, Clone, Default)]
struct RawTweetMetrics {
    views: Option<u64>,
    likes: Option<u64>,
    replies: Option<u64>,
    reposts: Option<u64>,
    quotes: Option<u64>,
    bookmarks: Option<u64>,
}

fn tweet_rows(value: &Value) -> Vec<&Value> {
    if let Some(array) = value.as_array() {
        return array.iter().collect();
    }
    for key in ["data", "tweets", "items", "rows", "results", "timeline"] {
        if let Some(array) = value.get(key).and_then(Value::as_array) {
            return array.iter().collect();
        }
    }
    Vec::new()
}

fn raw_tweet_metrics(item: &Value) -> RawTweetMetrics {
    let metrics = item
        .get("public_metrics")
        .or_else(|| item.get("metrics"))
        .unwrap_or(item);
    let from =
        |keys: &[&str]| optional_number(metrics, keys).or_else(|| optional_number(item, keys));
    RawTweetMetrics {
        views: from(&["impression_count", "views", "view_count"]),
        likes: from(&["like_count", "likes", "favorite_count"]),
        replies: from(&["reply_count", "replies"]),
        reposts: from(&["retweet_count", "reposts", "retweets"]),
        quotes: from(&["quote_count", "quotes", "quoteCount"]),
        bookmarks: from(&["bookmark_count", "bookmarks"]),
    }
}

/// Window sums from an OpenCLI tweets JSON payload. Missing keys → n/a.
pub fn tweet_window_from_json(text: &str, cap: usize) -> Result<TweetWindowStats, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let cap = cap.max(1);
    let rows: Vec<RawTweetMetrics> = tweet_rows(&value)
        .into_iter()
        .take(cap)
        .map(raw_tweet_metrics)
        .collect();
    if rows.is_empty() {
        return Ok(TweetWindowStats {
            loaded: 0,
            cap,
            ..TweetWindowStats::default()
        });
    }
    let mut window = TweetWindowStats {
        loaded: rows.len(),
        cap,
        views: sum_optional(rows.iter().map(|r| r.views)),
        likes: sum_optional(rows.iter().map(|r| r.likes)),
        replies: sum_optional(rows.iter().map(|r| r.replies)),
        reposts: sum_optional(rows.iter().map(|r| r.reposts)),
        quotes: sum_optional(rows.iter().map(|r| r.quotes)),
        bookmarks: sum_optional(rows.iter().map(|r| r.bookmarks)),
        engagement: None,
        engagement_rate: None,
    };
    window.engagement = engagement_of(&window);
    window.engagement_rate = rate_of(window.engagement, window.views);
    Ok(window)
}

pub fn account_stats(
    handle: &str,
    name: &str,
    totals: ProfileTotals,
    posts: &[Post],
    cap: usize,
) -> AccountStats {
    AccountStats {
        handle: handle.trim().trim_start_matches('@').to_string(),
        name: if name.trim().is_empty() {
            handle.trim().trim_start_matches('@').to_string()
        } else {
            name.to_string()
        },
        window: tweet_window_from_posts(posts, cap),
        totals,
    }
}

pub fn account_stats_from_profile(profile: &crate::model::Profile) -> AccountStats {
    account_stats(
        &profile.handle,
        &profile.name,
        ProfileTotals {
            followers: profile.followers,
            following: profile.following,
            tweets: profile.tweets,
            likes: profile.likes,
            verified: profile.verified,
            created_at: profile.created_at.clone(),
        },
        &profile.posts,
        TWEET_WINDOW_CAP,
    )
}

/// Human-readable block for the CLI and the profile card.
pub fn render_account_stats(stats: &AccountStats) -> String {
    let totals = &stats.totals;
    let window = &stats.window;
    format!(
        "@{handle} \u{b7} {name}\n\
followers {followers} \u{b7} following {following} \u{b7} posts {tweets} \u{b7} likes {likes} \u{b7} verified {verified} \u{b7} created_at {created}\n\
{window_note}\n\
views {views} \u{b7} likes {w_likes} \u{b7} replies {replies} \u{b7} reposts {reposts} \u{b7} quotes {quotes} \u{b7} bookmarks {bookmarks}\n\
engagement {engagement} \u{b7} rate {rate}",
        handle = stats.handle,
        name = stats.name,
        followers = fmt_count(totals.followers),
        following = fmt_count(totals.following),
        tweets = fmt_count(totals.tweets),
        likes = fmt_count(totals.likes),
        verified = fmt_verified(totals.verified),
        created = fmt_created(totals.created_at.as_deref()),
        window_note = window.window_note(),
        views = fmt_count(window.views),
        w_likes = fmt_count(window.likes),
        replies = fmt_count(window.replies),
        reposts = fmt_count(window.reposts),
        quotes = fmt_count(window.quotes),
        bookmarks = fmt_count(window.bookmarks),
        engagement = fmt_count(window.engagement),
        rate = fmt_rate(window.engagement_rate),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Post;

    /// Shape of live `opencli twitter profile -f json` saved at
    /// `C:\Users\jpana\work\profile-live.json` (counts are the live
    /// values; bio omitted).
    const LIVE_PROFILE: &str = r#"[
  {
    "screen_name": "Jasper_Black",
    "name": "MAGABRO",
    "bio": "links",
    "location": "Chicago",
    "url": "",
    "followers": 303,
    "following": 426,
    "tweets": 4391,
    "likes": 998,
    "verified": true,
    "created_at": "Mon May 02 03:33:07 +0000 2011"
  }
]"#;

    const LIVE_TWEETS: &str = r#"[
  {"id":"1","username":"Jasper_Black","text":"a","likes":2,"replies":1,"retweets":0,"views":100,"bookmarks":1,"quotes":0},
  {"id":"2","username":"Jasper_Black","text":"b","likes":4,"replies":0,"retweets":1,"views":50,"bookmarks":0},
  {"id":"3","username":"Jasper_Black","text":"c","like_count":1,"reply_count":2,"retweet_count":0,"impression_count":25,"quote_count":1}
]"#;

    fn post(likes: u64, views: u64, quotes: Option<u64>) -> Post {
        Post {
            likes,
            views,
            quotes,
            replies: 1,
            reposts: 0,
            bookmarks: 0,
            ..Post::default()
        }
    }

    #[test]
    fn live_profile_json_uses_payload_totals_not_page_len() {
        let totals = profile_totals_from_json(LIVE_PROFILE).unwrap();
        assert_eq!(totals.followers, Some(303));
        assert_eq!(totals.following, Some(426));
        assert_eq!(totals.tweets, Some(4391));
        assert_eq!(totals.likes, Some(998));
        assert_eq!(totals.verified, Some(true));
        assert_eq!(
            totals.created_at.as_deref(),
            Some("Mon May 02 03:33:07 +0000 2011")
        );
        // The old (wrong) path counted a capped followers page.
        let capped_page_len = 200u64;
        assert_ne!(
            totals.followers,
            Some(capped_page_len),
            "account total must not be len() of a 200-cap followers page"
        );
        assert_eq!(fmt_count(totals.followers), "303");
        assert_eq!(fmt_verified(totals.verified), "yes");
    }

    #[test]
    fn missing_profile_fields_are_na_not_zero() {
        let totals = profile_totals_from_json(r#"[{"screen_name":"ada","name":"Ada"}]"#).unwrap();
        assert_eq!(totals.followers, None);
        assert_eq!(totals.following, None);
        assert_eq!(totals.tweets, None);
        assert_eq!(totals.likes, None);
        assert_eq!(totals.verified, None);
        assert_eq!(totals.created_at, None);
        assert_eq!(fmt_count(None), "n/a");
        assert_eq!(fmt_verified(None), "n/a");
        assert_eq!(fmt_created(None), "n/a");
        assert_eq!(fmt_rate(None), "n/a");
    }

    #[test]
    fn public_metrics_and_string_counts_parse() {
        let json = r#"{"username":"a","public_metrics":{"followers_count":"1,200","following_count":3,"tweet_count":9},"legacy":{"verified":false},"createdAt":"2020-01-01"}"#;
        let totals = profile_totals_from_json(json).unwrap();
        assert_eq!(totals.followers, Some(1200));
        assert_eq!(totals.following, Some(3));
        assert_eq!(totals.tweets, Some(9));
        assert_eq!(totals.verified, Some(false));
        assert_eq!(totals.created_at.as_deref(), Some("2020-01-01"));
    }

    #[test]
    fn following_count_is_not_the_viewer_bool() {
        let totals =
            profile_totals_from_json(r#"{"username":"a","following":true,"followers":5}"#).unwrap();
        assert_eq!(totals.followers, Some(5));
        assert_eq!(totals.following, None);
        assert_eq!(totals.verified, None);
    }

    #[test]
    fn tweet_window_from_json_sums_present_keys_only() {
        let window = tweet_window_from_json(LIVE_TWEETS, TWEET_WINDOW_CAP).unwrap();
        assert_eq!(window.loaded, 3);
        assert_eq!(window.cap, 120);
        assert_eq!(window.views, Some(175));
        assert_eq!(window.likes, Some(7));
        assert_eq!(window.replies, Some(3));
        assert_eq!(window.reposts, Some(1));
        assert_eq!(window.quotes, Some(1));
        assert_eq!(window.bookmarks, Some(1));
        assert_eq!(window.engagement, Some(7 + 3 + 1 + 1 + 1));
        let rate = window.engagement_rate.expect("rate");
        assert!((rate - (13.0 / 175.0)).abs() < 1e-9);
        assert!(window.window_note().contains("3 loaded tweets"));
        assert!(window.window_note().contains("cap 120"));
    }

    #[test]
    fn tweet_window_missing_quotes_is_na() {
        let json = r#"[{"id":"1","likes":2,"views":10,"replies":0,"retweets":0,"bookmarks":0}]"#;
        let window = tweet_window_from_json(json, 120).unwrap();
        assert_eq!(window.quotes, None);
        assert_eq!(fmt_count(window.quotes), "n/a");
        assert_eq!(window.likes, Some(2));
        assert_eq!(window.views, Some(10));
        // engagement still uses the present fields
        assert_eq!(window.engagement, Some(2));
        assert!((window.engagement_rate.unwrap() - 0.2).abs() < 1e-9);
    }

    #[test]
    fn tweet_window_empty_and_zero_views_are_na_rate() {
        let empty = tweet_window_from_json("[]", 120).unwrap();
        assert_eq!(empty.loaded, 0);
        assert_eq!(empty.views, None);
        assert_eq!(empty.engagement_rate, None);
        let zero = tweet_window_from_json(r#"[{"id":"1","likes":2,"views":0}]"#, 120).unwrap();
        assert_eq!(zero.views, Some(0));
        assert_eq!(zero.engagement_rate, None);
        assert_eq!(fmt_rate(None), "n/a");
    }

    #[test]
    fn tweet_window_from_posts_keeps_quotes_optional() {
        let posts = vec![post(3, 10, Some(1)), post(1, 5, None)];
        let window = tweet_window_from_posts(&posts, 120);
        assert_eq!(window.loaded, 2);
        assert_eq!(window.likes, Some(4));
        assert_eq!(window.views, Some(15));
        assert_eq!(window.quotes, Some(1));
        let none_quotes = tweet_window_from_posts(&[post(1, 8, None)], 120);
        assert_eq!(none_quotes.quotes, None);
    }

    #[test]
    fn render_uses_na_and_live_counts() {
        let totals = profile_totals_from_json(LIVE_PROFILE).unwrap();
        let stats = account_stats("Jasper_Black", "MAGABRO", totals, &[], 120);
        let text = render_account_stats(&stats);
        assert!(text.contains("@Jasper_Black"));
        assert!(text.contains("followers 303"));
        assert!(text.contains("following 426"));
        assert!(text.contains("posts 4391"));
        assert!(text.contains("likes 998"));
        assert!(text.contains("verified yes"));
        assert!(text.contains("Mon May 02 03:33:07 +0000 2011"));
        assert!(text.contains("0 loaded tweets"));
        assert!(text.contains("views n/a"));
        assert!(text.contains("rate n/a"));
    }
}
