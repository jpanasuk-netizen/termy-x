use serde::{Deserialize, Serialize};

pub const SESSION_OWN: &str = "uses your own logged-in session";
pub const NO_CREDITS: &str = "no X API credits";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    OpenCli,
    TwitterCli,
    Mock,
    OfficialApi,
    WebIntent,
}

impl ProviderId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenCli => "opencli",
            Self::TwitterCli => "twitter-cli",
            Self::Mock => "mock",
            Self::OfficialApi => "official-api",
            Self::WebIntent => "web-intent",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::OpenCli => "OpenCLI",
            Self::TwitterCli => "twitter-cli",
            Self::Mock => "mock",
            Self::OfficialApi => "official X API",
            Self::WebIntent => "X web intent",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "opencli" => Some(Self::OpenCli),
            "twitter" | "twitter-cli" | "agent-reach" => Some(Self::TwitterCli),
            "mock" => Some(Self::Mock),
            "official" | "official-api" | "api" => Some(Self::OfficialApi),
            "intent" | "web-intent" => Some(Self::WebIntent),
            "auto" => None,
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptKind {
    Skipped,
    Failed,
    Served,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    pub provider: ProviderId,
    pub kind: AttemptKind,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CostEstimate {
    pub usd: f64,
    pub resources: usize,
    pub detail: String,
}

impl CostEstimate {
    pub fn label(&self) -> String {
        format!("est. ${:.3} · {}", self.usd, self.detail)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StatusLine {
    pub provider: ProviderId,
    pub session: Option<String>,
    pub credits: String,
    pub cost: Option<CostEstimate>,
    pub attempts: Vec<Attempt>,
    pub note: String,
}

impl StatusLine {
    pub fn free(provider: ProviderId, session: Option<&str>, note: impl Into<String>) -> Self {
        Self {
            provider,
            session: session.map(str::to_string),
            credits: NO_CREDITS.to_string(),
            cost: None,
            attempts: Vec::new(),
            note: note.into(),
        }
    }

    pub fn render(&self) -> String {
        let mut parts = vec![self.provider.label().to_string()];
        if let Some(session) = &self.session {
            parts.push(session.clone());
        }
        if let Some(cost) = &self.cost {
            parts.push(cost.label());
        } else {
            parts.push(self.credits.clone());
        }
        if !self.note.is_empty() {
            parts.push(self.note.clone());
        }
        parts.join(" · ")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Served<T> {
    pub value: T,
    pub status: StatusLine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaKind {
    Photo,
    Video,
    Gif,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaItem {
    pub kind: MediaKind,
    /// Direct media URL (photo image, or mp4 for video/GIF).
    pub url: String,
    /// Thumbnail/preview image URL; for photos this equals `url`.
    pub poster: Option<String>,
    /// Pixel dimensions when the provider or URL states them (video
    /// URLs carry `/480x270/`); photos usually leave this unset and the
    /// UI falls back to the decoded poster's real size.
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
}

impl MediaItem {
    /// Width/height aspect (w/h), when both dimensions are known.
    pub fn aspect(&self) -> Option<f32> {
        match (self.width, self.height) {
            (Some(w), Some(h)) if w > 0 && h > 0 => Some(w as f32 / h as f32),
            _ => None,
        }
    }
}

/// Dimensions encoded in a media URL path, e.g.
/// `.../vid/avc1/480x270/clip.mp4` or `.../320x568/...`. Returns None
/// when no plausible `WxH` segment exists (photo CDN URLs have none).
pub fn dims_from_media_url(url: &str) -> Option<(u32, u32)> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    for segment in path.split('/') {
        let Some((w, h)) = segment.split_once('x') else { continue };
        if w.is_empty() || h.is_empty() || w.len() > 5 || h.len() > 5 { continue; }
        if !w.bytes().all(|b| b.is_ascii_digit()) || !h.bytes().all(|b| b.is_ascii_digit()) { continue; }
        if let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>())
            && (16..=8192).contains(&w) && (16..=8192).contains(&h) {
            return Some((w, h));
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Post {
    pub id: String,
    pub url: String,
    pub author_handle: String,
    pub author_name: String,
    pub text: String,
    pub created_at: Option<String>,
    pub likes: u64,
    pub reposts: u64,
    pub replies: u64,
    pub bookmarks: u64,
    pub views: u64,
    /// Attached photos/videos/GIFs (empty for text-only posts).
    #[serde(default)]
    pub media: Vec<MediaItem>,
    /// The original tweet when this post is a repost/retweet: the
    /// outer post is the repost (its author is the reposter), and the
    /// card renders this embedded tweet inline, native-X style.
    /// Populated only when the provider payload carries the embedded
    /// tweet (GraphQL `retweeted_status_result`); never invented.
    #[serde(default)]
    pub retweeted: Option<Box<Post>>,
}

impl Post {
    /// The tweet Like/Retweet/Quote/Reply should act on: for a repost
    /// that's the original (embedded) tweet, matching native X, where
    /// engaging a repost engages the original. For a normal post it's
    /// the post itself.
    pub fn engage_target(&self) -> &Post {
        self.retweeted.as_deref().unwrap_or(self)
    }

    /// Handle of the original author when this post is a *bare*
    /// repost: no embedded original in the payload, only the classic
    /// `RT @handle:` text prefix some providers emit. Returns None for
    /// normal posts and for reposts that already carry the original.
    pub fn bare_repost_handle(&self) -> Option<&str> {
        if self.retweeted.is_some() {
            return None;
        }
        let rest = self.text.strip_prefix("RT @")?;
        let handle = rest.split(':').next()?;
        if handle.is_empty()
            || !handle
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return None;
        }
        // The colon must be present: "RT @x hello" without one is a
        // mention that merely starts a post, not a repost marker.
        rest[handle.len()..].starts_with(':').then_some(handle)
    }

    /// Text a card should show: the embedded original's text for a
    /// repost, otherwise this post's own text with any bare `RT @h:`
    /// repost prefix stripped (the reposter header carries that part).
    pub fn card_text(&self) -> &str {
        if let Some(inner) = &self.retweeted {
            return &inner.text;
        }
        if let Some(handle) = self.bare_repost_handle() {
            let prefix_len = "RT @".len() + handle.len() + 1;
            return self.text[prefix_len..].trim_start();
        }
        &self.text
    }

    pub fn engagement(&self) -> u64 {
        self.likes
            .saturating_add(self.reposts.saturating_mul(3))
            .saturating_add(self.replies.saturating_mul(2))
            .saturating_add(self.bookmarks.saturating_mul(5))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub handle: String,
    pub name: String,
    pub bio: String,
    pub followers: u64,
    pub following: u64,
    /// Latest posts (pinned excluded — see `pinned`).
    pub posts: Vec<Post>,
    /// The account's pinned post(s), read from the provider payload
    /// (profile-page "Pinned" marker + TweetResultByRestId). Empty
    /// when the account has no pinned post.
    #[serde(default)]
    pub pinned: Vec<Post>,
}

/// Latest posts with any pinned duplicates removed, so a pinned post
/// renders once (in the Pinned section), never twice.
pub fn exclude_pinned(latest: &[Post], pinned: &[Post]) -> Vec<Post> {
    if pinned.is_empty() {
        return latest.to_vec();
    }
    let ids: std::collections::HashSet<&str> = pinned.iter().map(|p| p.id.as_str()).collect();
    latest.iter().filter(|p| !ids.contains(p.id.as_str())).cloned().collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thread {
    pub root: Post,
    pub replies: Vec<Post>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trend {
    pub name: String,
    pub query: String,
    pub volume: Option<u64>,
    pub place: Option<String>,
    /// Provider rank (1-based) when the payload carries one.
    #[serde(default)]
    pub rank: Option<u64>,
    /// Verbatim provider context/category line (e.g. "Sports" or
    /// "Trending in United States"). Never synthesized: `None` renders
    /// no category line rather than an invented one.
    #[serde(default)]
    pub category: Option<String>,
}

/// Display buckets trends are grouped under. Derived ONLY from the
/// provider's real category/context string; anything unrecognized or
/// missing lands in `Other` — grouping never invents a topic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrendBucket {
    News,
    Politics,
    Sports,
    Entertainment,
    Technology,
    Other,
}

impl TrendBucket {
    pub fn label(self) -> &'static str {
        match self {
            TrendBucket::News => "News",
            TrendBucket::Politics => "Politics",
            TrendBucket::Sports => "Sports",
            TrendBucket::Entertainment => "Entertainment",
            TrendBucket::Technology => "Technology",
            TrendBucket::Other => "Other",
        }
    }

    /// Canonical display order for groups and chips.
    pub fn all() -> [TrendBucket; 6] {
        [
            TrendBucket::News,
            TrendBucket::Politics,
            TrendBucket::Sports,
            TrendBucket::Entertainment,
            TrendBucket::Technology,
            TrendBucket::Other,
        ]
    }
}

/// Bucket a trend by its real category string. Local/context lines like
/// "Trending in United States" carry no topic and land in Other.
pub fn trend_bucket(category: Option<&str>) -> TrendBucket {
    let Some(raw) = category else {
        return TrendBucket::Other;
    };
    let c = raw.to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| c.contains(n));
    if has(&["politic", "election", "government", "congress", "senate"]) {
        TrendBucket::Politics
    } else if has(&[
        "sport", "nfl", "nba", "mlb", "nhl", "soccer", "football", "baseball",
        "basketball", "tennis", "golf", "ufc", "wwe", "f1", "racing", "hockey",
        "olympic", "cricket", "boxing",
    ]) {
        TrendBucket::Sports
    } else if has(&["news", "breaking", "world news", "journalism"]) {
        TrendBucket::News
    } else if has(&[
        "entertainment", "music", "movie", "film", "tv", "television",
        "celebrity", "gaming", "game", "anime", "comedy", "streaming",
    ]) {
        TrendBucket::Entertainment
    } else if has(&[
        "tech", "science", "ai", "artificial intelligence", "software",
        "computing", "space", "crypto", "business", "finance",
    ]) {
        TrendBucket::Technology
    } else {
        TrendBucket::Other
    }
}

/// Trends sorted for display: provider rank order when ranks exist,
/// otherwise payload order.
pub fn sort_trends(trends: &[Trend]) -> Vec<Trend> {
    let mut out = trends.to_vec();
    out.sort_by_key(|t| t.rank.unwrap_or(u64::MAX));
    out
}

/// Group ALL trends by bucket (canonical order, empty groups omitted).
/// Every trend lands in exactly one group, so group counts sum to the
/// input length — the Top-20 overlap is presentational, not data loss.
pub fn group_trends(trends: &[Trend]) -> Vec<(TrendBucket, Vec<Trend>)> {
    let sorted = sort_trends(trends);
    TrendBucket::all()
        .into_iter()
        .filter_map(|bucket| {
            let items: Vec<Trend> = sorted
                .iter()
                .filter(|t| trend_bucket(t.category.as_deref()) == bucket)
                .cloned()
                .collect();
            (!items.is_empty()).then_some((bucket, items))
        })
        .collect()
}

/// Case-insensitive instant filter over name, query, category, and
/// place. A multi-word filter requires every word to match somewhere.
///
/// Perf (Lane-1 stack): fields are concatenated into one buffer and
/// lowercased in a single pass, instead of format! + 4 to_lowercase
/// allocations per trend (0.084ms -> 0.037-0.047ms per 500 trends). An
/// Aho-Corasick version was measured and REVERTED: rebuilding the
/// automaton on every call cost 0.239ms vs 0.084ms for this path on
/// 500 trends (examples/perf_bench) - it pays off only with a
/// persistent automaton and many patterns, not a per-keystroke
/// rebuild. Semantics unchanged; the model tests pin the behavior.
pub fn filter_trends(trends: &[Trend], filter: &str) -> Vec<Trend> {
    let words: Vec<String> = filter
        .to_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect();
    if words.is_empty() {
        return sort_trends(trends);
    }
    sort_trends(trends)
        .into_iter()
        .filter(|t| {
            let mut raw = String::with_capacity(t.name.len() + t.query.len() + 24);
            for field in [
                t.name.as_str(),
                t.query.as_str(),
                t.category.as_deref().unwrap_or_default(),
                t.place.as_deref().unwrap_or_default(),
            ] {
                raw.push_str(field);
                raw.push(' ');
            }
            let hay = raw.to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .collect()
}

/// Rank to show for a trend: the provider rank, else its 1-based
/// position in the full sorted list.
pub fn trend_rank(trend: &Trend, sorted: &[Trend]) -> u64 {
    trend.rank.unwrap_or_else(|| {
        sorted
            .iter()
            .position(|t| t.name == trend.name && t.query == trend.query)
            .map(|i| i as u64 + 1)
            .unwrap_or(0)
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchHit {
    pub source: String,
    pub title: String,
    pub url: String,
    pub excerpt: String,
    pub session: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    Punchy,
    Informative,
    Thread,
    Reply,
}

impl Tone {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "punchy" => Some(Self::Punchy),
            "informative" => Some(Self::Informative),
            "thread" => Some(Self::Thread),
            "reply" => Some(Self::Reply),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Punchy => "punchy",
            Self::Informative => "informative",
            Self::Thread => "thread",
            Self::Reply => "reply",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    pub id: String,
    pub text: String,
    pub tone: Option<String>,
    pub source: String,
    pub created_at: String,
    pub reply_to: Option<String>,
}

pub fn strip_handle(handle: &str) -> &str {
    handle.trim().trim_start_matches('@')
}

pub fn post_id_from_input(input: &str) -> String {
    let trimmed = input.trim();
    if let Some(idx) = trimmed.rfind("/status/") {
        let rest = &trimmed[idx + "/status/".len()..];
        let id: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        if !id.is_empty() {
            return id;
        }
    }
    trimmed.to_string()
}

pub fn post_url(handle: &str, id: &str) -> String {
    let handle = strip_handle(handle);
    if handle.is_empty() {
        format!("https://x.com/i/status/{id}")
    } else {
        format!("https://x.com/{handle}/status/{id}")
    }
}

pub fn rank_by_engagement(mut posts: Vec<Post>) -> Vec<Post> {
    posts.sort_by_key(|post| std::cmp::Reverse(post.engagement()));
    posts
}

pub fn trends_from_posts(posts: &[Post], place: Option<&str>) -> Vec<Trend> {
    posts
        .iter()
        .take(10)
        .map(|post| {
            let name = post
                .text
                .split_whitespace()
                .find(|word| word.starts_with('#'))
                .map(|tag| tag.trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '#'))
                .filter(|tag| !tag.is_empty())
                .map_or_else(
                    || {
                        let mut title = post.text.chars().take(48).collect::<String>();
                        if post.text.chars().count() > 48 {
                            title.push('…');
                        }
                        title
                    },
                    str::to_string,
                );
            Trend {
                query: name.clone(),
                name,
                volume: Some(post.engagement()),
                place: place.map(str::to_string),
                rank: None,
                category: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod trend_tests {
    use super::{filter_trends, group_trends, trend_bucket, Trend, TrendBucket};

    fn trend(name: &str, category: Option<&str>, rank: u64) -> Trend {
        Trend {
            name: name.into(),
            query: name.into(),
            volume: None,
            place: Some("worldwide".into()),
            rank: Some(rank),
            category: category.map(str::to_string),
        }
    }

    fn sample() -> Vec<Trend> {
        vec![
            trend("#NFLSunday", Some("Sports"), 3),
            trend("Election Night", Some("Politics"), 1),
            trend("Breaking: Storm", Some("News"), 2),
            trend("#AI", Some("Technology"), 5),
            trend("Local Fest", Some("Trending in United States"), 4),
            trend("Mystery Topic", None, 6),
        ]
    }

    #[test]
    fn bucketing_uses_real_categories_only() {
        assert_eq!(trend_bucket(Some("Sports")), TrendBucket::Sports);
        assert_eq!(trend_bucket(Some("NFL · Trending")), TrendBucket::Sports);
        assert_eq!(trend_bucket(Some("Politics")), TrendBucket::Politics);
        assert_eq!(trend_bucket(Some("Technology")), TrendBucket::Technology);
        // Context lines and missing categories are honestly Other.
        assert_eq!(
            trend_bucket(Some("Trending in United States")),
            TrendBucket::Other
        );
        assert_eq!(trend_bucket(None), TrendBucket::Other);
        assert_eq!(trend_bucket(Some("")), TrendBucket::Other);
    }

    #[test]
    fn grouping_drops_nothing_and_orders_canonically() {
        let groups = group_trends(&sample());
        let total: usize = groups.iter().map(|(_, items)| items.len()).sum();
        assert_eq!(total, 6, "every trend lands in exactly one group");
        let labels: Vec<&str> = groups.iter().map(|(b, _)| b.label()).collect();
        assert_eq!(
            labels,
            vec!["News", "Politics", "Sports", "Technology", "Other"]
        );
        // Within a group, provider rank order is kept.
        let other = &groups.last().unwrap().1;
        assert_eq!(other[0].name, "Local Fest");
        assert_eq!(other[1].name, "Mystery Topic");
    }

    #[test]
    fn filter_is_case_insensitive_and_multiword() {
        let all = sample();
        assert_eq!(filter_trends(&all, "").len(), 6);
        assert_eq!(filter_trends(&all, "nfl").len(), 1);
        assert_eq!(filter_trends(&all, "SPORTS").len(), 1);
        // Matches category text too.
        assert_eq!(filter_trends(&all, "united states").len(), 1);
        // Multi-word requires all words.
        assert_eq!(filter_trends(&all, "breaking storm").len(), 1);
        assert_eq!(filter_trends(&all, "breaking nfl").len(), 0);
    }
}
