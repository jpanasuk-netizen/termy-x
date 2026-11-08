use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::config::XConfig;
use crate::io::{CommandOutput, CommandRunner, Http, HttpRequest};
use crate::model::{
    Attempt, AttemptKind, CostEstimate, Post, Profile, ProviderId, ResearchHit, SESSION_OWN,
    Served, StatusLine, Thread, Trend, post_url, rank_by_engagement, strip_handle,
    trends_from_posts,
};
use crate::publish::{Publisher, session_label};
use std::path::PathBuf;

/// X's two home timelines. OpenCLI exposes both via
/// `twitter timeline --type for-you|following` (verified live 2026-10-18:
/// default is the algorithmic for-you feed; no cursor/pagination flag, but
/// a single `--limit 160` call returns 160 unique tweets per feed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimelineFeed {
    ForYou,
    Following,
}

impl TimelineFeed {
    pub fn label(self) -> &'static str {
        match self {
            TimelineFeed::ForYou => "For you",
            TimelineFeed::Following => "Following",
        }
    }
    pub fn opencli_value(self) -> &'static str {
        match self {
            TimelineFeed::ForYou => "for-you",
            TimelineFeed::Following => "following",
        }
    }
}

/// Merge post pages preserving order, dropping later duplicates by tweet
/// id (posts with no id are always kept). Used so a deep timeline fetch
/// can never show the same tweet twice.
/// Append a freshly fetched batch behind what is already loaded:
/// existing order is untouched, and only tweets whose id is not already
/// loaded are appended (in fetch order). This is the Load more merge —
/// the visible list can only ever grow downward with new unique tweets.
pub fn append_new_posts(existing: Vec<Post>, incoming: Vec<Post>) -> Vec<Post> {
    let mut seen: std::collections::HashSet<String> = existing
        .iter()
        .filter(|post| !post.id.is_empty())
        .map(|post| post.id.clone())
        .collect();
    let mut merged = existing;
    for post in incoming {
        if post.id.is_empty() || seen.insert(post.id.clone()) {
            merged.push(post);
        }
    }
    merged
}

pub fn dedupe_posts(posts: Vec<Post>) -> Vec<Post> {
    let mut seen = std::collections::HashSet::new();
    posts
        .into_iter()
        .filter(|post| post.id.is_empty() || seen.insert(post.id.clone()))
        .collect()
}

pub trait XProvider: Send + Sync {
    fn id(&self) -> ProviderId;
    fn available(&self) -> bool;
    fn lookup(&self, handle: &str) -> Result<Served<Profile>, String>;
    fn thread(&self, id: &str) -> Result<Served<Thread>, String>;
    /// One tweet by id. Default: the thread root (providers whose
    /// thread path already unwraps reposts get the embed for free).
    /// OpenCli overrides with a TweetResultByRestId eval, whose row
    /// carries `retweeted` when the tweet is a repost.
    fn post_by_id(&self, id: &str) -> Result<Served<Post>, String> {
        let served = self.thread(id)?;
        Ok(Served {
            value: served.value.root,
            status: served.status,
        })
    }
    fn search(&self, query: &str, recent: bool, limit: usize) -> Result<Served<Vec<Post>>, String>;
    /// Tweets that mention `handle`. Default: OpenCLI search `@handle`.
    fn mentions(&self, handle: &str, limit: usize) -> Result<Served<Vec<Post>>, String> {
        self.search(&format!("@{handle}"), false, limit)
    }
    /// Replies directed at `handle` (inbox). Default: search `to:handle`.
    fn inbox_replies(&self, handle: &str, limit: usize) -> Result<Served<Vec<Post>>, String> {
        self.search(&format!("to:{handle}"), true, limit)
    }
    fn timeline(&self, limit: usize) -> Result<Served<Vec<Post>>, String>;
    /// Feed-aware home timeline. Providers that cannot distinguish X's
    /// two home feeds must say so instead of faking one from the other.
    fn timeline_feed(
        &self,
        _feed: TimelineFeed,
        _limit: usize,
    ) -> Result<Served<Vec<Post>>, String> {
        Err("this provider does not expose separate For you / Following feeds".into())
    }
    fn trends(&self, place: Option<&str>, limit: usize) -> Result<Served<Vec<Trend>>, String>;
    /// Mentions / reply notifications for the connected account.
    fn notifications(&self, _limit: usize) -> Result<Served<Vec<Post>>, String> {
        Err("this provider has no notifications feed".into())
    }
    /// Chronological tweets for a handle (`opencli twitter tweets`).
    fn tweets(&self, handle: &str, _limit: usize) -> Result<Served<Vec<Post>>, String> {
        let served = self.lookup(handle)?;
        Ok(Served {
            value: served.value.posts,
            status: served.status,
        })
    }
    /// Follower list for a handle. Snapshot callers persist locally.
    fn followers(
        &self,
        _handle: &str,
        _limit: usize,
    ) -> Result<Served<Vec<crate::followers::Follower>>, String> {
        Err("this provider has no followers list".into())
    }
    /// One cursor-paginated slice of a home feed: `cursor: None` reads
    /// from the top; the returned `next_cursor` continues strictly older.
    /// Providers without a real cursor must Err, never fake it.
    fn timeline_page(
        &self,
        _feed: TimelineFeed,
        _cursor: Option<&str>,
        _count: usize,
    ) -> Result<Served<TimelinePage>, String> {
        Err("this provider has no cursor pagination".into())
    }
}

/// Tweets one home GraphQL call will accept. A UI page is larger than
/// this, so a page is several of these calls, not one oversized request.
pub const HOME_PROVIDER_PAGE: usize = 20;
/// Unique posts the For you / Following screen asks for on open and on
/// each Load more. Not a promise: a short provider ends the page early.
pub const HOME_UI_PAGE: usize = 100;
/// Provider calls inside one UI page. 100 / 20.
pub const HOME_MAX_HOPS: usize = HOME_UI_PAGE / HOME_PROVIDER_PAGE;

/// A page of timeline posts plus the opaque bottom cursor that fetches
/// the next (older) page. `next_cursor: None` means the source is dry.
#[derive(Debug, Clone, PartialEq)]
pub struct TimelinePage {
    pub posts: Vec<Post>,
    pub next_cursor: Option<String>,
}

/// Slice `posts` at an `off:N` cursor. `count` is clamped to one
/// provider page. A full page returns the next offset; a short page
/// returns no cursor.
pub fn slice_timeline(posts: &[Post], cursor: Option<&str>, count: usize) -> TimelinePage {
    let count = count.clamp(1, HOME_PROVIDER_PAGE);
    let start = cursor
        .and_then(|token| token.strip_prefix("off:"))
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(0)
        .min(posts.len());
    let end = (start + count).min(posts.len());
    let next_cursor = (end < posts.len()).then(|| format!("off:{end}"));
    TimelinePage {
        posts: posts[start..end].to_vec(),
        next_cursor,
    }
}

/// Walk provider pages until `target` unique posts, the cursor repeats
/// or disappears, or [`HOME_MAX_HOPS`] is spent. The whole last provider
/// page is kept, so the result can be a little over the target. It is
/// never padded when the source runs out.
pub fn accumulate_timeline_pages(
    mut fetch: impl FnMut(Option<&str>) -> TimelinePage,
    start: Option<&str>,
    target: usize,
) -> TimelinePage {
    let target = target.clamp(1, HOME_UI_PAGE);
    let mut cursor = start.map(str::to_string);
    let mut posts = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..HOME_MAX_HOPS {
        if posts.len() >= target {
            break;
        }
        let sent = cursor.clone();
        let page = fetch(cursor.as_deref());
        for post in page.posts {
            if post.id.is_empty() || !seen.insert(post.id.clone()) {
                continue;
            }
            posts.push(post);
        }
        match page.next_cursor {
            Some(next) if sent.as_ref() == Some(&next) => {
                cursor = None;
                break;
            }
            None => {
                cursor = None;
                break;
            }
            Some(next) => cursor = Some(next),
        }
    }
    TimelinePage {
        posts,
        next_cursor: cursor,
    }
}

/// Fixture home page: provider slices of [`HOME_PROVIDER_PAGE`] until
/// `target` unique posts.
pub fn accumulate_fixture(posts: &[Post], cursor: Option<&str>, target: usize) -> TimelinePage {
    accumulate_timeline_pages(
        |token| slice_timeline(posts, token, HOME_PROVIDER_PAGE),
        cursor,
        target,
    )
}

pub struct Service {
    pub readers: Vec<Box<dyn XProvider>>,
    pub publishers: Vec<Box<dyn Publisher>>,
}

impl Service {
    pub fn free(config: &XConfig, runner: Arc<dyn CommandRunner>, http: Arc<dyn Http>) -> Self {
        let mut readers: Vec<Box<dyn XProvider>> = Vec::new();
        let mut publishers: Vec<Box<dyn Publisher>> = Vec::new();
        if config.official_enabled {
            let official = OfficialProvider::new(config, http);
            publishers.push(Box::new(OfficialPublisher {
                inner: official.share(),
            }));
            readers.push(Box::new(official));
        }
        if config.provider != Some(ProviderId::Mock)
            && config.provider != Some(ProviderId::OfficialApi)
        {
            readers.push(Box::new(OpenCliProvider::new(config, runner.clone())));
            readers.push(Box::new(TwitterCliProvider::new(config, runner.clone())));
            publishers.push(Box::new(OpenCliPublisher::new(config, runner.clone())));
            publishers.push(Box::new(TwitterCliPublisher::new(config, runner)));
        }
        // Fixtures only when the config asks for them. The normal app
        // (provider unset) is OpenCLI, then twitter-cli. A miss is an
        // error, not a silent page of test posts.
        if config.provider == Some(ProviderId::Mock) {
            readers.push(Box::new(MockProvider));
        }
        if config.provider == Some(ProviderId::OpenCli) {
            readers.retain(|reader| reader.id() == ProviderId::OpenCli);
            publishers.retain(|publisher| publisher.id() == ProviderId::OpenCli);
        }
        if config.provider == Some(ProviderId::TwitterCli) {
            readers.retain(|reader| reader.id() == ProviderId::TwitterCli);
            publishers.retain(|publisher| publisher.id() == ProviderId::TwitterCli);
        }
        if config.provider == Some(ProviderId::OfficialApi) {
            readers.retain(|reader| reader.id() == ProviderId::OfficialApi);
            publishers.retain(|publisher| publisher.id() == ProviderId::OfficialApi);
        }
        Self {
            readers,
            publishers,
        }
    }

    pub fn reader_ids(&self) -> Vec<ProviderId> {
        self.readers.iter().map(|reader| reader.id()).collect()
    }

    pub fn publisher_ids(&self) -> Vec<ProviderId> {
        self.publishers
            .iter()
            .map(|publisher| publisher.id())
            .collect()
    }

    fn read<T>(
        &self,
        call: impl Fn(&dyn XProvider) -> Result<Served<T>, String>,
    ) -> Result<Served<T>, String> {
        let mut attempts = Vec::new();
        let mut last_error = "no provider available".to_string();
        for reader in &self.readers {
            if !reader.available() {
                attempts.push(Attempt {
                    provider: reader.id(),
                    kind: AttemptKind::Skipped,
                    detail: "not installed".to_string(),
                });
                continue;
            }
            match call(reader.as_ref()) {
                Ok(mut served) => {
                    attempts.push(Attempt {
                        provider: reader.id(),
                        kind: AttemptKind::Served,
                        detail: served.status.note.clone(),
                    });
                    served.status.attempts = attempts;
                    return Ok(served);
                }
                Err(error) => {
                    last_error = error.clone();
                    attempts.push(Attempt {
                        provider: reader.id(),
                        kind: AttemptKind::Failed,
                        detail: error,
                    });
                }
            }
        }
        Err(format!("{last_error} ({})", render_attempts(&attempts)))
    }

    pub fn lookup(&self, handle: &str) -> Result<Served<Profile>, String> {
        let handle = strip_handle(handle).to_string();
        self.read(|reader| reader.lookup(&handle))
    }

    pub fn thread(&self, id: &str) -> Result<Served<Thread>, String> {
        let id = crate::model::post_id_from_input(id);
        self.read(|reader| reader.thread(&id))
    }

    pub fn post_by_id(&self, id: &str) -> Result<Served<Post>, String> {
        let id = crate::model::post_id_from_input(id);
        self.read(|reader| reader.post_by_id(&id))
    }

    /// Fill in the embedded original for bare reposts (`RT @handle:`
    /// rows with no `retweeted` payload) by fetching each repost by id
    /// through the existing by-id path. `cache` maps repost id ->
    /// resolved original (None = tried, could not resolve); cached ids
    /// are never fetched twice. At most `max_fetches` new fetches run
    /// per call so a feed full of reposts cannot stall a load; call
    /// again to drain the rest. Returns how many posts got their
    /// original filled in.
    pub fn resolve_bare_reposts(
        &self,
        posts: &mut [Post],
        cache: &mut std::collections::HashMap<String, Option<Post>>,
        max_fetches: usize,
    ) -> usize {
        let mut filled = 0;
        let mut fetches = 0;
        for post in posts.iter_mut() {
            if post.retweeted.is_some() || post.unavailable_original_url.is_some() {
                continue;
            }
            if post.bare_repost_handle().is_none() && post.referenced_status_id.is_none() {
                continue;
            }
            if !cache.contains_key(&post.id) {
                if fetches >= max_fetches {
                    continue;
                }
                fetches += 1;
                let fetch_id = post.original_fetch_id().unwrap_or(post.id.as_str()).to_string();
                let resolved = self.post_by_id(&fetch_id).ok().and_then(|served| {
                    Post::adopt_fetched_original(&post.id, served.value)
                });
                if resolved.is_none() {
                    post.unavailable_original_url = post.original_link();
                }
                cache.insert(post.id.clone(), resolved);
            }
            match cache.get(&post.id) {
                Some(Some(original)) => {
                    post.retweeted = Some(Box::new(original.clone()));
                    post.unavailable_original_url = None;
                    filled += 1;
                }
                Some(None) if post.unavailable_original_url.is_none() => {
                    post.unavailable_original_url = post.original_link();
                }
                _ => {}
            }
        }
        filled
    }

    pub fn search(
        &self,
        query: &str,
        recent: bool,
        limit: usize,
    ) -> Result<Served<Vec<Post>>, String> {
        self.read(|reader| reader.search(query, recent, limit))
    }

    pub fn mentions(&self, handle: &str, limit: usize) -> Result<Served<Vec<Post>>, String> {
        let handle = strip_handle(handle);
        self.read(|reader| reader.mentions(handle, limit))
    }

    pub fn inbox_replies(&self, handle: &str, limit: usize) -> Result<Served<Vec<Post>>, String> {
        let handle = strip_handle(handle);
        self.read(|reader| reader.inbox_replies(handle, limit))
    }

    pub fn timeline(&self, limit: usize) -> Result<Served<Vec<Post>>, String> {
        self.read(|reader| reader.timeline(limit))
    }

    pub fn notifications(&self, limit: usize) -> Result<Served<Vec<Post>>, String> {
        self.read(|reader| reader.notifications(limit))
    }

    pub fn tweets(&self, handle: &str, limit: usize) -> Result<Served<Vec<Post>>, String> {
        let handle = strip_handle(handle).to_string();
        self.read(|reader| reader.tweets(&handle, limit))
    }

    pub fn followers(
        &self,
        handle: &str,
        limit: usize,
    ) -> Result<Served<Vec<crate::followers::Follower>>, String> {
        let handle = strip_handle(handle).to_string();
        self.read(|reader| reader.followers(&handle, limit))
    }

    /// Inbox rows: notifications when the provider has them, else
    /// `to:<me>` search. Errors stay errors; empty is a valid result.
    pub fn inbox_posts(&self, me: &str, limit: usize) -> Result<Served<Vec<Post>>, String> {
        let limit = limit.clamp(1, 50);
        let notes = self.notifications(limit);
        let directed = self.search(&crate::inbox::to_me_query(me), true, limit);
        match (notes, directed) {
            (Ok(mut notes), Ok(directed)) => {
                notes.value = crate::inbox::merge_inbox_posts(notes.value, directed.value);
                if notes.value.is_empty() && directed.status.note.is_empty() {
                    return Ok(notes);
                }
                Ok(notes)
            }
            (Ok(mut notes), Err(_)) => {
                notes.value = crate::inbox::merge_inbox_posts(notes.value, Vec::new());
                Ok(notes)
            }
            (Err(_), Ok(mut directed)) => {
                directed.value = crate::inbox::merge_inbox_posts(Vec::new(), directed.value);
                Ok(directed)
            }
            (Err(a), Err(b)) => Err(format!("{a}; {b}")),
        }
    }

    pub fn mentions_posts(&self, me: &str, limit: usize) -> Result<Served<Vec<Post>>, String> {
        let limit = limit.clamp(1, 50);
        let mut served = self.search(&crate::inbox::mentions_query(me), true, limit)?;
        crate::time::sort_newest(&mut served.value, |post| post.created_at.as_deref());
        Ok(served)
    }

    pub fn timeline_feed(
        &self,
        feed: TimelineFeed,
        limit: usize,
    ) -> Result<Served<Vec<Post>>, String> {
        let mut served = self.read(|reader| reader.timeline_feed(feed, limit))?;
        served.value = dedupe_posts(served.value);
        Ok(served)
    }

    pub fn timeline_page(
        &self,
        feed: TimelineFeed,
        cursor: Option<&str>,
        count: usize,
    ) -> Result<Served<TimelinePage>, String> {
        let mut served = self.read(|reader| reader.timeline_page(feed, cursor, count))?;
        served.value.posts = dedupe_posts(served.value.posts);
        Ok(served)
    }

    pub fn popular(&self, topic: &str, limit: usize) -> Result<Served<Vec<Post>>, String> {
        let mut served = self.search(topic, false, limit.max(10))?;
        served.value = rank_by_engagement(served.value);
        served.value.truncate(limit);
        let prior = served.status.note.clone();
        let label = if prior.is_empty() {
            "top posts"
        } else {
            prior.as_str()
        };
        served.status.note = format!("{label} · ranked by engagement");
        Ok(served)
    }

    pub fn trends(&self, place: Option<&str>, limit: usize) -> Result<Served<Vec<Trend>>, String> {
        let mut attempts = Vec::new();
        for reader in &self.readers {
            if reader.id() == ProviderId::Mock {
                continue;
            }
            if !reader.available() {
                attempts.push(Attempt {
                    provider: reader.id(),
                    kind: AttemptKind::Skipped,
                    detail: "not installed".into(),
                });
                continue;
            }
            match reader.trends(place, limit) {
                Ok(mut served) => {
                    served.status.attempts = attempts;
                    return Ok(served);
                }
                Err(error) => attempts.push(Attempt {
                    provider: reader.id(),
                    kind: AttemptKind::Failed,
                    detail: error,
                }),
            }
        }
        let topic = place.unwrap_or("news");
        if let Ok(mut posts) = self.search(topic, true, limit)
            && posts.status.provider != ProviderId::Mock
        {
            let trends =
                trends_from_posts(&rank_by_engagement(std::mem::take(&mut posts.value)), place);
            posts.status.note = "derived from search, ranked by engagement".to_string();
            posts.status.attempts.extend(attempts);
            return Ok(Served {
                value: trends,
                status: posts.status,
            });
        }
        let mut fixtures = self.read(|reader| reader.trends(place, limit))?;
        fixtures.status.attempts.extend(attempts);
        Ok(fixtures)
    }

    pub fn research(
        &self,
        topic: &str,
        runner: &dyn CommandRunner,
        config: &XConfig,
    ) -> Vec<ResearchHit> {
        research_topic(topic, runner, config)
    }
}

fn render_attempts(attempts: &[Attempt]) -> String {
    if attempts.is_empty() {
        return "no attempts".to_string();
    }
    attempts
        .iter()
        .map(|attempt| format!("{:?} {}", attempt.kind, attempt.provider.as_str()))
        .collect::<Vec<_>>()
        .join(", ")
}

pub struct MockProvider;

impl XProvider for MockProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Mock
    }
    fn available(&self) -> bool {
        true
    }
    fn lookup(&self, handle: &str) -> Result<Served<Profile>, String> {
        let handle = strip_handle(handle);
        let profile = fixtures()
            .into_iter()
            .find(|profile| profile.handle.eq_ignore_ascii_case(handle))
            .unwrap_or_else(|| Profile {
                handle: handle.to_string(),
                name: format!("Fixture {handle}"),
                bio: "Mock profile. No live session and no X API credits.".to_string(),
                followers: Some(128),
                following: Some(40),
                tweets: Some(2),
                likes: None,
                verified: None,
                created_at: None,
                avatar_url: None,
                banner_url: None,
                viewer_following: None,
                posts: sample_posts()
                    .into_iter()
                    .skip(1)
                    .take(2)
                    .map(|mut post| {
                        post.author_handle = handle.to_string();
                        post
                    })
                    .collect(),
                pinned: sample_posts()
                    .into_iter()
                    .take(1)
                    .map(|mut post| {
                        post.author_handle = handle.to_string();
                        post
                    })
                    .collect(),
            });
        Ok(served(profile, "fixture profile"))
    }
    fn thread(&self, id: &str) -> Result<Served<Thread>, String> {
        Ok(served(sample_thread(id), "fixture thread"))
    }
    fn search(
        &self,
        query: &str,
        _recent: bool,
        limit: usize,
    ) -> Result<Served<Vec<Post>>, String> {
        let mut posts = sample_posts();
        let needle = query.to_ascii_lowercase();
        posts.retain(|post| {
            post.text.to_ascii_lowercase().contains(&needle)
                || post.author_handle.to_ascii_lowercase().contains(&needle)
                || needle.is_empty()
        });
        if posts.is_empty() {
            posts = sample_posts();
            for post in &mut posts {
                post.text = format!("{query}: {}", post.text);
            }
        }
        posts.truncate(limit);
        Ok(served(posts, "fixture search"))
    }
    fn mentions(&self, handle: &str, limit: usize) -> Result<Served<Vec<Post>>, String> {
        let mut posts = sample_mentions(handle);
        posts.truncate(limit);
        Ok(served(posts, "fixture mentions"))
    }
    fn inbox_replies(&self, handle: &str, limit: usize) -> Result<Served<Vec<Post>>, String> {
        let mut posts = sample_inbox(handle);
        posts.truncate(limit);
        Ok(served(posts, "fixture inbox"))
    }
    fn timeline(&self, limit: usize) -> Result<Served<Vec<Post>>, String> {
        let mut posts = sample_posts();
        posts.truncate(limit);
        Ok(served(posts, "fixture timeline"))
    }
    fn timeline_feed(&self, feed: TimelineFeed, limit: usize) -> Result<Served<Vec<Post>>, String> {
        let posts = if limit <= HOME_PROVIDER_PAGE {
            slice_timeline(&home_fixture(feed), None, limit).posts
        } else {
            accumulate_fixture(&home_fixture(feed), None, limit.min(HOME_UI_PAGE)).posts
        };
        Ok(served(
            posts,
            match feed {
                TimelineFeed::ForYou => "fixture timeline (for you)",
                TimelineFeed::Following => "fixture timeline (following)",
            },
        ))
    }
    fn timeline_page(
        &self,
        feed: TimelineFeed,
        cursor: Option<&str>,
        count: usize,
    ) -> Result<Served<TimelinePage>, String> {
        Ok(served(
            accumulate_fixture(&home_fixture(feed), cursor, count),
            match feed {
                TimelineFeed::ForYou => "fixture timeline page (for you)",
                TimelineFeed::Following => "fixture timeline page (following)",
            },
        ))
    }
    fn notifications(&self, limit: usize) -> Result<Served<Vec<Post>>, String> {
        let mut posts = sample_posts();
        posts.truncate(limit.max(1));
        Ok(served(posts, "fixture notifications"))
    }
    fn tweets(&self, handle: &str, limit: usize) -> Result<Served<Vec<Post>>, String> {
        let handle = strip_handle(handle);
        let mut posts = sample_posts();
        let now = crate::time::unix_now();
        for (i, post) in posts.iter_mut().enumerate() {
            post.author_handle = handle.to_string();
            if post.created_at.is_none() {
                let ts = now.saturating_sub((i as i64) * 86_400);
                post.created_at = Some(format!("{}T12:00:00Z", crate::time::ymd(ts)));
            }
        }
        posts.truncate(limit.max(1));
        Ok(served(posts, "fixture tweets"))
    }
    fn followers(
        &self,
        handle: &str,
        limit: usize,
    ) -> Result<Served<Vec<crate::followers::Follower>>, String> {
        let _ = handle;
        let mut rows = vec![
            crate::followers::Follower {
                handle: "ada".into(),
                name: "Ada Lovelace".into(),
            },
            crate::followers::Follower {
                handle: "blue_swift".into(),
                name: "Blue Swift".into(),
            },
            crate::followers::Follower {
                handle: "panel_notes".into(),
                name: "Panel Notes".into(),
            },
        ];
        rows.truncate(limit.max(1));
        Ok(served(rows, "fixture followers"))
    }
    fn trends(&self, place: Option<&str>, limit: usize) -> Result<Served<Vec<Trend>>, String> {
        // Fixture set spans the real category shapes the live payload
        // carries (category + rank), enough to exercise Top-20 + groups.
        let place = place.map(str::to_string);
        let mk = |name: &str, category: &str, rank: u64, volume: u64| Trend {
            name: name.into(),
            query: name.into(),
            volume: Some(volume),
            place: place.clone(),
            rank: Some(rank),
            category: Some(category.into()),
        };
        let mut trends = vec![
            mk("Election Night Live", "Politics", 1, 182_000),
            mk("#TermyX", "Technology", 2, 2_400),
            mk("Storm Front Hits Coast", "News", 3, 96_500),
            mk("#NFLSunday", "Sports", 4, 210_300),
            mk("GPUI", "Technology", 5, 860),
            mk("Award Show Moments", "Entertainment", 6, 74_200),
            mk("Senate Vote Watch", "Politics", 7, 41_900),
            mk("weighted counts", "Technology", 8, 140),
            mk("Derby Day", "Sports", 9, 58_700),
            mk("Morning Headlines", "News", 10, 33_100),
            mk("Indie Game Launch", "Entertainment", 11, 12_400),
            mk("Rocket Launch Window", "Technology", 12, 27_800),
            mk("City Marathon", "Sports", 13, 9_300),
            mk("Policy Town Hall", "Politics", 14, 6_100),
            mk("Festival Weekend", "Trending in United States", 15, 4_400),
            mk("Local Bakery Fame", "Trending in United States", 16, 1_150),
            mk("#RustLang", "Technology", 17, 3_900),
            mk("Championship Final", "Sports", 18, 88_900),
            mk("Late Night Interview", "Entertainment", 19, 5_600),
            mk("Market Open Thread", "Technology", 20, 7_700),
            mk("School Board Recall", "Politics", 21, 2_050),
            mk("River Festival", "Trending in United States", 22, 640),
            mk("Comet Watch Party", "News", 23, 1_480),
            mk("Esports Grand Final", "Entertainment", 24, 19_600),
        ];
        trends.truncate(limit);
        Ok(served(trends, "fixture trends"))
    }
}

fn served<T>(value: T, note: &str) -> Served<T> {
    Served {
        value,
        status: StatusLine::free(ProviderId::Mock, None, note),
    }
}

pub fn sample_posts() -> Vec<Post> {
    let mut posts = vec![
        post(
            "1001",
            "termy",
            "Termy",
            "Termy X is in the side panel. Timeline, search, and a compose box that counts URLs as 23. https://termy.sh",
            128,
            24,
            16,
            9,
        ),
        post(
            "1002",
            "blue_swift",
            "Blue Swift",
            "Low-poly swift, navy field, cyan wireframe, magenta heart. Original bird, not a logo. +1",
            86,
            11,
            7,
            4,
        ),
        post(
            "1003",
            "panel_notes",
            "Panel Notes",
            "Free path: OpenCLI session, then twitter-cli cookie, then fixtures. Official API stays off.",
            64,
            8,
            5,
            2,
        ),
        post(
            "1004",
            "draft_desk",
            "Draft Desk",
            "AI writes variants. A human still has to confirm the exact text before anything is posted.",
            41,
            6,
            9,
            3,
        ),
        // A repost: outer author is the reposter, the embedded tweet
        // is the original — cards render the original inline.
        {
            let mut repost = post(
                "1005",
                "swift_fan",
                "Swift Fan",
                "RT @blue_swift: Low-poly swift, navy field, cyan wireframe, magenta heart. Original bird, not a logo. +1",
                0,
                0,
                0,
                0,
            );
            repost.retweeted = Some(Box::new(post(
                "1002",
                "blue_swift",
                "Blue Swift",
                "Low-poly swift, navy field, cyan wireframe, magenta heart. Original bird, not a logo. +1",
                86,
                11,
                7,
                4,
            )));
            repost
        },
    ];
    let stamps = [
        "2026-10-01T12:00:00Z",
        "2026-10-03T12:00:00Z",
        "2026-10-05T12:00:00Z",
        "2026-10-07T18:00:00Z",
        "2026-10-08T09:00:00Z",
    ];
    for (post, stamp) in posts.iter_mut().zip(stamps) {
        post.created_at = Some(stamp.into());
        if let Some(inner) = post.retweeted.as_mut() {
            inner.created_at = Some("2026-10-03T12:00:00Z".into());
        }
    }
    posts
}

/// Enough posts for two UI pages of [`HOME_UI_PAGE`] plus a short tail.
/// Likes fall as the index rises; timestamps rise, so Popular and Newest
/// disagree.
pub fn home_fixture(feed: TimelineFeed) -> Vec<Post> {
    let tag = match feed {
        TimelineFeed::ForYou => "fy",
        TimelineFeed::Following => "fl",
    };
    const N: usize = 250;
    (0..N)
        .map(|index| {
            let id = format!("{tag}-{index:03}");
            let mut item = post(
                &id,
                "blue_swift",
                "Blue Swift",
                &format!("{tag} home post {index}"),
                (N - index) as u64,
                1,
                0,
                0,
            );
            let day = 1 + (index / 24);
            let hour = index % 24;
            item.created_at = Some(format!("2026-10-{day:02}T{hour:02}:00:00Z"));
            item
        })
        .collect()
}

fn sample_mentions(handle: &str) -> Vec<Post> {
    let handle = strip_handle(handle);
    let tag = if handle.is_empty() { "termy" } else { handle };
    mention_posts(
        tag,
        &[
            (
                "3101",
                "blue_swift",
                "Blue Swift",
                120,
                2,
                "2026-09-01T10:00:00Z",
                "popular mention of the panel",
            ),
            (
                "3102",
                "panel_notes",
                "Panel Notes",
                44,
                1,
                "2026-10-20T08:00:00Z",
                "mid mention, fewer likes",
            ),
            (
                "3103",
                "draft_desk",
                "Draft Desk",
                6,
                0,
                "2026-10-27T16:30:00Z",
                "newest mention, fewest likes",
            ),
        ],
    )
}

fn sample_inbox(handle: &str) -> Vec<Post> {
    let handle = strip_handle(handle);
    let tag = if handle.is_empty() { "termy" } else { handle };
    mention_posts(
        tag,
        &[
            (
                "4101",
                "swift_fan",
                "Swift Fan",
                55,
                4,
                "2026-09-12T11:00:00Z",
                "inbox reply with the most likes",
            ),
            (
                "4102",
                "blue_swift",
                "Blue Swift",
                18,
                2,
                "2026-10-18T09:00:00Z",
                "mid inbox reply",
            ),
            (
                "4103",
                "panel_notes",
                "Panel Notes",
                3,
                1,
                "2026-10-27T17:00:00Z",
                "newest inbox reply",
            ),
        ],
    )
}

fn mention_posts(handle: &str, rows: &[(&str, &str, &str, u64, u64, &str, &str)]) -> Vec<Post> {
    rows.iter()
        .map(|(id, author, name, likes, replies, created, text)| {
            let mut item = post(
                id,
                author,
                name,
                &format!("@{handle} {text}"),
                *likes,
                1,
                *replies,
                0,
            );
            item.created_at = Some((*created).into());
            item.in_reply_to = Some("1001".into());
            item
        })
        .collect()
}

fn fixtures() -> Vec<Profile> {
    vec![Profile {
        handle: "termy".into(),
        name: "Termy".into(),
        bio: "Native terminal. The X panel is a fork feature for Jeremy.".into(),
        followers: Some(4096),
        following: Some(120),
        tweets: Some(5),
        likes: Some(88),
        verified: Some(false),
        created_at: Some("2024-01-15T00:00:00Z".into()),
        avatar_url: None,
        banner_url: None,
        viewer_following: Some(false),
        posts: sample_posts(),
        pinned: Vec::new(),
    }]
}

/// Nested fixture used by [`MockProvider::thread`]. Direct replies plus
/// a chain deep enough to exercise the indent cap.
pub fn sample_thread(id: &str) -> Thread {
    let posts = sample_posts();
    let mut root = posts
        .iter()
        .find(|post| post.id == id)
        .cloned()
        .unwrap_or_else(|| {
            let mut post = posts[0].clone();
            post.id = id.to_string();
            post.url = post_url(&post.author_handle, id);
            post
        });
    root.in_reply_to = None;
    Thread {
        root: root.clone(),
        replies: sample_nested_replies(&root.id),
    }
}

fn sample_nested_replies(root_id: &str) -> Vec<Post> {
    let reply = |id: &str,
                 handle: &str,
                 name: &str,
                 text: &str,
                 likes: u64,
                 created: &str,
                 parent: &str| {
        let mut post = post(id, handle, name, text, likes, 1, 1, 0);
        post.created_at = Some(created.into());
        post.in_reply_to = Some(parent.into());
        post
    };
    let mut liked = reply(
        "2006",
        "draft_desk",
        "Draft Desk",
        "Already-liked sibling (fewer likes, newer).",
        20,
        "2026-10-06T16:00:00Z",
        root_id,
    );
    liked.viewer_liked = Some(true);
    liked.viewer_follows_author = Some(true);
    let mut popular = reply(
        "2001",
        "blue_swift",
        "Blue Swift",
        "Direct reply with the most likes.",
        80,
        "2026-10-06T14:00:00Z",
        root_id,
    );
    popular.viewer_liked = Some(false);
    popular.viewer_follows_author = Some(false);
    vec![
        popular,
        reply(
            "2002",
            "panel_notes",
            "Panel Notes",
            "Nested under the popular reply.",
            12,
            "2026-10-06T14:30:00Z",
            "2001",
        ),
        reply(
            "2003",
            "draft_desk",
            "Draft Desk",
            "Third level in the chain.",
            4,
            "2026-10-06T14:45:00Z",
            "2002",
        ),
        reply(
            "2004",
            "swift_fan",
            "Swift Fan",
            "Fourth level.",
            2,
            "2026-10-06T14:50:00Z",
            "2003",
        ),
        reply(
            "2005",
            "termy",
            "Termy",
            "Fifth level — indent caps at four.",
            1,
            "2026-10-06T14:55:00Z",
            "2004",
        ),
        liked,
    ]
}

fn post(
    id: &str,
    handle: &str,
    name: &str,
    text: &str,
    likes: u64,
    reposts: u64,
    replies: u64,
    bookmarks: u64,
) -> Post {
    Post {
        id: id.into(),
        url: post_url(handle, id),
        author_handle: handle.into(),
        author_name: name.into(),
        text: text.into(),
        created_at: Some("2026-10-06T13:00:00Z".into()),
        likes,
        reposts,
        replies,
        bookmarks,
        views: likes.saturating_mul(20),
        quotes: None,
        media: Vec::new(),
        retweeted: None,
        quoted: None,
        referenced_status_id: None,
        unavailable_original_url: None,
        in_reply_to: None,
        viewer_liked: None,
        viewer_follows_author: None,
    }
}

struct Pace {
    interval: Duration,
    last: Mutex<Option<Instant>>,
}

impl Pace {
    fn new(ms: u64) -> Self {
        Self {
            interval: Duration::from_millis(ms),
            last: Mutex::new(None),
        }
    }

    fn wait(&self) {
        if self.interval.is_zero() {
            return;
        }
        let mut last = self.last.lock().expect("pace");
        if let Some(prev) = *last {
            let rest = self.interval.saturating_sub(prev.elapsed());
            if !rest.is_zero() {
                std::thread::sleep(rest);
            }
        }
        *last = Some(Instant::now());
    }
}

struct OpenCliProvider {
    bin: String,
    runner: Arc<dyn CommandRunner>,
    pace: Pace,
}

impl OpenCliProvider {
    fn new(config: &XConfig, runner: Arc<dyn CommandRunner>) -> Self {
        Self {
            bin: config.opencli_bin.clone(),
            runner,
            pace: Pace::new(config.min_interval_ms),
        }
    }

    fn call(&self, args: &[String]) -> Result<String, String> {
        self.pace.wait();
        // Bounded: OpenCLI's own browser timeout is ~60s; a closed Chrome
        // must fail in well under that, not the old 180s runner default.
        let output =
            self.runner
                .run_timeout(&self.bin, args, std::time::Duration::from_secs(45))?;
        if output.status != 0 {
            return Err(if output.stderr.is_empty() {
                format!("opencli exited {}", output.status)
            } else {
                output.stderr
            });
        }
        Ok(output.stdout)
    }

    /// Run one `opencli browser termyx …` subcommand with its own budget.
    fn browser_cmd(
        &self,
        args: &[&str],
        timeout: Duration,
    ) -> Result<crate::io::CommandOutput, String> {
        self.pace.wait();
        let owned: Vec<String> = args.iter().map(ToString::to_string).collect();
        self.runner.run_timeout(&self.bin, &owned, timeout)
    }

    /// Cursor-paginated timeline through the user's own logged-in X web
    /// session: one background OpenCLI browser session, one in-page eval
    /// that walks HomeTimeline/HomeLatestTimeline GraphQL pages with the
    /// bottom cursor (the same calls x.com's own infinite scroll makes).
    /// The CSRF cookie is read and used only inside the page; nothing but
    /// posts + the next cursor ever leaves it.
    fn eval_timeline_page(
        &self,
        feed: TimelineFeed,
        cursor: Option<&str>,
        count: usize,
    ) -> Result<TimelinePage, String> {
        let count = count.clamp(1, HOME_UI_PAGE);
        let stdout = self.eval_js("https://x.com/home", &timeline_eval_js(feed, cursor, count))?;
        parse_timeline_eval(&stdout)
    }

    /// One silent background browser session that runs `js` in a
    /// logged-in x.com page and returns its stdout. Shared by the
    /// timeline walker and the profile (pinned/latest) walker; the
    /// page's cookies never leave the page.
    fn eval_js(&self, open_url: &str, js: &str) -> Result<String, String> {
        let session = "termyx";
        let open = self.browser_cmd(
            &[
                "browser",
                session,
                "open",
                open_url,
                "--window",
                "background",
            ],
            Duration::from_secs(60),
        )?;
        if open.status != 0 {
            return Err(format!(
                "could not open the background X session: {}",
                open.stderr.trim()
            ));
        }
        // Warm-up: wait until the session tab is actually on x.com
        // (open returns before the page has loaded; an eval that lands
        // on about:blank can neither read ct0 nor fetch relatively).
        for _ in 0..15 {
            match self.browser_cmd(
                // No '|' here: on Windows this arg travels through
                // cmd.exe (opencli.cmd shim), which would treat the
                // pipe as a command separator and break the probe.
                &["browser", session, "eval", "location.href"],
                Duration::from_secs(30),
            ) {
                Ok(probe)
                    if probe.stdout.contains("https://x.com")
                        && !probe.stdout.trim_start().starts_with("loading") =>
                {
                    break;
                }
                _ => std::thread::sleep(Duration::from_millis(800)),
            }
        }
        // Ship the script in chunks: on Windows every arg travels
        // through `cmd.exe /c opencli.cmd ...`, whose command line is
        // capped at 8191 chars — one base64 blob for this script is
        // ~9.4 KB and cmd refuses it ("The command line is too
        // long"), which is exactly how this path used to die before
        // evaluating anything. Append <=6000-char pieces to a window
        // global instead, then decode+run it in one final eval.
        let b64 = b64_encode(js.as_bytes());
        let mut shipped: Result<(), String> = Ok(());
        for (i, chunk) in b64.as_bytes().chunks(6000).enumerate() {
            let piece = std::str::from_utf8(chunk).expect("base64 ascii");
            let arg = if i == 0 {
                format!("window.__tx='{piece}'")
            } else {
                format!("window.__tx+='{piece}'")
            };
            match self.browser_cmd(&["browser", session, "eval", &arg], Duration::from_secs(30)) {
                Ok(out) if out.status == 0 => {}
                Ok(out) => {
                    shipped = Err(format!(
                        "timeline session exited {} while uploading the script: {}",
                        out.status,
                        out.stderr.trim()
                    ));
                    break;
                }
                Err(e) => {
                    shipped = Err(e);
                    break;
                }
            }
        }
        let eval = match shipped {
            Ok(()) => self.browser_cmd(
                &["browser", session, "eval", "eval(atob(window.__tx))"],
                Duration::from_secs(150),
            ),
            Err(e) => Err(e),
        };
        let _ = self.browser_cmd(&["browser", session, "close"], Duration::from_secs(30));
        let eval = eval?;
        if eval.status != 0 {
            return Err(if eval.stderr.trim().is_empty() {
                format!("browser session exited {}", eval.status)
            } else {
                eval.stderr.trim().to_string()
            });
        }
        Ok(eval.stdout)
    }

    /// Latest + pinned posts for one profile, from the same silent
    /// browser GraphQL path as the timeline. Best-effort: any failure
    /// returns empty vecs rather than failing the profile lookup.
    fn eval_profile_posts(&self, handle: &str, want: usize) -> (Vec<Post>, Vec<Post>) {
        let url = format!("https://x.com/{handle}");
        match self.eval_js(&url, &profile_eval_js(handle, want)) {
            Ok(stdout) => parse_profile_eval(&stdout).unwrap_or_default(),
            Err(error) => {
                log_raw_provider_error("profile eval", &error);
                (Vec::new(), Vec::new())
            }
        }
    }
}

impl XProvider for OpenCliProvider {
    fn id(&self) -> ProviderId {
        ProviderId::OpenCli
    }
    fn available(&self) -> bool {
        self.runner.exists(&self.bin)
    }
    fn lookup(&self, handle: &str) -> Result<Served<Profile>, String> {
        let stdout = self.call(&opencli_read(&[
            "twitter".into(),
            "profile".into(),
            handle.into(),
        ]))?;
        let mut profile = parse_profile(&stdout, handle)?;
        // Latest posts: `twitter tweets` is chronological and already
        // excludes the pinned post (its GraphQL walk skips pin
        // entries). A tweets failure (protected/suspended account)
        // keeps the profile and just shows no Latest section.
        let latest = self
            .call(&opencli_read(&[
                "twitter".into(),
                "tweets".into(),
                handle.into(),
                "--limit".into(),
                "120".into(),
                "--page-delay".into(),
                "1".into(),
            ]))
            .and_then(|out| parse_posts(&out))
            .unwrap_or_else(|error| {
                log_raw_provider_error("lookup tweets", &error);
                Vec::new()
            });
        // Pinned + eval latest come from the silent GraphQL session;
        // eval posts are also the fallback when the CLI tweets call
        // came back empty.
        let (eval_latest, pinned) = self.eval_profile_posts(handle, 30);
        let mut latest = if latest.is_empty() {
            eval_latest
        } else {
            latest
        };
        latest = crate::model::exclude_pinned(&latest, &pinned);
        profile.posts = dedupe_posts(latest);
        profile.pinned = pinned;
        Ok(session_served(ProviderId::OpenCli, profile, "profile"))
    }
    fn thread(&self, id: &str) -> Result<Served<Thread>, String> {
        let stdout = self.call(&opencli_read(&[
            "twitter".into(),
            "thread".into(),
            id.into(),
            "--limit".into(),
            "50".into(),
        ]))?;
        Ok(session_served(
            ProviderId::OpenCli,
            parse_thread(&stdout, id)?,
            "thread",
        ))
    }
    fn post_by_id(&self, id: &str) -> Result<Served<Post>, String> {
        // TweetResultByRestId through the silent session: the row for a
        // repost carries the embedded original (`retweeted`), which is
        // exactly what bare-repost resolution needs.
        let stdout = self.eval_js("https://x.com/home", &tweet_by_id_eval_js(id))?;
        Ok(session_served(
            ProviderId::OpenCli,
            parse_tweet_by_id_eval(&stdout)?,
            "tweet by id",
        ))
    }
    fn search(&self, query: &str, recent: bool, limit: usize) -> Result<Served<Vec<Post>>, String> {
        let stdout = self.call(&opencli_read(&[
            "twitter".into(),
            "search".into(),
            query.into(),
            "--filter".into(),
            if recent { "live" } else { "top" }.into(),
            "--limit".into(),
            limit.to_string(),
        ]))?;
        Ok(session_served(
            ProviderId::OpenCli,
            parse_posts(&stdout)?,
            "search",
        ))
    }
    fn timeline(&self, limit: usize) -> Result<Served<Vec<Post>>, String> {
        let stdout = self.call(&opencli_read(&[
            "twitter".into(),
            "timeline".into(),
            "--limit".into(),
            limit.to_string(),
        ]))?;
        Ok(session_served(
            ProviderId::OpenCli,
            parse_posts(&stdout)?,
            "timeline",
        ))
    }
    fn timeline_feed(&self, feed: TimelineFeed, limit: usize) -> Result<Served<Vec<Post>>, String> {
        let stdout = self.call(&opencli_read(&[
            "twitter".into(),
            "timeline".into(),
            "--type".into(),
            feed.opencli_value().into(),
            "--limit".into(),
            limit.to_string(),
        ]))?;
        Ok(session_served(
            ProviderId::OpenCli,
            parse_posts(&stdout)?,
            match feed {
                TimelineFeed::ForYou => "timeline · for you",
                TimelineFeed::Following => "timeline · following",
            },
        ))
    }
    fn timeline_page(
        &self,
        feed: TimelineFeed,
        cursor: Option<&str>,
        count: usize,
    ) -> Result<Served<TimelinePage>, String> {
        let page = self.eval_timeline_page(feed, cursor, count)?;
        Ok(session_served(
            ProviderId::OpenCli,
            page,
            match feed {
                TimelineFeed::ForYou => "timeline page · for you",
                TimelineFeed::Following => "timeline page · following",
            },
        ))
    }
    fn trends(&self, place: Option<&str>, limit: usize) -> Result<Served<Vec<Trend>>, String> {
        let args = opencli_read(&[
            "twitter".into(),
            "trending".into(),
            "--limit".into(),
            limit.to_string(),
        ]);
        let stdout = self.call(&args)?;
        let trends = parse_trends(&stdout, place)
            .or_else(|_| Ok::<_, String>(trends_from_posts(&parse_posts(&stdout)?, place)))?;
        if trends.is_empty() {
            return Err("opencli trending returned no rows".into());
        }
        Ok(session_served(ProviderId::OpenCli, trends, "trending"))
    }
    fn notifications(&self, limit: usize) -> Result<Served<Vec<Post>>, String> {
        let stdout = self.call(&opencli_read(&[
            "twitter".into(),
            "notifications".into(),
            "--limit".into(),
            limit.max(1).to_string(),
        ]))?;
        let posts = parse_posts(&stdout).unwrap_or_default();
        Ok(session_served(ProviderId::OpenCli, posts, "notifications"))
    }
    fn tweets(&self, handle: &str, limit: usize) -> Result<Served<Vec<Post>>, String> {
        let handle = strip_handle(handle);
        let stdout = self.call(&opencli_read(&[
            "twitter".into(),
            "tweets".into(),
            handle.into(),
            "--limit".into(),
            limit.clamp(1, 200).to_string(),
            "--page-delay".into(),
            "1".into(),
        ]))?;
        Ok(session_served(
            ProviderId::OpenCli,
            parse_posts(&stdout).unwrap_or_default(),
            "tweets",
        ))
    }
    fn followers(
        &self,
        handle: &str,
        limit: usize,
    ) -> Result<Served<Vec<crate::followers::Follower>>, String> {
        let handle = strip_handle(handle);
        let stdout = self.call(&opencli_read(&[
            "twitter".into(),
            "followers".into(),
            handle.into(),
            "--limit".into(),
            limit.clamp(1, 200).to_string(),
        ]))?;
        Ok(session_served(
            ProviderId::OpenCli,
            crate::followers::parse_followers(&stdout).unwrap_or_default(),
            "followers",
        ))
    }
}

struct TwitterCliProvider {
    bin: String,
    runner: Arc<dyn CommandRunner>,
    pace: Pace,
}

impl TwitterCliProvider {
    fn new(config: &XConfig, runner: Arc<dyn CommandRunner>) -> Self {
        Self {
            bin: config.twitter_bin.clone(),
            runner,
            pace: Pace::new(config.min_interval_ms),
        }
    }

    fn call(&self, args: &[String]) -> Result<String, String> {
        self.pace.wait();
        let output =
            self.runner
                .run_timeout(&self.bin, args, std::time::Duration::from_secs(45))?;
        if output.status != 0 {
            return Err(if output.stderr.is_empty() {
                format!("twitter exited {}", output.status)
            } else {
                output.stderr
            });
        }
        Ok(output.stdout)
    }
}

impl XProvider for TwitterCliProvider {
    fn id(&self) -> ProviderId {
        ProviderId::TwitterCli
    }
    fn available(&self) -> bool {
        self.runner.exists(&self.bin)
    }
    fn lookup(&self, handle: &str) -> Result<Served<Profile>, String> {
        let stdout = self.call(&["user".into(), handle.into(), "--json".into()])?;
        Ok(session_served(
            ProviderId::TwitterCli,
            parse_profile(&stdout, handle)?,
            "user",
        ))
    }
    fn thread(&self, id: &str) -> Result<Served<Thread>, String> {
        let stdout = self.call(&["tweet".into(), id.into(), "--json".into()])?;
        Ok(session_served(
            ProviderId::TwitterCli,
            parse_thread(&stdout, id)?,
            "tweet",
        ))
    }
    fn search(&self, query: &str, recent: bool, limit: usize) -> Result<Served<Vec<Post>>, String> {
        let stdout = self.call(&[
            "search".into(),
            query.into(),
            "-t".into(),
            if recent { "latest" } else { "top" }.into(),
            "--max".into(),
            limit.to_string(),
            "--json".into(),
        ])?;
        Ok(session_served(
            ProviderId::TwitterCli,
            parse_posts(&stdout)?,
            "search",
        ))
    }
    fn timeline(&self, limit: usize) -> Result<Served<Vec<Post>>, String> {
        let stdout = self.call(&[
            "feed".into(),
            "--max".into(),
            limit.to_string(),
            "--json".into(),
        ])?;
        Ok(session_served(
            ProviderId::TwitterCli,
            parse_posts(&stdout)?,
            "feed",
        ))
    }
    fn trends(&self, _place: Option<&str>, _limit: usize) -> Result<Served<Vec<Trend>>, String> {
        Err("twitter-cli has no trends endpoint".into())
    }
}

fn session_served<T>(provider: ProviderId, value: T, note: &str) -> Served<T> {
    Served {
        value,
        status: StatusLine::free(provider, Some(SESSION_OWN), note),
    }
}

/// Raw provider failures go here (same file as the panel perf log:
/// %TEMP%/termy-x-perf.txt), never to the panel. The panel only ever
/// sees `publish::humanize_error` output.
fn log_raw_provider_error(context: &str, raw: &str) {
    let flat = raw.replace(['\n', '\r'], " ");
    write_perf_line(&format!("provider-error {context}: {flat}"));
}

/// One line in %TEMP%/termy-x-perf.txt (stage timings live here so a
/// slow write can be split probe / CLI / envelope after the fact).
fn write_perf_line(line: &str) {
    use std::io::Write;
    let path = std::env::temp_dir().join("termy-x-perf.txt");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{line}");
    }
}

/// OpenCLI success contract for write commands: exit 0 AND no failure
/// marker in the JSON payload. Exit-0-with-ok:false happens (the CLI
/// prints a JSON error envelope), so status alone is not proof.
fn check_opencli_output(output: &CommandOutput, context: &str) -> Result<String, String> {
    let combined = format!("{}\n{}", output.stdout, output.stderr);
    if output.status != 0 {
        log_raw_provider_error(context, &combined);
        return Err(crate::publish::humanize_error(&combined));
    }
    if let Ok(value) = serde_json::from_str::<Value>(output.stdout.trim()) {
        let failure = json_failure_message(&value);
        if let Some(message) = failure {
            log_raw_provider_error(context, &combined);
            return Err(crate::publish::humanize_error(&message));
        }
    }
    Ok(output.stdout.clone())
}

/// A parsed OpenCLI payload that reports failure: `ok:false`, or a
/// `status` of error/failed, at the top level or in the first row.
fn json_failure_message(value: &Value) -> Option<String> {
    let node = value
        .as_array()
        .and_then(|rows| rows.first())
        .unwrap_or(value);
    if node.get("ok").and_then(Value::as_bool) == Some(false) {
        return Some(value.to_string());
    }
    let status = node.get("status").and_then(Value::as_str).unwrap_or("");
    if matches!(status, "error" | "failed" | "failure") {
        return Some(value.to_string());
    }
    None
}

const QUOTE_SESSION: &str = "termyx-quote";
const QUOTE_READ_SESSION: &str = "termyx-quote-read";

fn sanitize_browser_note(raw: &str) -> String {
    let collapsed = raw.split_whitespace().take(12).collect::<Vec<_>>().join(" ");
    let lower = collapsed.to_ascii_lowercase();
    if lower.contains("token") || lower.contains("cookie") || lower.contains("authorization") {
        return "browser error omitted".into();
    }
    let mut out = String::new();
    for ch in collapsed.chars().take(80) {
        if ch != '{' && ch != '}' {
            out.push(ch);
        }
    }
    out
}

fn close_quote_session(
    bin: &str,
    runner: &dyn crate::io::CommandRunner,
    cli: crate::publish::BrowserCli,
    session: &str,
) -> String {
    // Session-scoped tabs only. Extra pages are closed here. The last page
    // stays so OpenCLI's close-owned-tab can ungroup and remove that lease
    // tab. Closing it first leaves the OpenCLI Browser placeholder, which
    // is the leftover group. Personal Chrome tabs are not in this session.
    // Do not call windows.update, windows.remove, or tabs.remove.
    let list_args = crate::publish::quote_browser_args(cli, session, &["tab", "list"], None, false);
    if let Ok(listed) = quote_run(bin, runner, &list_args, Duration::from_secs(15)) {
        if listed.status == 0 {
            let mut pages = crate::publish::quote_session_pages(&listed.stdout);
            if !pages.is_empty() {
                pages.pop();
            }
            for page in pages.iter().rev() {
                let close_tab = crate::publish::quote_browser_args(
                    cli,
                    session,
                    &["tab", "close", page],
                    None,
                    false,
                );
                let _ = quote_run(bin, runner, &close_tab, Duration::from_secs(15));
            }
        }
    }
    // Documented session release. OPENCLI_CLOSE_OWNED_TAB asks the extension
    // to drop this session's tab instead of leaving a blank placeholder.
    // It does not call windows.update or windows.remove.
    let previous = std::env::var("OPENCLI_CLOSE_OWNED_TAB").ok();
    unsafe {
        std::env::set_var("OPENCLI_CLOSE_OWNED_TAB", "1");
    }
    // The lease release that exited 0 was `browser close --session <name>`
    // with OPENCLI_CLOSE_OWNED_TAB=1, and its data said the tab lease was
    // released. Exit 75 is that exit code, not a protected-resource label.
    // The positional session form is the other shape this CLI may document.
    let session_form = vec![
        "browser".to_string(),
        "close".to_string(),
        "--session".into(),
        session.to_string(),
    ];
    let positional = crate::publish::browser_command(cli, session, "close", None);
    let mut attempts = vec![session_form];
    if positional != attempts[0] {
        attempts.push(positional);
    }
    let mut note = "session close unread".to_string();
    for args in attempts {
        match quote_run(bin, runner, &args, Duration::from_secs(15)) {
            Ok(output) if output.status == 0 => {
                note = crate::publish::quote_close_note(0, &output.stdout);
                break;
            }
            Ok(output) => note = crate::publish::quote_close_note(output.status, &output.stdout),
            Err(_) => {}
        }
    }
    unsafe {
        match previous {
            Some(value) => std::env::set_var("OPENCLI_CLOSE_OWNED_TAB", value),
            None => std::env::remove_var("OPENCLI_CLOSE_OWNED_TAB"),
        }
    }
    note
}

fn browser_cli_for(
    bin: &str,
    runner: &dyn crate::io::CommandRunner,
) -> Result<crate::publish::BrowserCli, String> {
    let output = runner
        .run_keeping_window(
            bin,
            &["browser".into(), "--help".into()],
            Duration::from_secs(20),
        )
        .map_err(|error| {
            crate::publish::explain_opencli_failure(
                "bridge",
                "browser --help",
                -1,
                "",
                &sanitize_browser_note(&error),
            )
        })?;
    let help = format!("{}\n{}", output.stdout, output.stderr);
    if help.trim().is_empty() {
        return Err(crate::publish::explain_opencli_failure(
            "bridge",
            "browser --help",
            output.status,
            "",
            "",
        ));
    }
    Ok(crate::publish::browser_cli_from_help(&help))
}

fn browser_page_json(
    bin: &str,
    runner: &dyn crate::io::CommandRunner,
    cli: crate::publish::BrowserCli,
    session: &str,
    url: Option<&str>,
    js: &str,
    marker: &str,
    keep_window: bool,
    and_close: bool,
) -> Result<crate::publish::QuotePageReport, String> {
    let run = |args: &[String], timeout: Duration| {
        if keep_window {
            runner.run_keeping_window(bin, args, timeout)
        } else {
            runner.run_timeout(bin, args, timeout)
        }
    };
    if let Some(url) = url {
    let open_args = if keep_window {
        crate::publish::quote_browser_args(cli, session, &["open", url], None, true)
    } else {
        crate::publish::browser_command(cli, session, "open", Some(url))
    };
    let open_public = crate::publish::browser_argv_public(&open_args);
    let opened = match run(&open_args, Duration::from_secs(60)) {
        Ok(output) => output,
        Err(error) => {
            return Err(crate::publish::explain_opencli_failure(
                "navigation",
                &open_public,
                -1,
                "",
                &error,
            ));
        }
    };
    if opened.status != 0 {
        close_quote_session(bin, runner, cli, session);
        return Err(crate::publish::explain_opencli_failure(
            "navigation",
            &open_public,
            opened.status,
            &opened.stdout,
            &opened.stderr,
        ));
    }
    for _ in 0..8 {
        match run(
            &crate::publish::browser_command(cli, session, "eval", Some("location.href")),
            Duration::from_secs(20),
        ) {
            Ok(probe) if probe.stdout.contains("https://x.com") => break,
            _ => std::thread::sleep(Duration::from_millis(200)),
        }
    }
    }
    let b64 = b64_encode(js.as_bytes());
    for (index, chunk) in b64.as_bytes().chunks(6000).enumerate() {
        let piece = std::str::from_utf8(chunk).unwrap_or("");
        let arg = if index == 0 {
            format!("window.__{marker}='{piece}'")
        } else {
            format!("window.__{marker}+='{piece}'")
        };
        let loaded = run(
            &crate::publish::browser_command(cli, session, "eval", Some(&arg)),
            Duration::from_secs(30),
        );
        if loaded.map(|output| output.status != 0).unwrap_or(true) {
            close_quote_session(bin, runner, cli, session);
            return Err(
                "stage editor · the quote page opened but the check script did not load · Post was not clicked"
                    .into(),
            );
        }
    }
    let eval_args =
        crate::publish::browser_command(
            cli,
            session,
            "eval",
            Some(&format!("eval(atob(window.__{marker}))")),
        );
    let eval_public = crate::publish::browser_argv_public(&eval_args);
    let evaluated = run(&eval_args, Duration::from_secs(90));
    let evaluated = match evaluated {
        Ok(output) => output,
        Err(error) => {
            close_quote_session(bin, runner, cli, session);
            return Err(crate::publish::explain_opencli_failure(
                "editor",
                &eval_public,
                -1,
                "",
                &error,
            ));
        }
    };
    if evaluated.status != 0 {
        close_quote_session(bin, runner, cli, session);
        return Err(crate::publish::explain_opencli_failure(
            "editor",
            &eval_public,
            evaluated.status,
            &evaluated.stdout,
            &evaluated.stderr,
        ));
    }
    let mut last = evaluated.stdout;
    let mut tries = 0;
    let report = loop {
        tries += 1;
        if tries > 8 {
            close_quote_session(bin, runner, cli, session);
            return match crate::publish::parse_quote_page_report(&last) {
                Err(error) => Err(error),
                Ok(_) => Err("stage result · eval stayed empty · Post was not clicked".into()),
            };
        }
        match crate::publish::parse_quote_page_report(&last) {
            Ok(report) => break report,
            Err(error) if !crate::publish::eval_needs_poll(&last) => {
                close_quote_session(bin, runner, cli, session);
                return Err(error);
            }
            Err(error) => {
                let poll = crate::publish::browser_command(
                    cli,
                    session,
                    "eval",
                    Some("window.__qxReport?JSON.stringify(window.__qxReport):\"pending\""),
                );
                let polled = run(&poll, Duration::from_secs(20));
                let polled = match polled {
                    Ok(output) if output.status == 0 => output.stdout,
                    Ok(output) => {
                        close_quote_session(bin, runner, cli, session);
                        return Err(crate::publish::explain_opencli_failure(
                            "result",
                            "browser eval",
                            output.status,
                            &output.stdout,
                            &output.stderr,
                        ));
                    }
                    Err(poll_error) => {
                        close_quote_session(bin, runner, cli, session);
                        return Err(crate::publish::explain_opencli_failure(
                            "result",
                            "browser eval",
                            -1,
                            "",
                            &poll_error,
                        ));
                    }
                };
                if polled == last {
                    close_quote_session(bin, runner, cli, session);
                    return Err(error);
                }
                last = polled;
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    };
    if and_close {
        close_quote_session(bin, runner, cli, session);
    }
    Ok(report)
}

fn quote_run(
    bin: &str,
    runner: &dyn crate::io::CommandRunner,
    args: &[String],
    timeout: Duration,
) -> Result<crate::io::CommandOutput, String> {
    runner.run_keeping_window(bin, args, timeout)
}

struct QuoteSurface {
    target: Option<String>,
    trace: String,
}

fn quote_surface_view(tabs: &str, target: &str, field: &str, opened: &str, click: &str) -> String {
    crate::publish::quote_view_line(tabs, target, field, opened, click)
}

fn with_quote_view(line: String, view: &str) -> String {
    if view.is_empty() || line.contains("tabs before") {
        line
    } else {
        format!("{line} · {view}")
    }
}

/// Open the status in the owned quote session and record whether that
/// page is actually visible. Does not click.
fn open_visible_quote_page(
    bin: &str,
    runner: &dyn crate::io::CommandRunner,
    cli: crate::publish::BrowserCli,
    session: &str,
    url: &str,
    status_id: &str,
) -> Result<QuoteSurface, String> {
    let list_args = crate::publish::quote_browser_args(cli, session, &["tab", "list"], None, false);
    let listed = quote_run(bin, runner, &list_args, Duration::from_secs(20));
    let tabs_before = match listed {
        Ok(output) if output.status == 0 => crate::publish::quote_tab_count(&output.stdout)
            .map(|count| count.to_string())
            .unwrap_or_else(|| "unread".into()),
        _ => "unread".into(),
    };
    let open_args = crate::publish::quote_browser_args(cli, session, &["open", url], None, true);
    let open_public = crate::publish::browser_argv_public(&open_args);
    let opened = match quote_run(bin, runner, &open_args, Duration::from_secs(60)) {
        Ok(output) => output,
        Err(error) => {
            return Err(with_quote_view(
                crate::publish::explain_opencli_failure("navigation", &open_public, -1, "", &error),
                &quote_surface_view(&tabs_before, "none", "absent", "not-read", "not-sent"),
            ));
        }
    };
    if opened.status != 0 || opened.stdout.contains("\"ok\":false") || opened.stdout.contains("\"ok\": false")
    {
        close_quote_session(bin, runner, cli, session);
        return Err(with_quote_view(
            crate::publish::explain_opencli_failure(
                "navigation",
                &open_public,
                opened.status,
                &opened.stdout,
                &opened.stderr,
            ),
            &quote_surface_view(&tabs_before, "none", "absent", "not-read", "not-sent"),
        ));
    }
    let parsed = crate::publish::quote_surface_target(&opened.stdout);
    let Some(parsed) = parsed else {
        close_quote_session(bin, runner, cli, session);
        return Err(format!(
            "stage open · {} · {} · native click was not sent · Post was not clicked",
            quote_surface_view(&tabs_before, "none", "absent", "not-read", "not-sent"),
            crate::publish::quote_open_shape(&opened.stdout)
        ));
    };
    let target_label = parsed.id.clone();
    let target_field = parsed.field;
    let target = Some(parsed.id);
    let after_open = read_quote_visibility(bin, runner, cli, session, target.as_deref(), status_id);
    let select_args = crate::publish::quote_browser_args(
        cli,
        session,
        &["tab", "select", &target_label],
        None,
        true,
    );
    let select_public = crate::publish::browser_argv_public(&select_args);
    let selected = match quote_run(bin, runner, &select_args, Duration::from_secs(20)) {
        Ok(output) => output,
        Err(error) => {
            close_quote_session(bin, runner, cli, session);
            return Err(format!(
                "stage select · open {open_public} ok page {target_label} field {target_field} · visibility after open {} · select {select_public} unread · {} · native click was not sent · Post was not clicked",
                after_open.summary,
                crate::publish::explain_opencli_failure("tab", &select_public, -1, "", &error)
            ));
        }
    };
    let ack = crate::publish::quote_select_ack(&selected.stdout);
    let select_note = if selected.status != 0 { format!("exit {}", selected.status) } else { ack.note };
    if selected.status != 0 || !ack.ok {
        close_quote_session(bin, runner, cli, session);
        return Err(format!(
            "stage select · open {open_public} ok page {target_label} field {target_field} · visibility after open {} · select {select_public} {select_note} · native click was not sent · Post was not clicked",
            after_open.summary
        ));
    }
    if let Some(selected_id) = ack.selected.as_deref() {
        if selected_id != target_label {
            close_quote_session(bin, runner, cli, session);
            return Err(format!(
                "stage select · open {open_public} ok page {target_label} field {target_field} · visibility after open {} · select {select_public} ok selected {selected_id} · selected mismatch · native click was not sent · Post was not clicked",
                after_open.summary
            ));
        }
    }
    let mut eval_location = "eval location unread".to_string();
    for _ in 0..8 {
        let probe_args = crate::publish::quote_browser_args(
            cli,
            session,
            &["eval", "location.href"],
            target.as_deref(),
            true,
        );
        match quote_run(bin, runner, &probe_args, Duration::from_secs(20)) {
            Ok(probe) if probe.status == 0 => {
                eval_location = crate::publish::quote_eval_location(&probe.stdout, status_id);
                if probe.stdout.contains("https://x.com") {
                    break;
                }
            }
            _ => std::thread::sleep(Duration::from_millis(200)),
        }
    }
    let after_select = read_quote_visibility(bin, runner, cli, session, target.as_deref(), status_id);
    let echoed = crate::publish::quote_landed_query(&opened.stdout, status_id);
    let trace = format!(
        "tabs before {tabs_before} · open {open_public} ok page {target_label} field {target_field} · {echoed} · visibility after open {} · select {select_public} {select_note} · {eval_location} · visibility after select {}",
        after_open.summary, after_select.summary
    );
    if !after_select.ready {
        let container = quote_container_note(bin, runner, status_id);
        close_quote_session(bin, runner, cli, session);
        return Err(format!(
            "stage visibility · {trace} · {container} · native click was not sent · Post was not clicked"
        ));
    }
    Ok(QuoteSurface {
        target,
        trace,
    })
}

fn chrome_call_args(method: &str, json_args: &str) -> Vec<String> {
    vec![
        "browser".into(),
        "chrome".into(),
        "call".into(),
        method.into(),
        json_args.into(),
    ]
}

fn quote_inspect_screenshot(
    bin: &str,
    runner: &dyn crate::io::CommandRunner,
    cli: crate::publish::BrowserCli,
    session: &str,
    tab: Option<&str>,
) -> String {
    let dir = crate::config::default_config_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return "screenshot unread".into();
    }
    let path = dir.join("quote-inspect.png");
    let path_text = path.to_string_lossy().to_string();
    let args = crate::publish::quote_browser_args(cli, session, &["screenshot", &path_text], tab, true);
    match quote_run(bin, runner, &args, Duration::from_secs(20)) {
        Ok(output) if output.status == 0 => format!("screenshot saved {path_text}"),
        _ => "screenshot unread".into(),
    }
}

fn quote_owned_leftover(bin: &str, runner: &dyn crate::io::CommandRunner) -> String {
    let listed = quote_run(
        bin,
        runner,
        &chrome_call_args("tabGroups.query", r#"[{"title":"OpenCLI Browser"}]"#),
        Duration::from_secs(15),
    );
    let Ok(listed) = listed else {
        return "owned group unread".into();
    };
    if listed.status != 0 {
        return "owned group unread".into();
    }
    let ids = crate::publish::quote_owned_group_ids(&listed.stdout);
    if ids.is_empty() {
        return "owned group OpenCLI Browser groups 0 tabs 0".into();
    }
    let mut tabs = 0usize;
    for id in &ids {
        let queried = quote_run(
            bin,
            runner,
            &chrome_call_args("tabs.query", &format!(r#"[{{"groupId":{id}}}]"#)),
            Duration::from_secs(15),
        );
        if let Ok(output) = queried {
            if output.status == 0 {
                tabs += crate::publish::quote_tab_count(&output.stdout).unwrap_or(0);
            }
        }
    }
    format!(
        "owned group OpenCLI Browser groups {} tabs {tabs} · session list is not chrome clean",
        ids.len()
    )
}

/// Read-only Chrome facts for the status tab. `windows.update` is not called:
/// OpenCLI 2.0.2 rejects it for the owned group with managed_resource_conflict.
fn quote_container_note(
    bin: &str,
    runner: &dyn crate::io::CommandRunner,
    status_id: &str,
) -> String {
    let listed = quote_run(
        bin,
        runner,
        &chrome_call_args("tabs.query", "[{}]"),
        Duration::from_secs(20),
    );
    let Ok(listed) = listed else {
        return "chrome windows unread · windows.update not called".into();
    };
    if listed.status != 0 {
        return "chrome windows unread · windows.update not called".into();
    }
    let tabs = crate::publish::quote_status_tabs(&listed.stdout, status_id);
    if tabs.len() != 1 {
        return format!("chrome match {} · windows.update not called", tabs.len());
    }
    let tab = &tabs[0];
    let group = if let Some(group_id) = tab.group_id {
        match quote_run(
            bin,
            runner,
            &chrome_call_args("tabGroups.get", &format!("[{group_id}]")),
            Duration::from_secs(20),
        ) {
            Ok(output) if output.status == 0 => crate::publish::quote_group_name(&output.stdout),
            _ => "unread",
        }
    } else {
        "none"
    };
    let window = quote_run(
        bin,
        runner,
        &chrome_call_args("windows.get", &format!("[{}]", tab.window_id)),
        Duration::from_secs(20),
    );
    let (state, focused, bounds) = match window {
        Ok(output) if output.status == 0 => {
            let parsed = crate::publish::quote_window_state(&output.stdout);
            let bounds = crate::publish::quote_window_bounds(&output.stdout);
            match parsed {
                Some((state, focused)) => (
                    state.to_string(),
                    if focused { "yes" } else { "no" }.to_string(),
                    bounds,
                ),
                None => ("unread".into(), "unread".into(), bounds),
            }
        }
        _ => ("unread".into(), "unread".into(), "absent"),
    };
    let note = if bounds == "parked" {
        " · close only the Chrome window with the OpenCLI Browser group · it is off the desktop · the next inspect opens a new window on the desktop"
    } else {
        ""
    };
    format!(
        "chrome match 1 active {} group {group} window state {state} focused {focused} bounds {bounds} · windows.update not called{note}",
        if tab.active { "yes" } else { "no" }
    )
}

fn eval_quote_script_keep_open(
    bin: &str,
    runner: &dyn crate::io::CommandRunner,
    cli: crate::publish::BrowserCli,
    session: &str,
    js: &str,
    marker: &str,
) -> Result<crate::publish::QuotePageReport, String> {
    let b64 = b64_encode(js.as_bytes());
    for (index, chunk) in b64.as_bytes().chunks(6000).enumerate() {
        let piece = std::str::from_utf8(chunk).unwrap_or("");
        let arg = if index == 0 {
            format!("window.__{marker}='{piece}'")
        } else {
            format!("window.__{marker}+='{piece}'")
        };
        let loaded = quote_run(
            bin,
            runner,
            &crate::publish::browser_command(cli, session, "eval", Some(&arg)),
            Duration::from_secs(30),
        );
        if loaded.map(|output| output.status != 0).unwrap_or(true) {
            return Err("script load unread".into());
        }
    }
    let output = quote_run(
        bin,
        runner,
        &crate::publish::browser_command(
            cli,
            session,
            "eval",
            Some(&format!("eval(atob(window.__{marker}))")),
        ),
        Duration::from_secs(30),
    )
    .map_err(|_| "script eval unread".to_string())?;
    if output.status != 0 {
        return Err("script eval status".into());
    }
    crate::publish::parse_quote_page_report(&output.stdout)
}

fn quote_native_opener(
    bin: &str,
    runner: &dyn crate::io::CommandRunner,
    cli: crate::publish::BrowserCli,
    tab: Option<&str>,
    status_id: &str,
) -> Result<String, String> {
    let mut notes = Vec::new();
    for testid in ["retweet", "unretweet"] {
        let args = crate::publish::quote_opener_click_args(cli, QUOTE_SESSION, status_id, testid, tab);
        match quote_run(bin, runner, &args, Duration::from_secs(20)) {
            Ok(output) if crate::publish::quote_click_trusted(&output.stdout) => {
                return Ok(format!("opener {testid} {}", crate::publish::quote_click_note(&output.stdout)));
            }
            Ok(output) => notes.push(format!("opener {testid} {}", crate::publish::quote_click_note(&output.stdout))),
            Err(_) => notes.push(format!("opener {testid} click unread")),
        }
    }
    Err(notes.join(" · "))
}

fn read_quote_visibility(
    bin: &str,
    runner: &dyn crate::io::CommandRunner,
    cli: crate::publish::BrowserCli,
    session: &str,
    tab: Option<&str>,
    status_id: &str,
) -> crate::publish::QuoteVisibilityRead {
    let expr = crate::publish::quote_visibility_expr(status_id);
    let args = crate::publish::quote_browser_args(cli, session, &["eval", &expr], tab, true);
    match quote_run(bin, runner, &args, Duration::from_secs(20)) {
        Ok(output) if output.status == 0 => crate::publish::quote_visibility_read(&output.stdout),
        _ => crate::publish::quote_visibility_read(""),
    }
}

fn run_quote_attachment(
    bin: &str,
    runner: &dyn crate::io::CommandRunner,
    target_id: &str,
    text: &str,
    inspect: bool,
) -> Result<crate::publish::QuotePageReport, String> {
    let cli = browser_cli_for(bin, runner)?;
    let page = crate::publish::quote_status_url(target_id);
    let surface = open_visible_quote_page(bin, runner, cli, QUOTE_SESSION, &page, target_id)?;
    let click_visibility = read_quote_visibility(
        bin,
        runner,
        cli,
        QUOTE_SESSION,
        surface.target.as_deref(),
        target_id,
    );
    let view = format!("{} · visibility before composer {}", surface.trace, click_visibility.summary);
    if !click_visibility.ready {
        close_quote_session(bin, runner, cli, QUOTE_SESSION);
        return Err(format!(
            "stage visibility · {view} · native click was not sent · Post was not clicked"
        ));
    }
    let source = eval_quote_script_keep_open(
        bin,
        runner,
        cli,
        QUOTE_SESSION,
        &crate::publish::quote_source_script(target_id),
        "qxSource",
    );
    let (screen, source_text, source_note) = match source {
        Ok(report) if !report.source_screen.is_empty() && !report.source_text.is_empty() => (
            report.source_screen.clone(),
            report.source_text.clone(),
            format!("source screen {} text yes chars {}", report.source_screen, report.source_text.chars().count()),
        ),
        Ok(_) => (String::new(), String::new(), "source missing".into()),
        Err(_) => (String::new(), String::new(), "source unread".into()),
    };
    let opener = match quote_native_opener(bin, runner, cli, surface.target.as_deref(), target_id) {
        Ok(note) => note,
        Err(note) => {
            let shot = quote_inspect_screenshot(bin, runner, cli, QUOTE_SESSION, surface.target.as_deref());
            let close_note = close_quote_session(bin, runner, cli, QUOTE_SESSION);
            let leftover = quote_owned_leftover(bin, runner);
            return Err(format!(
                "stage menu · {view} · {note} · {shot} · {close_note} · {leftover} · Post was not clicked"
            ));
        }
    };
    let menu_js = crate::publish::quote_menu_read_script(target_id);
    let menu_note = match browser_page_json(
        bin,
        runner,
        cli,
        QUOTE_SESSION,
        None,
        &menu_js,
        "qx",
        true,
        false,
    ) {
        Ok(report) => report.message,
        Err(error) => error,
    };
    let quote_args = crate::publish::quote_native_click_args(cli, QUOTE_SESSION, surface.target.as_deref());
    let quote_click = quote_run(bin, runner, &quote_args, Duration::from_secs(20));
    let quote_note = match &quote_click {
        Ok(output) => crate::publish::quote_click_note(&output.stdout),
        Err(_) => "click unread".into(),
    };
    let quote_trusted = matches!(quote_note.as_str(), "click cdp" | "click ax");
    let snap_js = crate::publish::quote_snap_script_bound(target_id, &screen, &source_text);
    let mut after = browser_page_json(
        bin,
        runner,
        cli,
        QUOTE_SESSION,
        None,
        &snap_js,
        "qxSnap",
        true,
        false,
    )?;
    if !crate::publish::quote_inspect_ready(&after) {
        for _ in 0..16 {
            std::thread::sleep(Duration::from_millis(250));
            let again = crate::publish::browser_command(
                cli,
                QUOTE_SESSION,
                "eval",
                Some("eval(atob(window.__qxSnap))"),
            );
            let Ok(output) = quote_run(bin, runner, &again, Duration::from_secs(20)) else {
                break;
            };
            if output.status != 0 {
                break;
            }
            if let Ok(parsed) = crate::publish::parse_quote_page_report(&output.stdout) {
                after = parsed;
                if crate::publish::quote_inspect_ready(&after) {
                    break;
                }
            }
        }
    }
    let ready = crate::publish::quote_inspect_ready(&after) && quote_trusted;
    if inspect || !ready {
        let shot = quote_inspect_screenshot(bin, runner, cli, QUOTE_SESSION, surface.target.as_deref());
        let close_note = close_quote_session(bin, runner, cli, QUOTE_SESSION);
        let leftover = quote_owned_leftover(bin, runner);
        let line = format!(
            "{} · {source_note} · {opener} · menu {menu_note} · quote {quote_note} · {shot} · {close_note} · {leftover}",
            with_quote_view(crate::publish::quote_report_line(&after), &view)
        );
        return if ready {
            Ok(after)
        } else {
            Err(line)
        };
    }
    let post_script = crate::publish::quote_type_and_post_script_bound(target_id, text, &screen, &source_text);
    let posted = browser_page_json(
        bin,
        runner,
        cli,
        QUOTE_SESSION,
        None,
        &post_script,
        "qxPost",
        true,
        false,
    )?;
    let report = posted;
    let write_failure = |report: &crate::publish::QuotePageReport| {
        let shot = quote_inspect_screenshot(bin, runner, cli, QUOTE_SESSION, surface.target.as_deref());
        let close_note = close_quote_session(bin, runner, cli, QUOTE_SESSION);
        let leftover = quote_owned_leftover(bin, runner);
        format!(
            "{} · {shot} · {close_note} · {leftover}",
            with_quote_view(crate::publish::quote_report_line(report), &view)
        )
    };
    if report.clicked_post
        && (report.quote_testids == 0 || report.composer_kind == "reply" || !report.card)
    {
        let detail = match crate::publish::quote_outcome(
            report.new_status_id.as_deref(),
            &report,
            target_id,
        ) {
            Err(error) => error,
            Ok(()) => "Quote evidence was ambiguous. Not marked sent. Nothing was retried.".into(),
        };
        return Err(format!("{detail} · {}", write_failure(&report)));
    }
    if report.clicked_repost_confirm
        || !report.clicked_post
        || !report.card
        || report.quote_testids == 0
        || report.composer_kind != "quote"
        || report.quoted_status_id.as_deref() != Some(target_id)
        || report.message != "Quote tweet posted successfully."
    {
        return Err(write_failure(&report));
    }
    let Some(new_id) = report.new_status_id.clone().filter(|id| {
        !id.is_empty() && id.chars().all(|ch| ch.is_ascii_digit()) && id != target_id
    }) else {
        return Err(format!(
            "Post was clicked once but the sent toast had no new status id. Not marked sent. Nothing was retried. · {}",
            write_failure(&report)
        ));
    };
    let read_page = format!("https://x.com/i/status/{new_id}");
    let read_script = crate::publish::quote_read_script_bound(target_id, &screen, &source_text, text);
    let read = match browser_page_json(
        bin,
        runner,
        cli,
        QUOTE_READ_SESSION,
        Some(&read_page),
        &read_script,
        "qx",
        false,
        true,
    ) {
        Ok(read) => read,
        Err(error) => {
            return Err(format!("{error} · {}", write_failure(&report)));
        }
    };
    if let Err(error) = crate::publish::quote_outcome(Some(&new_id), &read, target_id) {
        return Err(format!("{error} · {}", write_failure(&report)));
    }
    let _ = close_quote_session(bin, runner, cli, QUOTE_SESSION);
    Ok(report)
}

pub fn inspect_quote_attachment(
    bin: &str,
    runner: &dyn crate::io::CommandRunner,
    target_id: &str,
) -> Result<String, String> {
    let report = run_quote_attachment(bin, runner, target_id, "", true)?;
    let line = crate::publish::quote_report_line(&report);
    if crate::publish::quote_inspect_ready(&report) {
        Ok(line)
    } else {
        Err(line)
    }
}

struct OpenCliPublisher {
    bin: String,
    runner: Arc<dyn CommandRunner>,
    post_ok: Mutex<Option<bool>>,
}

impl OpenCliPublisher {
    fn new(config: &XConfig, runner: Arc<dyn CommandRunner>) -> Self {
        Self {
            bin: config.opencli_bin.clone(),
            runner,
            post_ok: Mutex::new(None),
        }
    }
}

impl Publisher for OpenCliPublisher {
    fn id(&self) -> ProviderId {
        ProviderId::OpenCli
    }
    fn available(&self) -> bool {
        posting_supported(
            &self.post_ok,
            self.runner.as_ref(),
            &self.bin,
            &["twitter".into(), "--help".into()],
        )
    }
    fn session_note(&self) -> Option<String> {
        session_label(ProviderId::OpenCli)
    }
    fn supports_media(&self) -> bool {
        true
    }

    fn supports_follow(&self) -> bool {
        true
    }

    fn publish(
        &self,
        parts: &[String],
        reply_to: Option<&str>,
        media: &[PathBuf],
    ) -> Result<String, String> {
        crate::publish::validate_media_paths(media)?;
        let mut previous = reply_to.map(status_url);
        let mut last = String::new();
        for (index, part) in parts.iter().enumerate() {
            let mut args = if let Some(target) = &previous {
                vec![
                    "twitter".into(),
                    "reply".into(),
                    target.clone(),
                    part.clone(),
                ]
            } else {
                vec!["twitter".into(), "post".into(), part.clone()]
            };
            // Media on the first part only. `twitter post` takes
            // `--images a,b,c` (max 4, validated above), but
            // `twitter reply` takes a single `--image <path>` (verified in
            // `opencli twitter reply --help` on 1.8.8) — say so plainly
            // instead of silently dropping the extra images.
            if index == 0 && !media.is_empty() {
                if reply_to.is_some() {
                    if media.len() > 1 {
                        return Err(
                            "OpenCLI replies can attach 1 image — remove the extra image(s) and try again. Nothing was posted."
                                .into(),
                        );
                    }
                    args.push("--image".into());
                    args.push(media[0].display().to_string());
                } else {
                    let joined = media
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(",");
                    args.push("--images".into());
                    args.push(joined);
                }
            }
            let args = opencli_json(&args);
            let call_started = Instant::now();
            let output = match self.runner.run_timeout(&self.bin, &args, POST_TIMEOUT) {
                Ok(output) => {
                    write_perf_line(&format!(
                        "write post part={index} cli_ms={} status={}",
                        call_started.elapsed().as_millis(),
                        output.status
                    ));
                    output
                }
                Err(error) => {
                    write_perf_line(&format!(
                        "write post part={index} cli_ms={} runner-error",
                        call_started.elapsed().as_millis()
                    ));
                    return Err(write_timeout_error(error, "posting", POST_TIMEOUT));
                }
            };
            last = check_opencli_output(&output, "opencli post/reply")?;
            if let Some(next) = status_ref_from_output(&last) {
                previous = Some(next);
            }
        }
        let media_note = if media.is_empty() {
            String::new()
        } else {
            format!(" with {} image(s)", media.len())
        };
        let proved = if reply_to.is_none() {
            crate::publish::prove_new_post(&last)
        } else {
            crate::publish::prove_reply(&last)
        };
        match proved {
            Ok(receipt) => {
                write_perf_line("write post proof=confirmed");
                Ok(format!("{receipt}{media_note}"))
            }
            Err(error) => {
                write_perf_line("write post proof=rejected");
                Err(error)
            }
        }
    }

    fn engage(
        &self,
        action: crate::publish::EngageAction,
        target_url: &str,
        text: Option<&str>,
        media: &[PathBuf],
    ) -> Result<String, String> {
        self.engage_opencli(action, target_url, text, media)
    }

    /// `opencli twitter follow|unfollow <handle>` — same success rule
    /// as engage: exit 0 AND no `ok:false` envelope (check_opencli_output),
    /// same 90s engage budget, timeouts say "check before retrying".
    fn set_follow(&self, handle: &str, follow: bool) -> Result<String, String> {
        let handle = handle.trim().trim_start_matches('@');
        if handle.is_empty() {
            return Err("no handle to follow \u{2014} nothing was sent".into());
        }
        let verb = if follow { "follow" } else { "unfollow" };
        let what = if follow { "following" } else { "unfollowing" };
        let args = opencli_json(&["twitter".into(), verb.into(), handle.into()]);
        let call_started = Instant::now();
        let output = match self.runner.run_timeout(&self.bin, &args, ENGAGE_TIMEOUT) {
            Ok(output) => {
                write_perf_line(&format!(
                    "write {verb} cli_ms={} status={}",
                    call_started.elapsed().as_millis(),
                    output.status
                ));
                output
            }
            Err(error) => {
                write_perf_line(&format!(
                    "write {verb} cli_ms={} runner-error",
                    call_started.elapsed().as_millis()
                ));
                return Err(write_timeout_error(error, what, ENGAGE_TIMEOUT));
            }
        };
        let stdout = check_opencli_output(&output, "opencli follow/unfollow")?;
        let kind = if follow {
            crate::publish::OpenCliWrite::Follow
        } else {
            crate::publish::OpenCliWrite::Unfollow
        };
        if let Err(error) = crate::publish::prove_opencli_write(kind, &stdout, Some(handle)) {
            write_perf_line(&format!("write {verb} proof=rejected"));
            return Err(error);
        }
        write_perf_line(&format!("write {verb} proof=confirmed"));
        Ok(if follow {
            format!("Followed @{handle}")
        } else {
            format!("Unfollowed @{handle}")
        })
    }
}

/// Write budgets. A healthy OpenCLI write lands in ~5-20s (measured
/// live), but the budget must cover the CLI's OWN worst case or our
/// timeout fires first and hides its verdict: post.js spends ~13s in
/// pre-submit stages (goto settle + composer insert/verify, measured
/// on the PC with a whitespace probe that cannot submit) and then
/// polls up to 90s for the success toast before reporting
/// "unconfirmed — check before retrying". A 90s cap killed cmd.exe at
/// exactly that point (and, before the process-tree kill, left node
/// running to post later). 150s lets the CLI deliver its own
/// check-before-retry answer; engage gets 90s for the same reason
/// (goto + up-to-10s button poll + verify). Timing out never retries.
const POST_TIMEOUT: Duration = Duration::from_secs(150);
const ENGAGE_TIMEOUT: Duration = Duration::from_secs(90);

fn write_timeout_error(error: String, what: &str, budget: Duration) -> String {
    if error.contains("timed out") {
        format!(
            "{what} timed out after {}s \u{2014} it may or may not have gone through; check X before retrying (nothing was retried automatically)",
            budget.as_secs()
        )
    } else {
        error
    }
}

impl OpenCliPublisher {
    /// `opencli twitter like|unlike|retweet|unretweet <url>` through
    /// the same silent-background shim as posts/replies. A quote does
    /// not call `twitter quote` or `twitter post`. Those commands open
    /// `/compose/post`, which can show a text box with no quote card.
    /// The quote path opens the status page, uses the Quote menu, and
    /// clicks Post only after that card links the original id.
    fn engage_opencli(
        &self,
        action: crate::publish::EngageAction,
        target_url: &str,
        text: Option<&str>,
        media: &[PathBuf],
    ) -> Result<String, String> {
        use crate::publish::EngageAction;
        let args: Vec<String> = vec![
            "twitter".into(),
            action.opencli_verb().into(),
            target_url.into(),
        ];
        if action == EngageAction::Quote {
            let text = text.unwrap_or_default().trim();
            if text.is_empty() {
                return Err("write your quote first — nothing was posted".into());
            }
            if !media.is_empty() {
                return Err(
                    "Remove the image before this quote. Nothing was posted.".into(),
                );
            }
            let target_id = crate::model::post_id_from_input(target_url);
            let report = match run_quote_attachment(
                &self.bin,
                self.runner.as_ref(),
                &target_id,
                text,
                false,
            ) {
                Ok(report) => report,
                Err(error) => {
                    write_perf_line("engage quote proof=rejected");
                    return Err(error);
                }
            };
            let stdout = serde_json::json!([{
                "status": "success",
                "message": "Quote tweet posted successfully.",
                "quoted_status_id_str": target_id,
                "text": text,
                "id": report.new_status_id,
            }])
            .to_string();
            if let Err(error) = crate::publish::prove_quote(&stdout, &target_id) {
                write_perf_line("engage quote proof=rejected");
                return Err(error);
            }
            write_perf_line("engage quote proof=confirmed");
            return Ok(stdout);
        }
        let args = opencli_json(&args);
        let call_started = Instant::now();
        let output = match self.runner.run_timeout(&self.bin, &args, ENGAGE_TIMEOUT) {
            Ok(output) => {
                write_perf_line(&format!(
                    "write engage {} cli_ms={} status={}",
                    action.opencli_verb(),
                    call_started.elapsed().as_millis(),
                    output.status
                ));
                output
            }
            Err(error) => {
                write_perf_line(&format!(
                    "write engage {} cli_ms={} runner-error",
                    action.opencli_verb(),
                    call_started.elapsed().as_millis()
                ));
                return Err(write_timeout_error(error, "that action", ENGAGE_TIMEOUT));
            }
        };
        let stdout = check_opencli_output(&output, "opencli engage")?;
        if let Err(error) = crate::publish::prove_engage(action, &stdout, None) {
            write_perf_line(&format!(
                "engage {} proof=rejected",
                action.opencli_verb()
            ));
            return Err(error);
        }
        write_perf_line(&format!(
            "engage {} proof=confirmed",
            action.opencli_verb()
        ));
        Ok(stdout)
    }
}

struct TwitterCliPublisher {
    bin: String,
    runner: Arc<dyn CommandRunner>,
    post_ok: Mutex<Option<bool>>,
}

impl TwitterCliPublisher {
    fn new(config: &XConfig, runner: Arc<dyn CommandRunner>) -> Self {
        Self {
            bin: config.twitter_bin.clone(),
            runner,
            post_ok: Mutex::new(None),
        }
    }
}

impl Publisher for TwitterCliPublisher {
    fn id(&self) -> ProviderId {
        ProviderId::TwitterCli
    }
    fn available(&self) -> bool {
        posting_supported(
            &self.post_ok,
            self.runner.as_ref(),
            &self.bin,
            &["--help".into()],
        )
    }
    fn session_note(&self) -> Option<String> {
        session_label(ProviderId::TwitterCli)
    }
    fn publish(
        &self,
        parts: &[String],
        reply_to: Option<&str>,
        media: &[PathBuf],
    ) -> Result<String, String> {
        if !media.is_empty() {
            return Err(
                "twitter-cli path cannot upload images here; use OpenCLI or the web intent fallback".into(),
            );
        }
        let mut previous = reply_to.map(crate::model::post_id_from_input);
        for part in parts {
            let mut args = vec!["post".into(), part.clone()];
            if let Some(reply) = &previous
                && !reply.is_empty()
            {
                args.push("--reply-to".into());
                args.push(reply.clone());
            }
            args.push("--json".into());
            let output = self.runner.run(&self.bin, &args)?;
            if output.status != 0 {
                return Err(if output.stderr.is_empty() {
                    format!("twitter exited {}", output.status)
                } else {
                    output.stderr
                });
            }
            let Some(id) = tweet_id_from_output(&output.stdout) else {
                return Err(
                    "Post was not confirmed. twitter-cli returned no status id. Not marked Posted. Nothing was retried."
                        .into(),
                );
            };
            if id.is_empty() || !id.chars().all(|ch| ch.is_ascii_digit()) {
                return Err(
                    "Post was not confirmed. twitter-cli returned no status id. Not marked Posted. Nothing was retried."
                        .into(),
                );
            }
            previous = Some(id);
        }
        Ok(format!(
            "twitter-cli posted {}",
            previous.unwrap_or_else(|| "posted".into())
        ))
    }
}

fn posting_supported(
    cache: &Mutex<Option<bool>>,
    runner: &dyn CommandRunner,
    bin: &str,
    help_args: &[String],
) -> bool {
    if !runner.exists(bin) {
        return false;
    }
    {
        let cached = cache.lock().expect("post support");
        if let Some(ok) = *cached {
            return ok;
        }
    }
    // Process-wide cache: submit paths rebuild their publisher on
    // every click, so the per-instance cache alone re-ran a full
    // `opencli twitter --help` cold start (~0.7s measured) before
    // EVERY post/like/retweet. Probe once per process, not per click.
    static GLOBAL: std::sync::OnceLock<Mutex<std::collections::HashMap<String, bool>>> =
        std::sync::OnceLock::new();
    let global = GLOBAL.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let key = format!("{bin}|{}", help_args.join(" "));
    if let Some(ok) = global
        .lock()
        .expect("post support global")
        .get(&key)
        .copied()
    {
        *cache.lock().expect("post support") = Some(ok);
        return ok;
    }
    let ok = match runner.run(bin, help_args) {
        Ok(output) => help_lists_command(&format!("{}\n{}", output.stdout, output.stderr), "post"),
        Err(_) => false,
    };
    global.lock().expect("post support global").insert(key, ok);
    *cache.lock().expect("post support") = Some(ok);
    ok
}

pub fn help_lists_command(text: &str, command: &str) -> bool {
    for line in text.lines() {
        let lower = line.trim().to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix(command)
            && (rest.is_empty() || rest.starts_with([' ', '\t', '<', '[']))
        {
            return true;
        }
        if lower.split(',').any(|part| part.trim() == command) {
            return true;
        }
    }
    false
}

/// Standard base64 (with padding) — just enough to hand a JS payload to
/// `opencli browser … eval eval(atob('…'))` without quoting/newline issues.
fn b64_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// The in-page timeline walker: fetches GraphQL pages of the home feed
/// starting at `cursor` (null = top) until `count` tweets are collected
/// or the bottom cursor dries up, then returns
/// `{posts, nextCursor}` (or `{error, posts, nextCursor}` on a mid-walk
/// failure so partial older progress is kept). Mirrors OpenCLI's own
/// timeline parsing; runs entirely inside the user's logged-in x.com
/// page, so the ct0 cookie never leaves the page.
fn timeline_eval_js(feed: TimelineFeed, cursor: Option<&str>, count: usize) -> String {
    let feed_js = serde_json::to_string(feed.opencli_value()).expect("feed js");
    let cursor_js = match cursor {
        Some(c) => serde_json::to_string(c).expect("cursor js"),
        None => "null".to_string(),
    };
    let provider = HOME_PROVIDER_PAGE;
    let hops = HOME_MAX_HOPS;
    format!(
        r##"(async () => {{
  const FEED = {feed_js};
  const WANT = {count};
  const PROVIDER = {provider};
  const MAX_HOPS = {hops};
  let cursor = {cursor_js};
  // The browser tab may still be loading x.com when this eval lands
  // (cold session): poll for the csrf cookie instead of failing fast.
  let ct0 = null;
  for (let w = 0; w < 24; w++) {{
    ct0 = (document.cookie.match(/(?:^|; )ct0=([^;]+)/) || [])[1];
    if (ct0) break;
    await new Promise(r => setTimeout(r, 500));
  }}
  if (!ct0) return JSON.stringify({{error: "not logged into x.com (no ct0 cookie)"}});
  const FEATURES = {{"rweb_video_screen_enabled":false,"profile_label_improvements_pcf_label_in_post_enabled":true,"rweb_tipjar_consumption_enabled":true,"verified_phone_label_enabled":false,"creator_subscriptions_tweet_preview_api_enabled":true,"responsive_web_graphql_timeline_navigation_enabled":true,"responsive_web_grok_analyze_post_followups_enabled":true,"responsive_web_grok_share_attachment_enabled":true,"articles_preview_enabled":true,"responsive_web_edit_tweet_api_enabled":true,"graphql_is_translatable_rweb_tweet_is_translatable_enabled":true,"view_counts_everywhere_api_enabled":true,"longform_notetweets_consumption_enabled":true,"responsive_web_twitter_article_tweet_consumption_enabled":true,"longform_notetweets_rich_text_read_enabled":true,"longform_notetweets_inline_media_enabled":true,"responsive_web_grok_image_annotation_enabled":true,"responsive_web_enhance_cards_enabled":false}};
  const EP = FEED === "following"
    ? {{name: "HomeLatestTimeline", qid: "BKB7oi212Fi7kQtCBGE4zA", method: "POST"}}
    : {{name: "HomeTimeline", qid: "c-CzHF1LboFilMpsx4ZCrQ", method: "GET"}};
  const headers = {{Authorization: "Bearer AAAAAAAAAAAAAAAAAAAAANRILgAAAAAAnNwIzUejRCOuH5E6I8xnZz4puTs%3D1Zv7ttfk8LF81IUq16cHjhLTvJu4FA33AGWWjCpTnA", "X-Csrf-Token": decodeURIComponent(ct0), "X-Twitter-Auth-Type": "OAuth2Session", "X-Twitter-Active-User": "yes"}};
  async function resolveQid(op) {{
    try {{
      const srcs = [...document.scripts].map(s => s.src).filter(Boolean);
      for (const src of srcs) {{
        const txt = await (await fetch(src)).text();
        let i = txt.indexOf(op);
        let hops = 0;
        while (i >= 0 && hops < 6) {{
          const seg = txt.slice(Math.max(0, i - 1200), i + 1200);
          const m = seg.match(/queryId:"([A-Za-z0-9_-]{{10,}})"/);
          if (m) return m[1];
          i = txt.indexOf(op, i + 1); hops++;
        }}
      }}
    }} catch (e) {{}}
    return null;
  }}
  function mediaOf(l) {{
    const media = (l.extended_entities && l.extended_entities.media) || (l.entities && l.entities.media) || [];
    const urls = [], posters = [], durations = [];
    for (const m of media) {{
      if (!m) continue;
      if (m.type === "video" || m.type === "animated_gif") {{
        let best = null, bestScore = 0;
        for (const v of (m.video_info && m.video_info.variants) || []) {{
          if (!v || !v.url) continue;
          const t = String(v.content_type || "");
          let rank = 0;
          if (t === "video/mp4") rank = 3;
          else if (t === "application/x-mpegURL" || t === "application/vnd.apple.mpegurl") rank = 2;
          else if (t.indexOf("audio/") === 0) rank = 1;
          if (!rank) continue;
          const score = rank * 100000000 + (v.bitrate || 0);
          if (!best || score > bestScore) {{ best = v; bestScore = score; }}
        }}
        urls.push(best ? best.url : m.media_url_https);
        posters.push([m.media_url_https, m.media_url, m.preview_image_url].find(function(c){{ return typeof c==="string" && c && !/\.(mp4|m4v|webm|mkv|mov|m3u8|ts|m4s)(\?|#|$)/i.test(c.split("?")[0]); }}) || "");
        durations.push((m.video_info && m.video_info.duration_millis) || 0);
      }} else {{
        urls.push(m.media_url_https); posters.push(m.media_url_https); durations.push(0);
      }}
    }}
    return {{has_media: urls.length > 0, media_urls: urls, media_posters: posters, media_durations: durations}};
  }}
  function unwrapTweet(node) {{
    let cur = node;
    for (let i = 0; i < 4 && cur; i++) {{
      if (cur.result) cur = cur.result;
      else if (cur.tweet && !cur.legacy && !cur.rest_id) cur = cur.tweet;
      else break;
    }}
    return cur;
  }}
  function tombstone(node) {{
    const t = (node && (node.__typename || node.typename)) || "";
    return t.indexOf("Unavailable") >= 0 || t.indexOf("Tombstone") >= 0;
  }}
  function extract(result, seen) {{
    if (!result) return null;
    const tw = result.tweet || result;
    const l = tw.legacy || {{}};
    if (!tw.rest_id || seen.has(tw.rest_id)) return null;
    seen.add(tw.rest_id);
    const u = tw.core && tw.core.user_results && tw.core.user_results.result;
    const screenName = (u && u.legacy && u.legacy.screen_name) || (u && u.core && u.core.screen_name) || "unknown";
    const noteText = tw.note_tweet && tw.note_tweet.note_tweet_results && tw.note_tweet.note_tweet_results.result && tw.note_tweet.note_tweet_results.result.text;
    const rtNode = unwrapTweet(tw.retweeted_status_result) || unwrapTweet(l.retweeted_status_result);
    const qtNode = unwrapTweet(tw.quoted_status_result) || unwrapTweet(l.quoted_status_result);
    const rtId = String((l && (l.retweeted_status_id_str || l.retweeted_status_id)) || "");
    return {{id: tw.rest_id, author: screenName, author_verified: Boolean((u && u.is_blue_verified) || (u && u.legacy && u.legacy.verified)), text: noteText || l.full_text || "", likes: l.favorite_count || 0, retweets: l.retweet_count || 0, replies: l.reply_count || 0, views: tw.views && tw.views.count ? parseInt(tw.views.count, 10) : 0, created_at: l.created_at || "", url: "https://x.com/" + screenName + "/status/" + tw.rest_id, retweeted: (rtNode && !tombstone(rtNode) && (rtNode.legacy || rtNode.tweet || rtNode.rest_id)) ? extract(rtNode, new Set()) : null, quoted: (qtNode && !tombstone(qtNode) && (qtNode.legacy || qtNode.tweet || qtNode.rest_id)) ? extract(qtNode, new Set()) : null, referenced_status_id: rtId || null, unavailable_original_url: (tombstone(rtNode) && rtId) ? ("https://x.com/i/status/" + rtId) : null, ...mediaOf(l)}};
  }}
  function parsePage(data, seen) {{
    const tweets = [];
    let next = null;
    const instructions = (data && data.data && data.data.home && data.data.home.home_timeline_urt && data.data.home.home_timeline_urt.instructions) || [];
    for (const inst of instructions) {{
      for (const entry of inst.entries || []) {{
        const c = entry.content || {{}};
        const cursorValue = c.value || (c.itemContent && c.itemContent.value) || "";
        const cursorType = c.cursorType || (c.itemContent && c.itemContent.cursorType) || "";
        if (cursorType === "Bottom" || String(entry.entryId || "").indexOf("cursor-bottom") === 0) {{ if (cursorValue) next = cursorValue; continue; }}
        const tr = c.itemContent && c.itemContent.tweet_results && c.itemContent.tweet_results.result;
        if (tr) {{ if (!c.itemContent.promotedMetadata) {{ const tw = extract(tr, seen); if (tw) tweets.push(tw); }} continue; }}
        for (const item of c.items || []) {{
          const nested = item.item && item.item.itemContent && item.item.itemContent.tweet_results && item.item.itemContent.tweet_results.result;
          if (nested && !(item.item.itemContent && item.item.itemContent.promotedMetadata)) {{ const tw = extract(nested, seen); if (tw) tweets.push(tw); }}
        }}
      }}
    }}
    return {{tweets, next}};
  }}
  const seen = new Set();
  const out = [];
  let resolved = false;
  for (let page = 0; page < MAX_HOPS && out.length < WANT; page++) {{
    const vars = {{count: PROVIDER, includePromotedContent: false, latestControlAvailable: true, requestContext: "launch"}};
    if (FEED === "for-you") vars.withCommunity = true; else vars.seenTweetIds = [];
    if (cursor) vars.cursor = cursor;
    let url, opts;
    if (EP.method === "GET") {{
      url = "/i/api/graphql/" + EP.qid + "/" + EP.name + "?variables=" + encodeURIComponent(JSON.stringify(vars)) + "&features=" + encodeURIComponent(JSON.stringify(FEATURES));
      opts = {{method: "GET", headers, credentials: "include"}};
    }} else {{
      url = "/i/api/graphql/" + EP.qid + "/" + EP.name;
      opts = {{method: "POST", headers: {{...headers, "Content-Type": "application/json"}}, credentials: "include", body: JSON.stringify({{variables: vars, features: FEATURES, queryId: EP.qid}})}};
    }}
    const r = await fetch(url, opts);
    if (!r.ok) {{
      if (r.status === 404 && !resolved) {{
        const q = await resolveQid(EP.name);
        if (q && q !== EP.qid) {{ EP.qid = q; resolved = true; page--; continue; }}
      }}
      return JSON.stringify({{error: "http-" + r.status, posts: out, nextCursor: cursor}});
    }}
    const data = await r.json();
    const got = parsePage(data, seen);
    out.push(...got.tweets);
    if (!got.next || got.next === cursor) {{ cursor = null; break; }}
    cursor = got.next;
  }}
  return JSON.stringify({{posts: out.slice(0, WANT), nextCursor: cursor}});
}})()"##
    )
}

/// Parse one timeline-eval payload. A mid-walk `error` with posts keeps
/// the older progress instead of throwing the whole batch away; an
/// error with nothing fetched is a real failure (rate limit, logged
/// out, dead session) and must surface as one.
fn parse_timeline_eval(stdout: &str) -> Result<TimelinePage, String> {
    if let Some(page) = crate::parse_borrowed::parse_timeline_eval_borrowed(stdout.trim()) {
        return page;
    }
    let value: Value = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("timeline session returned unreadable output: {e}"))?;
    let next_cursor = value
        .get("nextCursor")
        .and_then(Value::as_str)
        .map(str::to_string);
    // Already-parsed rows are converted in place; the previous code
    // re-serialized them to a String and re-parsed the whole array.
    let posts = match value.get("posts") {
        Some(rows) if rows.is_array() => posts_from_value(rows),
        _ => Vec::new(),
    };
    if let Some(error) = value.get("error").and_then(Value::as_str)
        && posts.is_empty()
    {
        return Err(if error == "http-429" {
            "X rate-limited the session (HTTP 429) — wait a few minutes and try Load more again"
                .into()
        } else {
            format!("timeline session: {error}")
        });
    }
    Ok(TimelinePage { posts, next_cursor })
}

/// In-page pinned-post finder. OpenCLI's `twitter tweets` (the Latest
/// source) excludes the pinned post, and hand-rolled UserByScreenName /
/// UserTweets GraphQL calls are rejected by the current web client
/// (422 GRAPHQL_VALIDATION_FAILED), so this uses what works: the
/// profile page renders the pinned post first with a "Pinned" marker —
/// take its status id from the DOM and fetch the full row via
/// TweetResultByRestId. Returns `{posts: [], pinned}` in the same
/// normalized row shape as the timeline eval; Latest comes from the
/// CLI. No marker means the account has no pinned post.
fn profile_eval_js(handle: &str, want: usize) -> String {
    let handle_js = serde_json::to_string(handle).expect("handle js");
    PROFILE_EVAL_JS
        .replace("__HANDLE_JSON__", &handle_js)
        .replace("__WANT__", &want.to_string())
}

const PROFILE_EVAL_JS: &str = r##"(async () => {
  const HANDLE = __HANDLE_JSON__;
  const WANT = __WANT__;
  let ct0 = null;
  for (let w = 0; w < 24; w++) {
    ct0 = (document.cookie.match(/(?:^|; )ct0=([^;]+)/) || [])[1];
    if (ct0) break;
    await new Promise(r => setTimeout(r, 500));
  }
  if (!ct0) return JSON.stringify({error: "not logged into x.com (no ct0 cookie)", posts: [], pinned: null});
  const FEATURES = {"rweb_video_screen_enabled":false,"profile_label_improvements_pcf_label_in_post_enabled":true,"rweb_tipjar_consumption_enabled":true,"verified_phone_label_enabled":false,"creator_subscriptions_tweet_preview_api_enabled":true,"responsive_web_graphql_timeline_navigation_enabled":true,"responsive_web_grok_analyze_post_followups_enabled":true,"responsive_web_grok_share_attachment_enabled":true,"articles_preview_enabled":true,"responsive_web_edit_tweet_api_enabled":true,"graphql_is_translatable_rweb_tweet_is_translatable_enabled":true,"view_counts_everywhere_api_enabled":true,"longform_notetweets_consumption_enabled":true,"responsive_web_twitter_article_tweet_consumption_enabled":true,"longform_notetweets_rich_text_read_enabled":true,"longform_notetweets_inline_media_enabled":true,"responsive_web_grok_image_annotation_enabled":true,"responsive_web_enhance_cards_enabled":false};
  const headers = {Authorization: "Bearer AAAAAAAAAAAAAAAAAAAAANRILgAAAAAAnNwIzUejRCOuH5E6I8xnZz4puTs%3D1Zv7ttfk8LF81IUq16cHjhLTvJu4FA33AGWWjCpTnA", "X-Csrf-Token": decodeURIComponent(ct0), "X-Twitter-Auth-Type": "OAuth2Session", "X-Twitter-Active-User": "yes"};
  const gqlDiag = {};
  async function resolveQid(op) {
    try {
      const urlSet = new Set();
      [...document.scripts].forEach(s => { if (s.src) urlSet.add(s.src); });
      try { performance.getEntriesByType("resource").forEach(r => { if (r.name && r.name.includes(".js")) urlSet.add(r.name); }); } catch (e) {}
      const srcs = [...urlSet];
      for (const src of srcs) {
        const txt = await (await fetch(src)).text();
        let i = txt.indexOf(op);
        let hops = 0;
        while (i >= 0 && hops < 6) {
          const seg = txt.slice(Math.max(0, i - 1200), i + 1200);
          const m = seg.match(/queryId:"([A-Za-z0-9_-]{10,})"/);
          if (m) return m[1];
          i = txt.indexOf(op, i + 1); hops++;
        }
      }
    } catch (e) {}
    return null;
  }
  async function gql(op, vars) {
    let qid = await resolveQid(op);
    if (!qid) { gqlDiag[op] = "noqid"; return null; }
    for (let attempt = 0; attempt < 2; attempt++) {
      const url = "/i/api/graphql/" + qid + "/" + op + "?variables=" + encodeURIComponent(JSON.stringify(vars)) + "&features=" + encodeURIComponent(JSON.stringify(FEATURES));
      const r = await fetch(url, {method: "GET", headers, credentials: "include"});
      if (r.status === 404 && attempt === 0) {
        const q2 = await resolveQid(op);
        if (q2 && q2 !== qid) { qid = q2; continue; }
      }
      if (!r.ok) { try { gqlDiag[op] = r.status + " " + (await r.text()).slice(0, 200); } catch (e) { gqlDiag[op] = r.status; } return null; }
      gqlDiag[op] = r.status;
      return await r.json();
    }
    return null;
  }
  function mediaOf(l) {
    const media = (l.extended_entities && l.extended_entities.media) || (l.entities && l.entities.media) || [];
    const urls = [], posters = [], durations = [];
    for (const m of media) {
      if (!m) continue;
      if (m.type === "video" || m.type === "animated_gif") {
        let best = null, bestScore = 0;
        for (const v of (m.video_info && m.video_info.variants) || []) {
          if (!v || !v.url) continue;
          const t = String(v.content_type || "");
          let rank = 0;
          if (t === "video/mp4") rank = 3;
          else if (t === "application/x-mpegURL" || t === "application/vnd.apple.mpegurl") rank = 2;
          else if (t.indexOf("audio/") === 0) rank = 1;
          if (!rank) continue;
          const score = rank * 100000000 + (v.bitrate || 0);
          if (!best || score > bestScore) { best = v; bestScore = score; }
        }
        urls.push(best ? best.url : m.media_url_https);
        posters.push([m.media_url_https, m.media_url, m.preview_image_url].find(function(c){ return typeof c==="string" && c && !/\.(mp4|m4v|webm|mkv|mov|m3u8|ts|m4s)(\?|#|$)/i.test(c.split("?")[0]); }) || "");
        durations.push((m.video_info && m.video_info.duration_millis) || 0);
      } else {
        urls.push(m.media_url_https); posters.push(m.media_url_https); durations.push(0);
      }
    }
    return {has_media: urls.length > 0, media_urls: urls, media_posters: posters, media_durations: durations};
  }
  function unwrapTweet(node) {
    let cur = node;
    for (let i = 0; i < 4 && cur; i++) {
      if (cur.result) cur = cur.result;
      else if (cur.tweet && !cur.legacy && !cur.rest_id) cur = cur.tweet;
      else break;
    }
    return cur;
  }
  function tombstone(node) {
    const t = (node && (node.__typename || node.typename)) || "";
    return t.indexOf("Unavailable") >= 0 || t.indexOf("Tombstone") >= 0;
  }
  function extract(result, seen) {
    if (!result) return null;
    const tw = result.tweet || result;
    const l = tw.legacy || {};
    if (!tw.rest_id || seen.has(tw.rest_id)) return null;
    seen.add(tw.rest_id);
    const u = tw.core && tw.core.user_results && tw.core.user_results.result;
    const screenName = (u && u.legacy && u.legacy.screen_name) || (u && u.core && u.core.screen_name) || HANDLE;
    const noteText = tw.note_tweet && tw.note_tweet.note_tweet_results && tw.note_tweet.note_tweet_results.result && tw.note_tweet.note_tweet_results.result.text;
    const rtNode = unwrapTweet(tw.retweeted_status_result) || unwrapTweet(l.retweeted_status_result);
    const qtNode = unwrapTweet(tw.quoted_status_result) || unwrapTweet(l.quoted_status_result);
    const rtId = String((l && (l.retweeted_status_id_str || l.retweeted_status_id)) || "");
    return {id: tw.rest_id, author: screenName, author_verified: Boolean((u && u.is_blue_verified) || (u && u.legacy && u.legacy.verified)), text: noteText || l.full_text || "", likes: l.favorite_count || 0, retweets: l.retweet_count || 0, replies: l.reply_count || 0, views: tw.views && tw.views.count ? parseInt(tw.views.count, 10) : 0, created_at: l.created_at || "", url: "https://x.com/" + screenName + "/status/" + tw.rest_id, retweeted: (rtNode && !tombstone(rtNode) && (rtNode.legacy || rtNode.tweet || rtNode.rest_id)) ? extract(rtNode, new Set()) : null, quoted: (qtNode && !tombstone(qtNode) && (qtNode.legacy || qtNode.tweet || qtNode.rest_id)) ? extract(qtNode, new Set()) : null, referenced_status_id: rtId || null, unavailable_original_url: (tombstone(rtNode) && rtId) ? ("https://x.com/i/status/" + rtId) : null, ...mediaOf(l)};
  }
  // Pinned extraction, the mechanism that actually works against the
  // live client: the profile page renders the pinned post first with a
  // "Pinned" marker, so read its status id from the DOM, then fetch the
  // full normalized row with TweetResultByRestId. (Hand-rolled
  // profile GraphQL calls get 422s from the current client, and
  // `twitter tweets` never includes the pin.)
  let pinned = null, domPinId = null;
  for (let w = 0; w < 16 && !domPinId; w++) {
    try {
      const arts = [...document.querySelectorAll("article")];
      for (const a of arts) {
        const tx = (a.innerText || "").slice(0, 80);
        if (tx.includes("Pinned")) {
          const lk = a.querySelector("a[href*='/status/']");
          if (lk) { const mm = lk.getAttribute("href").match(/\/status\/(\d+)/); if (mm) { domPinId = mm[1]; break; } }
        }
      }
      if (!arts.length && !domPinId) await new Promise(r => setTimeout(r, 500));
    } catch (e) {}
    if (!domPinId) await new Promise(r => setTimeout(r, 250));
  }
  if (domPinId) {
    const one = await gql("TweetResultByRestId", {tweetId: domPinId, withCommunity: false, includePromotedContent: false, withVoice: false});
    const tr = one && one.data && one.data.tweetResult && one.data.tweetResult.result;
    if (tr) pinned = extract(tr, new Set());
  }
  const diag = {domPinId, gql: gqlDiag};
  return JSON.stringify({posts: [], pinned, diag});
})()"##;

/// Parse one profile-eval payload into (latest, pinned). An `error`
/// with no posts at all is a real failure; partial data is kept.
fn parse_profile_eval(stdout: &str) -> Result<(Vec<Post>, Vec<Post>), String> {
    if let Some(parsed) = crate::parse_borrowed::parse_profile_eval_borrowed(stdout.trim()) {
        return parsed;
    }
    let value: Value = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("profile session returned unreadable output: {e}"))?;
    if let Some(diag) = value.get("diag") {
        log_raw_provider_error(
            "profile diag",
            &serde_json::to_string(diag).unwrap_or_else(|_| "?".to_string()),
        );
    }
    let posts = match value.get("posts") {
        Some(rows) if rows.is_array() => posts_from_value(rows),
        _ => Vec::new(),
    };
    let pinned = match value.get("pinned") {
        Some(row) if row.is_object() => post_from_value(row, &[]).into_iter().collect(),
        _ => Vec::new(),
    };
    if let Some(error) = value.get("error").and_then(Value::as_str)
        && posts.is_empty()
        && pinned.is_empty()
    {
        return Err(error.to_string());
    }
    Ok((posts, pinned))
}

/// Single-tweet fetch for bare-repost resolution: TweetResultByRestId
/// in the silent session (same GraphQL path the pinned-post fetch
/// proved against the live client). Returns `{post: <normalized row>}`
/// where the row's `retweeted` carries the original when the tweet is
/// a repost, or `{error, post: null}`.
fn tweet_by_id_eval_js(id: &str) -> String {
    let id_js = serde_json::to_string(id).expect("tweet id js");
    TWEET_BY_ID_EVAL_JS.replace("__TWEET_ID_JSON__", &id_js)
}

const TWEET_BY_ID_EVAL_JS: &str = r##"(async () => {
  const TWEET_ID = __TWEET_ID_JSON__;
  let ct0 = null;
  for (let w = 0; w < 24; w++) {
    ct0 = (document.cookie.match(/(?:^|; )ct0=([^;]+)/) || [])[1];
    if (ct0) break;
    await new Promise(r => setTimeout(r, 500));
  }
  if (!ct0) return JSON.stringify({error: "not logged into x.com (no ct0 cookie)", post: null});
  const FEATURES = {{"rweb_video_screen_enabled":false,"profile_label_improvements_pcf_label_in_post_enabled":true,"rweb_tipjar_consumption_enabled":true,"verified_phone_label_enabled":false,"creator_subscriptions_tweet_preview_api_enabled":true,"responsive_web_graphql_timeline_navigation_enabled":true,"responsive_web_grok_analyze_post_followups_enabled":true,"responsive_web_grok_share_attachment_enabled":true,"articles_preview_enabled":true,"responsive_web_edit_tweet_api_enabled":true,"graphql_is_translatable_rweb_tweet_is_translatable_enabled":true,"view_counts_everywhere_api_enabled":true,"longform_notetweets_consumption_enabled":true,"responsive_web_twitter_article_tweet_consumption_enabled":true,"longform_notetweets_rich_text_read_enabled":true,"longform_notetweets_inline_media_enabled":true,"responsive_web_grok_image_annotation_enabled":true,"responsive_web_enhance_cards_enabled":false}};
  const headers = {Authorization: "Bearer AAAAAAAAAAAAAAAAAAAAANRILgAAAAAAnNwIzUejRCOuH5E6I8xnZz4puTs%3D1Zv7ttfk8LF81IUq16cHjhLTvJu4FA33AGWWjCpTnA", "X-Csrf-Token": decodeURIComponent(ct0), "X-Twitter-Auth-Type": "OAuth2Session", "X-Twitter-Active-User": "yes"};
  async function resolveQid(op) {
    try {
      const urlSet = new Set();
      [...document.scripts].forEach(sc => { if (sc.src) urlSet.add(sc.src); });
      try { performance.getEntriesByType("resource").forEach(r => { if (r.name && r.name.includes(".js")) urlSet.add(r.name); }); } catch (e) {}
      for (const src of [...urlSet]) {
        const txt = await (await fetch(src)).text();
        let i = txt.indexOf(op);
        let hops = 0;
        while (i >= 0 && hops < 6) {
          const seg = txt.slice(Math.max(0, i - 1200), i + 1200);
          const mm = seg.match(/queryId:"([A-Za-z0-9_-]{10,})"/);
          if (mm) return mm[1];
          i = txt.indexOf(op, i + 1); hops++;
        }
      }
    } catch (e) {}
    return null;
  }
  async function gql(op, vars) {
    const qid = await resolveQid(op);
    if (!qid) return null;
    const url = "/i/api/graphql/" + qid + "/" + op + "?variables=" + encodeURIComponent(JSON.stringify(vars)) + "&features=" + encodeURIComponent(JSON.stringify(FEATURES));
    const r = await fetch(url, {method: "GET", headers, credentials: "include"});
    if (!r.ok) return null;
    return await r.json();
  }
  function mediaOf(l) {
    const media = (l.extended_entities && l.extended_entities.media) || (l.entities && l.entities.media) || [];
    const urls = [], posters = [], durations = [];
    for (const m of media) {
      if (!m) continue;
      if (m.type === "video" || m.type === "animated_gif") {
        let best = null, bestScore = 0;
        for (const v of (m.video_info && m.video_info.variants) || []) {
          if (!v || !v.url) continue;
          const t = String(v.content_type || "");
          let rank = 0;
          if (t === "video/mp4") rank = 3;
          else if (t === "application/x-mpegURL" || t === "application/vnd.apple.mpegurl") rank = 2;
          else if (t.indexOf("audio/") === 0) rank = 1;
          if (!rank) continue;
          const score = rank * 100000000 + (v.bitrate || 0);
          if (!best || score > bestScore) { best = v; bestScore = score; }
        }
        urls.push(best ? best.url : m.media_url_https);
        posters.push([m.media_url_https, m.media_url, m.preview_image_url].find(function(c){ return typeof c==="string" && c && !/\.(mp4|m4v|webm|mkv|mov|m3u8|ts|m4s)(\?|#|$)/i.test(c.split("?")[0]); }) || "");
        durations.push((m.video_info && m.video_info.duration_millis) || 0);
      } else {
        urls.push(m.media_url_https); posters.push(m.media_url_https); durations.push(0);
      }
    }
    return {has_media: urls.length > 0, media_urls: urls, media_posters: posters, media_durations: durations};
  }
  function unwrapTweet(node) {
    let cur = node;
    for (let i = 0; i < 4 && cur; i++) {
      if (cur.result) cur = cur.result;
      else if (cur.tweet && !cur.legacy && !cur.rest_id) cur = cur.tweet;
      else break;
    }
    return cur;
  }
  function tombstone(node) {
    const t = (node && (node.__typename || node.typename)) || "";
    return t.indexOf("Unavailable") >= 0 || t.indexOf("Tombstone") >= 0;
  }
  function extract(result, seen) {
    if (!result) return null;
    const tw = result.tweet || result;
    const l = tw.legacy || {};
    if (!tw.rest_id || seen.has(tw.rest_id)) return null;
    seen.add(tw.rest_id);
    const u = tw.core && tw.core.user_results && tw.core.user_results.result;
    const screenName = (u && u.legacy && u.legacy.screen_name) || (u && u.core && u.core.screen_name) || "unknown";
    const noteText = tw.note_tweet && tw.note_tweet.note_tweet_results && tw.note_tweet.note_tweet_results.result && tw.note_tweet.note_tweet_results.result.text;
    const rtNode = unwrapTweet(tw.retweeted_status_result) || unwrapTweet(l.retweeted_status_result);
    const qtNode = unwrapTweet(tw.quoted_status_result) || unwrapTweet(l.quoted_status_result);
    const rtId = String((l && (l.retweeted_status_id_str || l.retweeted_status_id)) || "");
    return {id: tw.rest_id, author: screenName, author_verified: Boolean((u && u.is_blue_verified) || (u && u.legacy && u.legacy.verified)), text: noteText || l.full_text || "", likes: l.favorite_count || 0, retweets: l.retweet_count || 0, replies: l.reply_count || 0, views: tw.views && tw.views.count ? parseInt(tw.views.count, 10) : 0, created_at: l.created_at || "", url: "https://x.com/" + screenName + "/status/" + tw.rest_id, retweeted: (rtNode && !tombstone(rtNode) && (rtNode.legacy || rtNode.tweet || rtNode.rest_id)) ? extract(rtNode, new Set()) : null, quoted: (qtNode && !tombstone(qtNode) && (qtNode.legacy || qtNode.tweet || qtNode.rest_id)) ? extract(qtNode, new Set()) : null, referenced_status_id: rtId || null, unavailable_original_url: (tombstone(rtNode) && rtId) ? ("https://x.com/i/status/" + rtId) : null, ...mediaOf(l)};
  }
  const one = await gql("TweetResultByRestId", {tweetId: TWEET_ID, withCommunity: false, includePromotedContent: false, withVoice: false});
  const tr = one && one.data && one.data.tweetResult && one.data.tweetResult.result;
  if (!tr) return JSON.stringify({error: "tweet not found", post: null});
  const post = extract(tr, new Set());
  if (!post) return JSON.stringify({error: "tweet payload unreadable", post: null});
  return JSON.stringify({post});
})()"##;

/// Parse one `tweet_by_id_eval_js` payload into the normalized post.
fn parse_tweet_by_id_eval(stdout: &str) -> Result<Post, String> {
    let value: Value = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("tweet session returned unreadable output: {e}"))?;
    if let Some(row) = value.get("post").filter(|row| row.is_object()) {
        let posts = parse_posts(&serde_json::json!([row]).to_string())?;
        return posts
            .into_iter()
            .next()
            .ok_or_else(|| "tweet payload carried no post".to_string());
    }
    Err(value
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("tweet not found")
        .to_string())
}

fn opencli_read(args: &[String]) -> Vec<String> {
    opencli_json(args)
}

/// Public wrapper so `x connect` builds the exact same OpenCLI call as reads.
pub fn opencli_read_pub(args: &[String]) -> Vec<String> {
    opencli_read(args)
}

fn opencli_json(args: &[String]) -> Vec<String> {
    let mut args = args.to_vec();
    args.push("-f".into());
    args.push("json".into());
    // Silent background, verified against OpenCLI 1.8.8 `--help` on Windows:
    // `--window background` never focuses Chrome, and an explicit ephemeral
    // site session with `--keep-tab false` releases the tab lease when the
    // command finishes (the extension parks its automation tab on
    // about:blank instead of leaving an x.com page open). Never use
    // `twitter whoami` for this: it is a *persistent* session in OpenCLI's
    // manifest, which forces keep-tab on and leaves a live X tab behind.
    args.push("--window".into());
    args.push("background".into());
    args.push("--site-session".into());
    args.push("ephemeral".into());
    args.push("--keep-tab".into());
    args.push("false".into());
    args
}

fn status_url(id_or_url: &str) -> String {
    let trimmed = id_or_url.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!(
            "https://x.com/i/status/{}",
            crate::model::post_id_from_input(trimmed)
        )
    }
}

fn status_ref_from_output(stdout: &str) -> Option<String> {
    let value: Value = serde_json::from_str(stdout).ok()?;
    let node = value.get("data").unwrap_or(&value);
    let node = node
        .as_array()
        .and_then(|rows| rows.first())
        .unwrap_or(node);
    if let Some(url) = first_str(node, &["url"])
        && url.starts_with("http")
    {
        return Some(url.to_string());
    }
    first_str(node, &["id", "id_str"]).map(status_url)
}

fn tweet_id_from_output(stdout: &str) -> Option<String> {
    let value: Value = serde_json::from_str(stdout).ok()?;
    let node = value.get("data").unwrap_or(&value);
    let node = node
        .as_array()
        .and_then(|rows| rows.first())
        .unwrap_or(node);
    first_str(node, &["id", "id_str"]).map(|id| id.to_string())
}

struct OfficialProvider {
    config: XConfig,
    http: Arc<dyn Http>,
    token: Mutex<Option<String>>,
}

struct OfficialShare {
    config: XConfig,
    http: Arc<dyn Http>,
    token: Arc<Mutex<Option<String>>>,
}

impl OfficialProvider {
    fn new(config: &XConfig, http: Arc<dyn Http>) -> Self {
        Self {
            config: config.clone(),
            http,
            token: Mutex::new(None),
        }
    }

    fn share(&self) -> OfficialShare {
        OfficialShare {
            config: self.config.clone(),
            http: self.http.clone(),
            token: Arc::new(Mutex::new(self.token.lock().expect("token").clone())),
        }
    }
}

impl OfficialShare {
    fn token(&self) -> Result<String, String> {
        if let Some(token) = self.token.lock().expect("token").clone() {
            return Ok(token);
        }
        let token = crate::auth::load_access_token(&self.config)?;
        *self.token.lock().expect("token") = Some(token.clone());
        Ok(token)
    }

    fn get(&self, url: &str) -> Result<String, String> {
        let token = self.token()?;
        self.http.request(HttpRequest {
            method: "GET",
            url: url.to_string(),
            body: None,
            bearer: Some(token),
            content_type: None,
            headers: Vec::new(),
        })
    }
}

impl XProvider for OfficialProvider {
    fn id(&self) -> ProviderId {
        ProviderId::OfficialApi
    }
    fn available(&self) -> bool {
        self.config.official_enabled
    }
    fn lookup(&self, handle: &str) -> Result<Served<Profile>, String> {
        let share = self.share();
        let body = share.get(&format!(
            "https://api.x.com/2/users/by/username/{handle}?user.fields=description,public_metrics,profile_image_url"
        ))?;
        let profile = parse_profile(&body, handle)?;
        Ok(official_served(
            profile,
            1,
            self.config.user_read_usd,
            "1 user",
        ))
    }
    fn thread(&self, id: &str) -> Result<Served<Thread>, String> {
        let share = self.share();
        let body = share.get(&format!(
            "https://api.x.com/2/tweets/{id}?tweet.fields=public_metrics,created_at,conversation_id&expansions=author_id&user.fields=username,name"
        ))?;
        let thread = parse_thread(&body, id)?;
        let resources = 1 + thread.replies.len();
        Ok(official_served(
            thread,
            resources,
            self.config.post_read_usd,
            &format!("{resources} posts"),
        ))
    }
    fn search(&self, query: &str, recent: bool, limit: usize) -> Result<Served<Vec<Post>>, String> {
        let _ = recent;
        let share = self.share();
        let url = format!(
            "https://api.x.com/2/tweets/search/recent?query={}&max_results={limit}&tweet.fields=public_metrics,created_at,author_id&expansions=author_id&user.fields=username,name",
            crate::text::percent_encode(query)
        );
        let body = share.get(&url)?;
        let posts = parse_posts(&body)?;
        let resources = posts.len();
        Ok(official_served(
            posts,
            resources,
            self.config.post_read_usd,
            &format!("{resources} posts"),
        ))
    }
    fn timeline(&self, limit: usize) -> Result<Served<Vec<Post>>, String> {
        let share = self.share();
        let me = share.get("https://api.x.com/2/users/me")?;
        let user_id = serde_json::from_str::<Value>(&me)
            .ok()
            .and_then(|value| value["data"]["id"].as_str().map(str::to_string))
            .ok_or_else(|| "official API did not return the current user".to_string())?;
        let body = share.get(&format!(
            "https://api.x.com/2/users/{user_id}/timelines/reverse_chronological?max_results={limit}&tweet.fields=public_metrics,created_at&expansions=author_id&user.fields=username,name"
        ))?;
        let posts = parse_posts(&body)?;
        let resources = posts.len();
        Ok(official_served(
            posts,
            resources,
            self.config.post_read_usd,
            &format!("{resources} posts"),
        ))
    }
    fn timeline_feed(&self, feed: TimelineFeed, limit: usize) -> Result<Served<Vec<Post>>, String> {
        match feed {
            // reverse_chronological is X's Following home timeline.
            TimelineFeed::Following => self.timeline(limit),
            TimelineFeed::ForYou => {
                Err("official API exposes only the Following (reverse chronological) home timeline, not For you".into())
            }
        }
    }
    fn trends(&self, place: Option<&str>, limit: usize) -> Result<Served<Vec<Trend>>, String> {
        let _ = (place, limit);
        Err("official trends are not on the free path; derive from search".into())
    }
}

struct OfficialPublisher {
    inner: OfficialShare,
}

impl Publisher for OfficialPublisher {
    fn id(&self) -> ProviderId {
        ProviderId::OfficialApi
    }
    fn available(&self) -> bool {
        self.inner.config.official_enabled
    }
    fn session_note(&self) -> Option<String> {
        Some(format!(
            "est. ${:.3} per post write",
            self.inner.config.post_write_usd
        ))
    }
    fn publish(
        &self,
        parts: &[String],
        reply_to: Option<&str>,
        media: &[PathBuf],
    ) -> Result<String, String> {
        if !media.is_empty() {
            return Err(
                "official API media upload is not wired in Termy X yet; use OpenCLI --images or attach in the browser".into(),
            );
        }
        let token = self.inner.token()?;
        let mut ids = Vec::new();
        for (index, part) in parts.iter().enumerate() {
            let mut body = serde_json::json!({ "text": part });
            if index == 0 {
                if let Some(reply) = reply_to {
                    body["reply"] = serde_json::json!({ "in_reply_to_tweet_id": reply });
                }
            } else if let Some(previous) = ids.last() {
                body["reply"] = serde_json::json!({ "in_reply_to_tweet_id": previous });
            }
            let response = self.inner.http.request(HttpRequest {
                method: "POST",
                url: "https://api.x.com/2/tweets".into(),
                body: Some(body.to_string()),
                bearer: Some(token.clone()),
                content_type: Some("application/json".into()),
                headers: Vec::new(),
            })?;
            let id = serde_json::from_str::<Value>(&response)
                .ok()
                .and_then(|value| value["data"]["id"].as_str().map(str::to_string));
            let Some(id) = id.filter(|id| !id.is_empty() && id.chars().all(|ch| ch.is_ascii_digit())) else {
                return Err(
                    "Post was not confirmed. The X API response had no status id. Not marked Posted. Nothing was retried."
                        .into(),
                );
            };
            ids.push(id);
        }
        Ok(format!(
            "official API posted {}",
            ids.last().cloned().unwrap_or_else(|| "posted".into())
        ))
    }
}

fn official_served<T>(value: T, resources: usize, unit: f64, detail: &str) -> Served<T> {
    Served {
        value,
        status: StatusLine {
            provider: ProviderId::OfficialApi,
            session: None,
            credits: "X API credits".into(),
            cost: Some(CostEstimate {
                usd: unit * resources as f64,
                resources,
                detail: format!("{detail} × ${unit:.3} estimate"),
            }),
            attempts: Vec::new(),
            note: "estimate, verify current X pricing".into(),
        },
    }
}

pub fn research_topic(
    topic: &str,
    runner: &dyn CommandRunner,
    config: &XConfig,
) -> Vec<ResearchHit> {
    let mut hits = Vec::new();
    if runner.exists(&config.opencli_bin) {
        if let Ok(output) = runner.run(
            &config.opencli_bin,
            &opencli_read(&[
                "reddit".into(),
                "search".into(),
                topic.into(),
                "--limit".into(),
                "5".into(),
            ]),
        ) && output.status == 0
        {
            hits.extend(parse_research("reddit", &output.stdout, Some(SESSION_OWN)));
        }
        if let Ok(output) = runner.run(
            &config.opencli_bin,
            &opencli_read(&["youtube".into(), "search".into(), topic.into()]),
        ) && output.status == 0
        {
            hits.extend(parse_research("youtube", &output.stdout, Some(SESSION_OWN)));
        }
        // Labeled read-only lanes where OpenCLI has a real query
        // search today. Each is cookie-session/background like the X
        // reads; a platform that isn't logged in or is walled simply
        // contributes no hits — never an error, never a browser pop.
        // Instagram is deliberately absent: its search is users-only,
        // not topic content.
        if let Ok(output) = runner.run(
            &config.opencli_bin,
            &opencli_read(&[
                "tiktok".into(),
                "search".into(),
                topic.into(),
                "--limit".into(),
                "5".into(),
            ]),
        ) && output.status == 0
        {
            hits.extend(parse_tiktok(&output.stdout));
        }
        if let Ok(output) = runner.run(
            &config.opencli_bin,
            &opencli_read(&[
                "pinterest".into(),
                "search-pins".into(),
                topic.into(),
                "--limit".into(),
                "5".into(),
            ]),
        ) && output.status == 0
        {
            hits.extend(parse_research(
                "pinterest",
                &output.stdout,
                Some(SESSION_OWN),
            ));
        }
        if let Ok(output) = runner.run(
            &config.opencli_bin,
            &opencli_read(&[
                "facebook".into(),
                "search".into(),
                topic.into(),
                "--limit".into(),
                "5".into(),
            ]),
        ) && output.status == 0
        {
            hits.extend(parse_facebook(&output.stdout));
        }
    }
    if runner.exists(&config.youtube_bin) {
        let query = format!("ytsearch5:{topic}");
        if let Ok(output) = runner.run(
            &config.youtube_bin,
            &[
                "--flat-playlist".into(),
                "--print".into(),
                "%(title)s\t%(webpage_url)s".into(),
                query,
            ],
        ) && output.status == 0
        {
            for line in output.stdout.lines().take(5) {
                let mut parts = line.split('\t');
                let title = parts.next().unwrap_or("").trim();
                let url = parts.next().unwrap_or("").trim();
                if title.is_empty() {
                    continue;
                }
                hits.push(ResearchHit {
                    source: "youtube".into(),
                    title: title.into(),
                    url: url.into(),
                    excerpt: String::new(),
                    session: None,
                });
            }
        }
    }
    if runner.exists("curl") {
        let reader = format!(
            "https://r.jina.ai/https://lite.duckduckgo.com/lite/?q={}",
            crate::text::percent_encode(topic)
        );
        if let Ok(output) = runner.run(
            "curl",
            &["-fsSL".into(), "--max-time".into(), "15".into(), reader],
        ) && output.status == 0
        {
            let excerpt = output
                .stdout
                .chars()
                .take(280)
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            if !excerpt.is_empty() {
                hits.push(ResearchHit {
                    source: "web".into(),
                    title: format!("Web notes for {topic}"),
                    url: "https://r.jina.ai/".to_string(),
                    excerpt,
                    session: None,
                });
            }
        }
    }
    hits.extend(rss_hits(topic, runner));
    hits
}

fn rss_hits(topic: &str, runner: &dyn CommandRunner) -> Vec<ResearchHit> {
    if !runner.exists("curl") {
        return Vec::new();
    }
    let url = format!(
        "https://www.reddit.com/search.rss?q={}&sort=new&limit=5",
        crate::text::percent_encode(topic)
    );
    let Ok(output) = runner.run(
        "curl",
        &[
            "-fsSL".into(),
            "--max-time".into(),
            "15".into(),
            "-A".into(),
            "termy-x".into(),
            url,
        ],
    ) else {
        return Vec::new();
    };
    if output.status != 0 {
        return Vec::new();
    }
    parse_rss(&output.stdout).into_iter().take(5).collect()
}

fn parse_rss(xml: &str) -> Vec<ResearchHit> {
    let mut hits = Vec::new();
    for item in xml.split("<item").skip(1) {
        let title = xml_text(item, "title");
        let link = xml_text(item, "link");
        if title.is_empty() {
            continue;
        }
        hits.push(ResearchHit {
            source: "rss".into(),
            title,
            url: link,
            excerpt: xml_text(item, "description"),
            session: None,
        });
    }
    hits
}

fn xml_text(item: &str, tag: &str) -> String {
    let open = format!("<{tag}");
    let Some(start) = item.find(&open) else {
        return String::new();
    };
    let Some(content_at) = item[start..].find('>') else {
        return String::new();
    };
    let from = start + content_at + 1;
    let close = format!("</{tag}>");
    let end = item[from..].find(&close).map_or(from, |index| from + index);
    item[from..end]
        .replace("<![CDATA[", "")
        .replace("]]>", "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// TikTok search rows: {rank, desc, author, url, plays, likes,
/// comments, shares}. Title is the caption; stats go to the excerpt.
fn parse_tiktok(text: &str) -> Vec<ResearchHit> {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    array_of(&value)
        .into_iter()
        .map(|item| {
            let title = first_str(item, &["desc", "text", "title"]).unwrap_or("TikTok video");
            let author = first_str(item, &["author"]).unwrap_or("");
            let mut stats: Vec<String> = Vec::new();
            if !author.is_empty() {
                stats.push(format!("@{author}"));
            }
            for (key, label) in [
                ("plays", "plays"),
                ("likes", "likes"),
                ("comments", "comments"),
                ("shares", "shares"),
            ] {
                if let Some(n) = item.get(key).and_then(Value::as_u64) {
                    stats.push(format!("{n} {label}"));
                }
            }
            ResearchHit {
                source: "tiktok".into(),
                title: title.to_string(),
                url: first_str(item, &["url", "link"]).unwrap_or("").to_string(),
                excerpt: stats.join(" \u{b7} "),
                session: Some(SESSION_OWN.to_string()),
            }
        })
        .collect()
}

/// Facebook search rows: {index, title, text, url} mixing people,
/// pages, and posts — labeled as one facebook lane, text as excerpt.
fn parse_facebook(text: &str) -> Vec<ResearchHit> {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    array_of(&value)
        .into_iter()
        .map(|item| {
            let excerpt = first_str(item, &["text", "description", "body"]).unwrap_or("");
            ResearchHit {
                source: "facebook".into(),
                title: first_str(item, &["title", "name"])
                    .unwrap_or("Facebook result")
                    .to_string(),
                url: first_str(item, &["url", "link"]).unwrap_or("").to_string(),
                excerpt: excerpt.chars().take(240).collect(),
                session: Some(SESSION_OWN.to_string()),
            }
        })
        .collect()
}

fn parse_research(source: &str, text: &str, session: Option<&str>) -> Vec<ResearchHit> {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    array_of(&value)
        .into_iter()
        .map(|item| {
            let title = first_str(item, &["title", "name", "text"]).unwrap_or("untitled");
            let url = first_str(item, &["url", "link", "permalink"]).unwrap_or("");
            let excerpt =
                first_str(item, &["excerpt", "body", "selftext", "description"]).unwrap_or("");
            ResearchHit {
                source: source.into(),
                title: title.to_string(),
                url: url.to_string(),
                excerpt: excerpt.chars().take(240).collect(),
                session: session.map(str::to_string),
            }
        })
        .collect()
}

pub fn parse_posts(text: &str) -> Result<Vec<Post>, String> {
    // Zero-copy fast path (borrowed &'a str fields straight from `text`);
    // any shape it does not model exactly falls back to the legacy
    // `Value`-tree parser below, so behaviour is unchanged.
    if let Some(posts) = crate::parse_borrowed::parse_posts_borrowed(text) {
        return Ok(posts);
    }
    let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let posts = posts_from_value(&value);
    if posts.is_empty() {
        Err("no posts in provider response".into())
    } else {
        Ok(posts)
    }
}

/// Legacy `Value`-tree conversion, shared by `parse_posts` (fallback)
/// and the eval-payload parsers (which already hold a `Value`).
fn posts_from_value(value: &Value) -> Vec<Post> {
    let users = users_by_id(value);
    array_of(value)
        .into_iter()
        .filter_map(|item| post_from_value(item, &users))
        .collect()
}

fn parse_thread(text: &str, fallback_id: &str) -> Result<Thread, String> {
    let posts = parse_posts(text)?;
    let mut posts = posts;
    let root_index = posts
        .iter()
        .position(|post| post.id == fallback_id)
        .unwrap_or(0);
    let root = posts.remove(root_index);
    Ok(Thread {
        root,
        replies: posts,
    })
}

/// Avatar/banner URLs live at the top level in OpenCLI rows and under
/// `legacy`/`user` in GraphQL/API shapes. Only real payload values are
/// returned — never a synthesized CDN URL.
fn profile_image_str(node: &Value, keys: &[&str]) -> Option<String> {
    let containers = [
        Some(node),
        node.get("legacy"),
        node.get("user"),
        node.get("profile"),
    ];
    for container in containers.into_iter().flatten() {
        if let Some(found) = first_str(container, keys) {
            let found = found.trim();
            if found.starts_with("http") {
                return Some(found.to_string());
            }
        }
    }
    None
}

/// The payload's own word on whether the connected account follows
/// this profile: a boolean `following` (or is_following /
/// viewer_following) on the user object or its `legacy` sub-object.
/// A numeric `following` is the following-COUNT, not this flag —
/// `as_bool` correctly ignores it. Absent flag -> None, never guessed.
fn viewer_following_from(node: &Value) -> Option<bool> {
    for key in ["following", "is_following", "viewer_following"] {
        if let Some(flag) = node.get(key).and_then(Value::as_bool) {
            return Some(flag);
        }
    }
    node.get("legacy").and_then(viewer_following_from)
}

fn parse_profile(text: &str, fallback_handle: &str) -> Result<Profile, String> {
    let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let node = value
        .get("data")
        .or_else(|| value.get("user"))
        .unwrap_or(&value);
    // OpenCLI returns a profile as a one-row JSON array (1.8.8 and 2.x).
    let node = node
        .as_array()
        .and_then(|rows| rows.first())
        .unwrap_or(node);
    let handle = first_str(node, &["username", "screen_name", "screenName", "handle"])
        .unwrap_or(fallback_handle)
        .trim_start_matches('@')
        .to_string();
    let name = first_str(node, &["name", "display_name"])
        .unwrap_or(&handle)
        .to_string();
    let bio = first_str(node, &["description", "bio", "summary"])
        .unwrap_or("")
        .to_string();
    let totals = crate::stats::profile_totals_from_value(&value);
    let posts = posts_from_value(&value);
    Ok(Profile {
        handle,
        name,
        bio,
        followers: totals.followers,
        following: totals.following,
        tweets: totals.tweets,
        likes: totals.likes,
        verified: totals.verified,
        created_at: totals.created_at,
        avatar_url: profile_image_str(
            node,
            &[
                "profile_image_url_https",
                "profile_image_url",
                "avatar_url",
                "avatarUrl",
                "avatar",
            ],
        ),
        banner_url: profile_image_str(
            node,
            &[
                "profile_banner_url_https",
                "profile_banner_url",
                "banner_url",
                "bannerUrl",
                "cover_url",
            ],
        ),
        viewer_following: viewer_following_from(node),
        posts,
        pinned: Vec::new(),
    })
}

fn parse_trends(text: &str, place: Option<&str>) -> Result<Vec<Trend>, String> {
    let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let trends = array_of(&value)
        .into_iter()
        .filter_map(|item| {
            let name = first_str(item, &["name", "title", "topic", "trend", "query"])?.to_string();
            Some(Trend {
                query: first_str(item, &["query", "name"])
                    .unwrap_or(&name)
                    .to_string(),
                // The live trending payload has no volume (X dropped
                // post counts); keep parsing it for payloads that do.
                volume: item
                    .get("volume")
                    .or_else(|| item.get("tweet_volume"))
                    .or_else(|| item.get("tweets"))
                    .and_then(|v| {
                        v.as_u64().or_else(|| {
                            v.as_str().and_then(|s| {
                                s.chars()
                                    .filter(char::is_ascii_digit)
                                    .collect::<String>()
                                    .parse()
                                    .ok()
                            })
                        })
                    }),
                place: place.map(str::to_string),
                rank: item.get("rank").and_then(Value::as_u64),
                category: first_str(item, &["category", "context", "category_name"])
                    .map(str::to_string),
                name,
            })
        })
        .collect::<Vec<_>>();
    if trends.is_empty() {
        Err("no trends".into())
    } else {
        Ok(trends)
    }
}

fn array_of(value: &Value) -> Vec<&Value> {
    if let Some(array) = value.as_array() {
        return array.iter().collect();
    }
    for key in ["data", "tweets", "items", "rows", "results", "timeline"] {
        if let Some(array) = value.get(key).and_then(Value::as_array) {
            return array.iter().collect();
        }
    }
    if value.get("text").is_some() || value.get("full_text").is_some() || value.get("id").is_some()
    {
        return vec![value];
    }
    Vec::new()
}

fn users_by_id(value: &Value) -> Vec<(String, String, String)> {
    value
        .pointer("/includes/users")
        .and_then(Value::as_array)
        .map(|users| {
            users
                .iter()
                .filter_map(|user| {
                    Some((
                        user.get("id")?.as_str()?.to_string(),
                        first_str(user, &["username", "screen_name"])?.to_string(),
                        first_str(user, &["name"]).unwrap_or("").to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn post_from_value(item: &Value, users: &[(String, String, String)]) -> Option<Post> {
    let text = first_str(item, &["text", "full_text", "rawContent", "content"])
        .unwrap_or("")
        .to_string();
    let id = first_str(item, &["id", "id_str", "rest_id"])
        .unwrap_or("0")
        .to_string();
    let author = item.get("user").or_else(|| item.get("author"));
    let (mut handle, mut name) = match author {
        Some(Value::String(text)) => (
            text.trim().trim_start_matches('@').to_string(),
            String::new(),
        ),
        Some(author) => (
            first_str(author, &["username", "screen_name", "screenName", "handle"])
                .unwrap_or("")
                .trim_start_matches('@')
                .to_string(),
            first_str(author, &["name", "display_name"])
                .unwrap_or("")
                .to_string(),
        ),
        None => (String::new(), String::new()),
    };
    if handle.is_empty() {
        handle = first_str(item, &["username", "screen_name", "screenName", "author"])
            .unwrap_or("")
            .trim_start_matches('@')
            .to_string();
    }
    if handle.is_empty()
        && let Some(author_id) = item.get("author_id").and_then(Value::as_str)
        && let Some((_, username, display)) = users.iter().find(|(id, _, _)| id == author_id)
    {
        handle = username.clone();
        name = display.clone();
    }
    if name.is_empty() {
        name = handle.clone();
    }
    if text.is_empty() && (id.is_empty() || id == "0") && handle.is_empty() {
        return None;
    }
    let metrics = item
        .get("public_metrics")
        .or_else(|| item.get("metrics"))
        .unwrap_or(item);
    Some(Post {
        url: post_url(&handle, &id),
        author_handle: handle,
        author_name: name,
        created_at: first_str(item, &["created_at", "createdAt", "time"]).map(str::to_string),
        likes: number(metrics, &["like_count", "likes", "favorite_count"]),
        reposts: number(metrics, &["retweet_count", "reposts", "retweets"]),
        replies: number(metrics, &["reply_count", "replies"]),
        bookmarks: number(metrics, &["bookmark_count", "bookmarks"]),
        views: number(metrics, &["impression_count", "views", "view_count"]),
        quotes: crate::stats::quote_count_from_metrics(metrics, item),
        text,
        id,
        media: media_from_value(item),
        retweeted: embedded_post(item, &["retweeted", "retweeted_status", "retweeted_tweet", "retweeted_status_result"]),
        quoted: embedded_post(item, &["quoted", "quoted_status", "quoted_tweet", "quoted_status_result"]),
        referenced_status_id: first_id(item, &["retweeted_status_id_str", "retweeted_status_id", "referenced_status_id"]),
        unavailable_original_url: tombstone_url(item),
        in_reply_to: in_reply_to_from(item),
        viewer_liked: viewer_liked_from(item),
        viewer_follows_author: author
            .and_then(viewer_following_from)
            .or_else(|| viewer_following_from(item)),
    })
}

fn in_reply_to_from(item: &Value) -> Option<String> {
    const KEYS: &[&str] = &[
        "in_reply_to_status_id_str",
        "in_reply_to_status_id",
        "in_reply_to_tweet_id",
        "inReplyToStatusId",
        "in_reply_to",
        "reply_to",
    ];
    first_id(item, KEYS)
        .or_else(|| item.get("legacy").and_then(in_reply_to_from))
        .or_else(|| item.get("reply").and_then(|reply| first_id(reply, KEYS)))
}

fn viewer_liked_from(item: &Value) -> Option<bool> {
    first_bool(
        item,
        &["favorited", "liked", "viewer_liked", "favorited_by_viewer"],
    )
    .or_else(|| item.get("legacy").and_then(viewer_liked_from))
}

/// First embedded-tweet object among `keys` (GraphQL eval rows emit
/// `retweeted`; some API-shaped payloads use the other names).
fn first_embedded<'a>(item: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .filter_map(|key| item.get(*key))
        .find(|value| value.is_object())
}

fn unwrap_tweet(value: &Value) -> &Value {
    let mut cur = value;
    for _ in 0..4 {
        let next = cur
            .get("result")
            .filter(|inner| inner.is_object())
            .or_else(|| {
                let wrapped = cur.get("tweet").filter(|inner| inner.is_object())?;
                if cur.get("text").is_some() || cur.get("full_text").is_some() {
                    None
                } else {
                    Some(wrapped)
                }
            });
        match next {
            Some(inner) => cur = inner,
            None => break,
        }
    }
    cur
}

fn is_tombstone(value: &Value) -> bool {
    first_str(value, &["__typename", "typename"])
        .is_some_and(|kind| kind.contains("Unavailable") || kind.contains("Tombstone"))
        || value.get("tombstone").is_some()
}

fn embedded_post(item: &Value, keys: &[&str]) -> Option<Box<Post>> {
    let raw = first_embedded(item, keys)?;
    let inner = unwrap_tweet(raw);
    if is_tombstone(inner) {
        return None;
    }
    post_from_value(inner, &[]).map(Box::new)
}

fn tombstone_url(item: &Value) -> Option<String> {
    for key in ["retweeted_status_result", "retweeted", "retweeted_status"] {
        let Some(raw) = item.get(key).filter(|value| value.is_object()) else {
            continue;
        };
        let inner = unwrap_tweet(raw);
        if !is_tombstone(inner) {
            continue;
        }
        let id = first_id(inner, &["id", "id_str", "rest_id"])
            .or_else(|| first_id(item, &["retweeted_status_id_str", "retweeted_status_id"]))?;
        return Some(format!("https://x.com/i/status/{id}"));
    }
    None
}

/// Media from provider payloads. Verified live against OpenCLI
/// `twitter timeline -f json` (2026-10-18): rows carry `media_urls`
/// (photo JPGs on pbs.twimg.com, mp4s on video.twimg.com) with parallel
/// `media_posters` thumbnails. Also accepts official-API style
/// `includes.media` / `attachments` and simple `media`/`images` arrays.
fn media_from_value(item: &Value) -> Vec<crate::model::MediaItem> {
    use crate::model::{MediaItem, MediaKind};
    let kind_of = crate::parse_borrowed::media_kind;
    let mut out: Vec<MediaItem> = Vec::new();
    if let Some(urls) = item.get("media_urls").and_then(Value::as_array) {
        let posters = item.get("media_posters").and_then(Value::as_array);
        for (i, url) in urls.iter().filter_map(Value::as_str).enumerate() {
            if url.is_empty() {
                continue;
            }
            let kind = kind_of(url);
            let poster = posters
                .and_then(|p| p.get(i))
                .and_then(Value::as_str)
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .or_else(|| (kind == MediaKind::Photo).then(|| url.to_string()));
            let (width, height) = match crate::model::dims_from_media_url(url) {
                Some((w, h)) => (Some(w), Some(h)),
                None => (None, None),
            };
            out.push(MediaItem {
                kind,
                url: url.to_string(),
                poster,
                width,
                height,
            });
        }
        return out;
    }
    if let Some(list) = item
        .get("media")
        .and_then(Value::as_array)
        .or_else(|| item.get("images").and_then(Value::as_array))
    {
        for entry in list {
            let (url, poster) = match entry {
                Value::String(u) => (u.clone(), None),
                Value::Object(_) => (
                    first_str(
                        entry,
                        &["url", "media_url", "media_url_https", "preview_image_url"],
                    )
                    .unwrap_or("")
                    .to_string(),
                    first_str(entry, &["preview_image_url", "poster", "thumbnail"])
                        .map(str::to_string),
                ),
                _ => (String::new(), None),
            };
            if url.is_empty() {
                continue;
            }
            let dims = crate::model::dims_from_media_url(&url);
            let (width, height) = match (
                entry.get("width").and_then(Value::as_u64),
                entry.get("height").and_then(Value::as_u64),
            ) {
                (Some(w), Some(h)) => (Some(w as u32), Some(h as u32)),
                _ => match dims {
                    Some((w, h)) => (Some(w), Some(h)),
                    None => (None, None),
                },
            };
            let kind = match first_str(entry, &["type", "kind"]) {
                Some("video" | "animated_gif") => {
                    if first_str(entry, &["type"]) == Some("animated_gif") {
                        MediaKind::Gif
                    } else {
                        MediaKind::Video
                    }
                }
                _ => kind_of(&url),
            };
            let poster = poster.or_else(|| (kind == MediaKind::Photo).then(|| url.clone()));
            out.push(MediaItem {
                kind,
                url,
                poster,
                width,
                height,
            });
        }
    }
    out
}

#[cfg(test)]
mod media_parse_tests {
    use super::*;
    use crate::model::MediaKind;

    #[test]
    fn opencli_media_urls_and_posters_parse_photo_and_video() {
        // Exact field names from a live `opencli twitter timeline -f json`
        // row (2026-10-18): video mp4 + poster, and a photo row.
        let json = r#"[
          {"id":"1","author":"a","text":"vid https://t.co/x","likes":1,"media_urls":["https://video.twimg.com/amplify_video/9/vid/avc1/320x568/a.mp4?tag=29"],"media_posters":["https://pbs.twimg.com/amplify_video_thumb/9/img/p.jpg"],"has_media":true},
          {"id":"2","author":"b","text":"pic https://t.co/y","likes":2,"media_urls":["https://pbs.twimg.com/media/ABC.jpg"],"media_posters":[],"has_media":true},
          {"id":"3","author":"c","text":"plain","likes":3}
        ]"#;
        let posts = parse_posts(json).unwrap();
        assert_eq!(posts[0].media.len(), 1);
        assert_eq!(posts[0].media[0].kind, MediaKind::Video);
        assert_eq!(
            posts[0].media[0].poster.as_deref(),
            Some("https://pbs.twimg.com/amplify_video_thumb/9/img/p.jpg")
        );
        assert_eq!(posts[1].media[0].kind, MediaKind::Photo);
        assert_eq!(
            posts[1].media[0].poster.as_deref(),
            Some("https://pbs.twimg.com/media/ABC.jpg")
        );
        assert!(posts[2].media.is_empty());
        // Video URLs state their size (/480x270/ style segment, here 320x568 in fixture? use parsed below)
        assert_eq!(posts[0].media[0].width, Some(320));
        assert_eq!(posts[0].media[0].height, Some(568));
        assert_eq!(posts[1].media[0].width, None);
    }

    #[test]
    fn dims_from_media_url_parses_only_plausible_segments() {
        use crate::model::dims_from_media_url;
        assert_eq!(
            dims_from_media_url(
                "https://video.twimg.com/amplify_video/9/vid/avc1/480x270/a.mp4?tag=29"
            ),
            Some((480, 270))
        );
        assert_eq!(
            dims_from_media_url("https://pbs.twimg.com/media/ABC.jpg"),
            None
        );
        assert_eq!(
            dims_from_media_url("https://pbs.twimg.com/media/ABC.jpg?name=4096x4096"),
            None
        );
        assert_eq!(dims_from_media_url("https://x.com/1x1/a.jpg"), None);
    }
}

#[cfg(test)]
mod borrowed_parse_tests {
    use super::*;

    fn legacy(text: &str) -> Vec<Post> {
        let value: Value = serde_json::from_str(text).unwrap();
        posts_from_value(&value)
    }

    #[test]
    fn borrowed_matches_legacy_on_modelled_and_fallback_shapes() {
        let cases: &[&str] = &[
            r#"[{"id":"50","author":"reposter","text":"RT @orig: hi","likes":1,"retweets":2,"retweeted":{"id":"40","author":"orig","text":"hi","likes":99,"media_urls":["https://pbs.twimg.com/media/a.jpg"],"media_posters":[]}},{"id":"51","author":"plain","text":"no embed","likes":0}]"#,
            r#"{"data":[{"id":"7","text":"api row","author_id":"u1","public_metrics":{"like_count":5,"retweet_count":2,"reply_count":1,"bookmark_count":3,"impression_count":50}}],"includes":{"users":[{"id":"u1","username":"ada","name":"Ada L"}]}}"#,
            r#"[{"id":"8","full_text":"obj author","user":{"screen_name":"bob","display_name":"Bob"},"metrics":{"likes":4}}]"#,
            r#"[{"id":"9","author":"ada","text":"quirk","likes":42,"public_metrics":{}}]"#,
            "[{\"id\":\"10\",\"author\":\"ada\",\"text\":\"line\\nbreak \\u2603\"}]",
            r#"[{"id":"11","author":"ada","text":"media obj","media":[{"url":"https://video.twimg.com/x/480x270/a.mp4","type":"video"}]}]"#,
            r#"[{"id":12,"author":"ada","text":"numeric id"}]"#,
            r#"{"id":"13","author":"ada","text":"single","likes":2}"#,
            r#"{"tweets":[{"id":"14","author":"ada","text":"env","view_count":9}]}"#,
            r#"[{"id":"15","author":"ada","text":"string counts","likes":"12"}]"#,
        ];
        for (i, json) in cases.iter().enumerate() {
            assert_eq!(parse_posts(json).unwrap(), legacy(json), "case {i}");
        }
    }

    #[test]
    fn borrowed_eval_parsers_match_legacy_results() {
        let tl = r#"{"posts":[{"id":"900","author":"ada","text":"hi","likes":3,"retweeted":{"id":"800","author":"orig","text":"orig","likes":9}}],"nextCursor":"CUR2"}"#;
        let page = parse_timeline_eval(tl).unwrap();
        assert_eq!(page.posts.len(), 1);
        assert_eq!(page.posts[0].retweeted.as_ref().unwrap().id, "800");
        assert_eq!(page.next_cursor.as_deref(), Some("CUR2"));
        let prof = r#"{"posts":[{"id":"30","author":"someone","text":"newest"}],"pinned":{"id":"20","author":"someone","text":"pinned","likes":9}}"#;
        let (latest, pinned) = parse_profile_eval(prof).unwrap();
        assert_eq!(latest.len(), 1);
        assert_eq!(pinned[0].id, "20");
    }

    #[test]
    fn ui_page_collects_100_unique_then_continues() {
        let posts = home_fixture(TimelineFeed::ForYou);
        assert!(posts.len() > HOME_UI_PAGE * 2);
        let first = accumulate_fixture(&posts, None, HOME_UI_PAGE);
        assert!(
            first.posts.len() >= HOME_UI_PAGE,
            "got {}",
            first.posts.len()
        );
        assert!(first.posts.len() <= HOME_UI_PAGE + HOME_PROVIDER_PAGE);
        let cursor = first.next_cursor.clone().expect("more than one page");
        let second = accumulate_fixture(&posts, Some(&cursor), HOME_UI_PAGE);
        assert!(second.posts.len() >= HOME_UI_PAGE);
        let kept = append_new_posts(first.posts.clone(), second.posts.clone());
        assert_eq!(kept.len(), first.posts.len() + second.posts.len());
        let mut popular = kept.clone();
        popular.sort_by(|left, right| {
            crate::sort::cmp_posts(left, right, crate::sort::ResultSort::Popular)
        });
        let mut newest = kept.clone();
        newest.sort_by(|left, right| {
            crate::sort::cmp_posts(left, right, crate::sort::ResultSort::Newest)
        });
        assert_ne!(popular[0].id, newest[0].id);
    }

    #[test]
    fn short_provider_stops_below_100_without_padding() {
        let posts = home_fixture(TimelineFeed::ForYou);
        let short = &posts[..40];
        let page = accumulate_fixture(short, None, HOME_UI_PAGE);
        assert_eq!(page.posts.len(), 40);
        assert!(page.next_cursor.is_none());
        assert!(page.posts.len() < HOME_UI_PAGE);
    }

    #[test]
    fn repeated_cursor_stops_and_duplicate_ids_collapse() {
        let page = accumulate_timeline_pages(
            |cursor| {
                if cursor.is_none() {
                    TimelinePage {
                        posts: vec![
                            post("1", "a", "A", "one", 3, 0, 0, 0),
                            post("2", "a", "A", "two", 1, 0, 0, 0),
                        ],
                        next_cursor: Some("same".into()),
                    }
                } else {
                    TimelinePage {
                        posts: vec![
                            post("2", "a", "A", "two-again", 1, 0, 0, 0),
                            post("3", "a", "A", "three", 9, 0, 0, 0),
                        ],
                        next_cursor: Some("same".into()),
                    }
                }
            },
            None,
            HOME_UI_PAGE,
        );
        let ids: Vec<&str> = page.posts.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(ids, vec!["1", "2", "3"]);
        assert!(page.next_cursor.is_none());
        assert!(page.posts.len() < HOME_UI_PAGE);
    }
}

fn first_str<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
}

fn first_id(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|node| match node {
            Value::String(text) => {
                let text = text.trim();
                (!text.is_empty() && text != "0").then(|| text.to_string())
            }
            Value::Number(n) => n
                .as_u64()
                .filter(|n| *n > 0)
                .map(|n| n.to_string())
                .or_else(|| n.as_i64().filter(|n| *n > 0).map(|n| n.to_string())),
            _ => None,
        })
    })
}

fn first_bool(value: &Value, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_bool))
}

fn number(value: &Value, keys: &[&str]) -> u64 {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_u64))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::ScriptedRunner;
    use crate::publish::IntentPublisher;
    use std::sync::Arc;

    fn quiet_config() -> XConfig {
        XConfig {
            min_interval_ms: 0,
            ..XConfig::default()
        }
    }

    #[test]
    fn default_readers_are_live_only_and_do_not_serve_fixtures() {
        let runner = Arc::new(ScriptedRunner::new());
        let http = Arc::new(crate::io::MapHttp::default());
        let service = Service::free(&quiet_config(), runner, http);
        assert_eq!(
            service.reader_ids(),
            vec![ProviderId::OpenCli, ProviderId::TwitterCli]
        );
        assert!(!service.reader_ids().contains(&ProviderId::Mock));
        assert!(!service.reader_ids().contains(&ProviderId::OfficialApi));
        let error = service.timeline(100).unwrap_err();
        assert!(
            !error.contains("fy home post") && !error.contains("home post"),
            "{error}"
        );
        let page = service.timeline_page(TimelineFeed::ForYou, None, 100);
        assert!(page.is_err());
        if let Err(error) = page {
            assert!(!error.contains("fy home post"), "{error}");
        }
    }

    #[test]
    fn fixtures_require_an_explicit_mock_provider() {
        let mut config = quiet_config();
        config.provider = Some(ProviderId::Mock);
        let service = Service::free(
            &config,
            Arc::new(ScriptedRunner::new()),
            Arc::new(crate::io::MapHttp::default()),
        );
        assert_eq!(service.reader_ids(), vec![ProviderId::Mock]);
        let page = service
            .timeline_page(TimelineFeed::ForYou, None, HOME_UI_PAGE)
            .unwrap();
        assert_eq!(page.status.provider, ProviderId::Mock);
        assert!(
            page.value
                .posts
                .iter()
                .any(|post| post.text.contains("fy home post"))
        );
    }

    #[test]
    fn append_new_posts_only_appends_unseen_ids() {
        let mk = |id: &str| Post {
            id: id.into(),
            author_handle: "a".into(),
            author_name: "a".into(),
            text: "t".into(),
            ..Post::default()
        };
        let existing = vec![mk("300"), mk("200"), mk("100")];
        // Batch overlaps the loaded tail and extends below it.
        let incoming = vec![mk("200"), mk("90"), mk("100"), mk("80"), mk("90")];
        let merged = append_new_posts(existing, incoming);
        let ids: Vec<&str> = merged.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["300", "200", "100", "90", "80"]);
        // Nothing new: list is unchanged (no fake growth).
        let same = append_new_posts(merged.clone(), vec![mk("300"), mk("80")]);
        assert_eq!(same.len(), merged.len());
        // Empty-id posts are always appended (cannot be deduped).
        let with_blank = append_new_posts(merged, vec![mk("")]);
        assert_eq!(with_blank.len(), 6);
    }

    #[test]
    fn timeline_eval_parsing_keeps_pages_cursors_and_partial_progress() {
        // Clean page: posts parsed, cursor carried through.
        let page = parse_timeline_eval(
            r#"{"posts":[{"id":"900","author":"ada","text":"hi","likes":3,"retweets":1,"replies":0,"views":10,"created_at":"","url":"https://x.com/ada/status/900"}],"nextCursor":"CUR2"}"#,
        )
        .unwrap();
        assert_eq!(page.posts.len(), 1);
        assert_eq!(page.posts[0].id, "900");
        assert_eq!(page.next_cursor.as_deref(), Some("CUR2"));
        // Exhausted source: no cursor, no error.
        let dry = parse_timeline_eval(r#"{"posts":[],"nextCursor":null}"#).unwrap();
        assert!(dry.posts.is_empty());
        assert_eq!(dry.next_cursor, None);
        // Mid-walk rate limit with progress: keep the older posts.
        let partial = parse_timeline_eval(
            r#"{"error":"http-429","posts":[{"id":"800","author":"ada","text":"old","likes":0,"retweets":0,"replies":0,"views":0,"created_at":"","url":""}],"nextCursor":"CUR1"}"#,
        )
        .unwrap();
        assert_eq!(partial.posts.len(), 1);
        assert_eq!(partial.next_cursor.as_deref(), Some("CUR1"));
        // Hard failures with nothing fetched must surface.
        assert!(
            parse_timeline_eval(r#"{"error":"http-429","posts":[],"nextCursor":null}"#)
                .unwrap_err()
                .contains("429")
        );
        assert!(parse_timeline_eval("not json").is_err());
    }

    #[test]
    fn b64_and_eval_js_are_well_formed() {
        assert_eq!(b64_encode(b""), "");
        assert_eq!(b64_encode(b"f"), "Zg==");
        assert_eq!(b64_encode(b"fo"), "Zm8=");
        assert_eq!(b64_encode(b"foo"), "Zm9v");
        assert_eq!(b64_encode(b"foobarbaz"), "Zm9vYmFyYmF6");
        let js = timeline_eval_js(TimelineFeed::Following, Some("CUR\"X"), 160);
        assert!(js.contains("HomeLatestTimeline"));
        assert!(js.contains("CUR\\\"X") || js.contains("CUR"));
        assert!(!js.contains("auth_token"));
        let js_top = timeline_eval_js(TimelineFeed::ForYou, None, 160);
        assert!(js_top.contains("HomeTimeline"));
        assert!(js_top.contains("let cursor = null"));
    }

    #[test]
    fn eval_script_ships_in_cmd_safe_chunks() {
        // Regression: the whole base64 script is ~9.4 KB, over the
        // 8191-char cmd.exe limit that the Windows opencli.cmd shim
        // imposes; eval_timeline_page must ship it in <=6000-char
        // window-global pieces that reassemble exactly.
        let js = timeline_eval_js(TimelineFeed::Following, None, 580);
        let b64 = b64_encode(js.as_bytes());
        assert!(b64.len() > 6000, "script should force multi-chunk shipping");
        let pieces: Vec<&[u8]> = b64.as_bytes().chunks(6000).collect();
        assert!(pieces.len() >= 2);
        for piece in &pieces {
            assert!(piece.len() <= 6000);
            // cmd metacharacters must never appear in a shipped arg.
            let text = std::str::from_utf8(piece).unwrap();
            assert!(!text.contains(['|', '&', '<', '>', '^', '"', '\'']));
        }
        let joined: Vec<u8> = pieces.concat();
        assert_eq!(joined, b64.as_bytes());
    }

    #[test]
    fn gui_write_uses_the_opencli_receipt_once() {
        use crate::action_state::{ActionDb, ReceiptKind, ToggleKind};
        use crate::publish::{GuiWrite, GuiWriteOutcome, GuiWriteRequest, run_gui_write};
        let post_ok = r#"[{"status":"success","message":"Tweet posted successfully.","id":"42"}]"#;
        let reply_ok = r#"[{"status":"success","message":"Reply posted successfully."}]"#;
        let retweet_ok = r#"[{"status":"success","message":"Tweet successfully retweeted."}]"#;
        let quote_menu = r#"{"stage":"menu","path":"/cryptoXRPwhale/status/2118971448614334959","selected_label":"Quote","selected_scope":"menu","selected_testid":"-","selected_href_kind":"path=/compose/post keys=none","clicked_post":false,"card":false,"quote_testids":0,"message":"Quote row recorded. Post was not clicked."}"#;
        let quote_snap = r#"{"stage":"inspect","path":"/cryptoXRPwhale/status/2118971448614334959","composer":true,"composer_kind":"quote","card":true,"quote_testids":1,"quoted_status_id":"2118971448614334959","clicked_post":false,"clicked_repost_confirm":false,"message":"Quote attachment is visible. Post was not clicked."}"#;
        let quote_page = r#"{"stage":"posted","path":"/compose/post","testid_count":40,"composer":true,"composer_kind":"quote","card":true,"status_ids":["2118971448614334959"],"quote_testids":1,"clicked_post":true,"clicked_repost_confirm":false,"text_landed":true,"quoted_status_id":"2118971448614334959","new_status_id":"2119069999999999999","message":"Quote tweet posted successfully."}"#;
        let quote_read = r#"{"stage":"read","path":"/cryptoXRPwhale/status/2119069999999999999","composer_kind":"quote","card":true,"status_ids":["2118971448614334959"],"quote_testids":1,"quoted_status_id":"2118971448614334959","clicked_post":false,"clicked_repost_confirm":false,"message":"Read only. Post was not clicked."}"#;
        let runner = Arc::new(ScriptedRunner::new());
        runner.script("opencli", &["twitter", "post"], post_ok);
        runner.script("opencli", &["twitter", "reply"], reply_ok);
        runner.script("opencli", &["twitter", "retweet"], retweet_ok);
        runner.script(
            "opencli",
            &["browser", "--help"],
            "Usage: opencli browser <session> <command>\nbrowser <session> open <url>\n--window background\n",
        );
        runner.script(
            "opencli",
            &["browser", "termyx-quote", "open"],
            r#"{"ok":true,"data":{"url":"https://x.com/i/status/2118971448614334959","page":"A03BB6CCAE9402E75FCA7CB1AAED51FA"}}"#,
        );
        runner.script(
            "opencli",
            &["browser", "termyx-quote", "tab", "select"],
            r#"{"ok":true,"data":{"selected":"A03BB6CCAE9402E75FCA7CB1AAED51FA"}}"#,
        );
        runner.script("opencli", &["browser", "termyx-quote", "close"], "ok");
        runner.script("opencli", &["browser", "termyx-quote", "eval"], "https://x.com/i/status/2118971448614334959");
        let vis = crate::publish::quote_visibility_expr("2118971448614334959");
        runner.script(
            "opencli",
            &["browser", "termyx-quote", "eval", vis.as_str()],
            r#"{"ok":true,"data":"{\"v\":\"visible\",\"h\":false,\"p\":false,\"f\":true,\"t\":true,\"m\":true,\"host\":\"x.com\"}"}"#,
        );
        runner.script("opencli", &["browser", "termyx-quote", "eval", "eval(atob(window.__qx))"], quote_menu);
        runner.script(
            "opencli",
            &["browser", "termyx-quote", "click"],
            r#"{"ok":true,"data":{"clicked":true,"click_method":"cdp"}}"#,
        );
        runner.script("opencli", &["browser", "termyx-quote", "eval", "eval(atob(window.__qxSnap))"], quote_snap);
        runner.script("opencli", &["browser", "termyx-quote", "eval", "eval(atob(window.__qxPost))"], quote_page);
        runner.script("opencli", &["browser", "termyx-quote-read", "open"], "ok");
        runner.script("opencli", &["browser", "termyx-quote-read", "close"], "ok");
        runner.script("opencli", &["browser", "termyx-quote-read", "eval"], "https://x.com/i/status/2119069999999999999");
        runner.script("opencli", &["browser", "termyx-quote-read", "eval", "eval(atob(window.__qx))"], quote_read);
        let publisher = OpenCliPublisher::new(&quiet_config(), runner.clone());
        let mut db = ActionDb::default();
        let request = |write| GuiWriteRequest {
            write,
            confirm: Some("yes".into()),
            account: "Jasper_Black".into(),
            char_limit: 280,
        };
        let sent = run_gui_write(
            &publisher,
            &mut db,
            &request(GuiWrite::Post {
                text: "harness post".into(),
            }),
            20,
            1,
        );
        assert!(matches!(sent, GuiWriteOutcome::Sent { .. }), "{sent:?}");
        let reply = run_gui_write(
            &publisher,
            &mut db,
            &request(GuiWrite::Reply {
                target_id: "2118971448614334959".into(),
                text: "harness reply".into(),
            }),
            21,
            2,
        );
        assert!(matches!(reply, GuiWriteOutcome::Sent { .. }), "{reply:?}");
        let quote = run_gui_write(
            &publisher,
            &mut db,
            &request(GuiWrite::Quote {
                target_id: "2118971448614334959".into(),
                text: "harness quote".into(),
            }),
            22,
            3,
        );
        assert!(matches!(quote, GuiWriteOutcome::Sent { .. }), "{quote:?}");
        assert_eq!(
            db.receipt_ids("jasper_black", ReceiptKind::QTweet),
            vec!["2118971448614334959".to_string()]
        );
        let repost = run_gui_write(
            &publisher,
            &mut db,
            &request(GuiWrite::Repost {
                target_id: "2118971448614334959".into(),
            }),
            23,
            4,
        );
        assert!(matches!(repost, GuiWriteOutcome::Sent { .. }), "{repost:?}");
        assert_eq!(
            db.confirmed("Jasper_Black", ToggleKind::Repost, "2118971448614334959"),
            Some(true)
        );
        let calls = runner.calls.lock().unwrap().clone();
        assert!(calls.len() > 4, "{calls:?}");
        assert_eq!(calls[0].1[1], "post");
        assert_eq!(calls[0].1[2], "harness post");
        assert!(!calls[0].1.iter().any(|arg| arg == "reply"));
        assert_eq!(calls[1].1[1], "reply");
        assert!(calls[1].1.iter().any(|arg| arg.contains("2118971448614334959")));
        assert_eq!(calls[1].1[3], "harness reply");
        assert!(calls.iter().any(|(_, args)| {
            args.iter().any(|arg| arg.contains("https://x.com/i/status/2118971448614334959"))
                && args.iter().any(|arg| arg == "open")
        }));
        assert!(calls.iter().any(|(_, args)| args.windows(2).any(|pair| pair[0] == "--name" && pair[1] == "Quote")));
        assert!(!calls.iter().any(|(_, args)| args.get(1).map(String::as_str) == Some("quote")));
        assert!(!calls.iter().any(|(_, args)| {
            args.get(1).map(String::as_str) == Some("post")
                && args.iter().any(|arg| arg.contains("harness quote"))
        }));
        let retweet = calls.iter().find(|(_, args)| args.get(1).map(String::as_str) == Some("retweet"));
        assert!(retweet.is_some(), "{calls:?}");
        assert!(retweet.unwrap().1.iter().any(|arg| arg.contains("2118971448614334959")));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x-action-state.db");
        db.save(&path).unwrap();
        let exported = crate::action_state::export_sanitized(&path).unwrap();
        assert!(exported.contains("opencli-contract"), "{exported}");
        assert!(exported.contains("2118971448614334959"), "{exported}");
        assert!(exported.contains("Posted 42"), "{exported}");
        assert!(exported.contains("q_tweet"), "{exported}");
        assert!(exported.contains("Quoted 2118971448614334959"), "{exported}");
        assert!(exported.contains("Reply sent"), "{exported}");

        let bad = Arc::new(ScriptedRunner::new());
        bad.script("opencli", &["twitter", "retweet"], r#"{"ok":true,"id":"2118971448614334959"}"#);
        bad.script("opencli", &["twitter", "post"], r#"{"ok":true}"#);
        let publisher = OpenCliPublisher::new(&quiet_config(), bad.clone());
        let mut db = ActionDb::default();
        let rejected = run_gui_write(
            &publisher,
            &mut db,
            &request(GuiWrite::Post {
                text: "not a receipt".into(),
            }),
            24,
            5,
        );
        assert!(matches!(rejected, GuiWriteOutcome::Rejected { .. }), "{rejected:?}");
        assert!(db.receipt_ids("jasper_black", ReceiptKind::Post).is_empty());
        let rejected = run_gui_write(
            &publisher,
            &mut db,
            &request(GuiWrite::Repost {
                target_id: "2118971448614334959".into(),
            }),
            25,
            6,
        );
        assert!(matches!(rejected, GuiWriteOutcome::Rejected { .. }), "{rejected:?}");
        assert_ne!(
            db.confirmed("Jasper_Black", ToggleKind::Repost, "2118971448614334959"),
            Some(true)
        );
        assert_eq!(bad.calls.lock().unwrap().len(), 2);
    }

    #[test]
    fn quote_open_failure_keeps_the_opencli_cause() {
        use crate::io::{CommandOutput, CommandRunner};
        struct Runner;
        impl CommandRunner for Runner {
            fn exists(&self, _: &str) -> bool {
                true
            }
            fn run(&self, _: &str, args: &[String]) -> Result<CommandOutput, String> {
                if args.get(1).map(String::as_str) == Some("--help") {
                    return Ok(CommandOutput {
                        status: 0,
                        stdout: "opencli browser open <url> --session <name>\n--window background\n"
                            .into(),
                        stderr: String::new(),
                    });
                }
                Ok(CommandOutput {
                    status: 1,
                    stdout: r#"{"ok":false,"error":{"code":"UNKNOWN_COMMAND","message":"unknown command termyx-quote"}}"#.into(),
                    stderr: String::new(),
                })
            }
        }
        let err = inspect_quote_attachment("opencli", &Runner, "20").unwrap_err();
        assert!(err.contains("stage navigation"), "{err}");
        assert!(err.contains("unknown command termyx-quote"), "{err}");
        assert!(err.contains("--session"), "{err}");
        assert!(err.contains("Post was not clicked"), "{err}");
    }

    #[test]
    fn quote_opens_the_status_menu_and_stops_when_the_card_is_missing() {
        use crate::publish::EngageAction;
        let menu = r#"{"stage":"menu","path":"/i/status/20","selected_label":"Quote","selected_scope":"menu","selected_testid":"-","selected_href_kind":"path=/compose/post keys=none","menu_shape":"A:menuitem:-:Quote:path=/compose/post keys=none","topo_before":"page textareas=1 quotes=0 dialogs=0","clicked_post":false,"card":false,"quote_testids":0,"composer":true,"composer_kind":"unknown","message":"Quote row recorded. Post was not clicked."}"#;
        let missing = r#"{"stage":"composer","path":"/i/status/20","testid_count":12,"composer":true,"composer_kind":"unknown","card":false,"status_ids":[],"quote_testids":0,"clicked_post":false,"clicked_repost_confirm":false,"topo_after":"page textareas=1 quotes=0 dialogs=0","message":"No quote attachment for 20. Post was not clicked."}"#;
        let runner = Arc::new(ScriptedRunner::new());
        runner.script(
            "opencli",
            &["browser", "--help"],
            "Usage: opencli browser <session> <command>\nbrowser <session> open <url>\n--window background\n",
        );
        runner.script(
            "opencli",
            &["browser", "termyx-quote", "open"],
            r#"{"ok":true,"data":{"url":"https://x.com/i/status/20","page":"A03BB6CCAE9402E75FCA7CB1AAED51FA"}}"#,
        );
        runner.script(
            "opencli",
            &["browser", "termyx-quote", "tab", "select"],
            r#"{"ok":true,"data":{"selected":"A03BB6CCAE9402E75FCA7CB1AAED51FA"}}"#,
        );
        runner.script("opencli", &["browser", "termyx-quote", "close"], "ok");
        runner.script("opencli", &["browser", "termyx-quote", "eval"], "https://x.com/i/status/20");
        let vis = crate::publish::quote_visibility_expr("20");
        runner.script(
            "opencli",
            &["browser", "termyx-quote", "eval", vis.as_str()],
            r#"{"ok":true,"data":"{\"v\":\"visible\",\"h\":false,\"p\":false,\"f\":true,\"t\":true,\"m\":true,\"host\":\"x.com\"}"}"#,
        );
        runner.script(
            "opencli",
            &["browser", "termyx-quote", "eval", "eval(atob(window.__qx))"],
            menu,
        );
        runner.script(
            "opencli",
            &["browser", "termyx-quote", "click"],
            r#"{"ok":true,"data":{"clicked":true,"click_method":"cdp"}}"#,
        );
        runner.script(
            "opencli",
            &["browser", "termyx-quote", "eval", "eval(atob(window.__qxSnap))"],
            missing,
        );
        let publisher = OpenCliPublisher::new(&quiet_config(), runner.clone());
        let err = publisher
            .engage(
                EngageAction::Quote,
                "https://x.com/i/status/20",
                Some("my words"),
                &[],
            )
            .unwrap_err();
        assert!(err.contains("Post was not clicked"), "{err}");
        assert!(err.contains("No quote attachment"), "{err}");
        let calls = runner.calls.lock().unwrap().clone();
        assert!(calls.iter().any(|(_, args)| {
            args.iter().any(|arg| arg.contains("https://x.com/i/status/20") && arg.contains("/status/20"))
                && args.iter().any(|arg| arg == "open")
        }));
        assert!(calls.iter().any(|(_, args)| {
            args.windows(2).any(|pair| pair[0] == "--window" && pair[1] == "foreground")
        }));
        assert!(!calls.iter().any(|(_, args)| {
            args.windows(2).any(|pair| pair[0] == "--window" && pair[1] == "background")
        }));
        assert!(calls.iter().any(|(_, args)| {
            args.windows(2).any(|pair| pair[0] == "--name" && pair[1] == "Quote")
        }));
        assert!(calls.iter().any(|(_, args)| {
            args.iter().any(|arg| arg.contains("[data-testid=\"retweet\"]"))
        }));
        assert!(!calls.iter().any(|(_, args)| args.iter().any(|arg| arg.contains("retweetConfirm") || arg.contains("unretweetConfirm"))));
        assert!(!calls.iter().any(|(_, args)| args.iter().any(|arg| arg.contains("qxPost"))));
        assert!(!calls.iter().any(|(_, args)| args.get(1).map(String::as_str) == Some("post")));
        assert!(!calls.iter().any(|(_, args)| args.get(1).map(String::as_str) == Some("quote")));
        assert!(!calls.iter().any(|(_, args)| args.first().map(String::as_str) == Some("browser") && args.get(1).map(String::as_str) == Some("termyx-quote-read")));
        let _ = calls;
    }

    #[test]
    fn quote_write_failure_keeps_the_card_counts_and_cleans_up() {
        use crate::publish::EngageAction;
        let ready = r#"{"stage":"inspect","path":"/cryptoXRPwhale/status/2118971448614334959","composer":true,"composer_kind":"quote","card":true,"quote_testids":2,"quoted_status_id":"2118971448614334959","clicked_post":false,"clicked_repost_confirm":false,"attach":"card-bind article text-equal yes","message":"Quote attachment is visible. Post was not clicked."}"#;
        let missed = r#"{"stage":"composer","path":"/cryptoXRPwhale/status/2118971448614334959","composer":true,"composer_kind":"unknown","card":false,"quote_testids":0,"clicked_post":false,"clicked_repost_confirm":false,"attach":"textareas 2 dialogs 1 | d1 box yes quotes 0 target-links 1 card-bind miss attach-count 1 text-equal no diff-at 16","message":"No quote attachment for 2118971448614334959. Post was not clicked."}"#;
        let runner = Arc::new(ScriptedRunner::new());
        runner.script(
            "opencli",
            &["browser", "--help"],
            "Usage: opencli browser <session> <command>\nbrowser <session> open <url>\n--window background\n",
        );
        runner.script(
            "opencli",
            &["browser", "termyx-quote", "open"],
            r#"{"ok":true,"data":{"url":"https://x.com/i/status/2118971448614334959","page":"A03BB6CCAE9402E75FCA7CB1AAED51FA"}}"#,
        );
        runner.script(
            "opencli",
            &["browser", "termyx-quote", "tab", "select"],
            r#"{"ok":true,"data":{"selected":"A03BB6CCAE9402E75FCA7CB1AAED51FA"}}"#,
        );
        runner.script("opencli", &["browser", "termyx-quote", "close"], "ok");
        runner.script("opencli", &["browser", "termyx-quote", "eval"], "https://x.com/i/status/2118971448614334959");
        let vis = crate::publish::quote_visibility_expr("2118971448614334959");
        runner.script(
            "opencli",
            &["browser", "termyx-quote", "eval", vis.as_str()],
            r#"{"ok":true,"data":"{\"v\":\"visible\",\"h\":false,\"p\":false,\"f\":true,\"t\":true,\"m\":true,\"host\":\"x.com\"}"}"#,
        );
        runner.script(
            "opencli",
            &["browser", "termyx-quote", "eval", "eval(atob(window.__qx))"],
            r#"{"stage":"menu","path":"/cryptoXRPwhale/status/2118971448614334959","selected_label":"Quote","selected_scope":"menu","clicked_post":false,"card":false,"quote_testids":0,"message":"Quote row recorded. Post was not clicked."}"#,
        );
        runner.script(
            "opencli",
            &["browser", "termyx-quote", "click"],
            r#"{"ok":true,"data":{"clicked":true,"click_method":"cdp"}}"#,
        );
        runner.script(
            "opencli",
            &["browser", "termyx-quote", "eval", "eval(atob(window.__qxSnap))"],
            ready,
        );
        runner.script(
            "opencli",
            &["browser", "termyx-quote", "eval", "eval(atob(window.__qxPost))"],
            missed,
        );
        runner.script("opencli", &["browser", "termyx-quote", "screenshot"], "Screenshot saved");
        runner.script(
            "opencli",
            &["browser", "chrome", "call", "tabGroups.query"],
            r#"{"ok":true,"data":[]}"#,
        );
        let publisher = OpenCliPublisher::new(&quiet_config(), runner.clone());
        let err = publisher
            .engage(
                EngageAction::Quote,
                "https://x.com/i/status/2118971448614334959",
                Some("100 is enough"),
                &[],
            )
            .unwrap_err();
        assert!(err.contains("No quote attachment"), "{err}");
        assert!(err.contains("Post was not clicked"), "{err}");
        assert!(err.contains("textareas 2"), "{err}");
        assert!(err.contains("dialogs 1"), "{err}");
        assert!(err.contains("card-bind miss"), "{err}");
        assert!(err.contains("text-equal no"), "{err}");
        assert!(err.contains("diff-at 16"), "{err}");
        assert!(err.contains("quote nodes 0"), "{err}");
        assert!(err.contains("screenshot saved"), "{err}");
        assert!(err.contains("quote-inspect.png"), "{err}");
        assert!(err.contains("session close acknowledged"), "{err}");
        assert!(err.contains("owned group OpenCLI Browser groups 0 tabs 0"), "{err}");
        let calls = runner.calls.lock().unwrap().clone();
        let posts = calls
            .iter()
            .filter(|(_, args)| args.iter().any(|arg| arg == "eval(atob(window.__qxPost))"))
            .count();
        assert_eq!(posts, 1);
        let shot_at = calls.iter().position(|(_, args)| args.iter().any(|arg| arg == "screenshot")).expect("screenshot");
        let close_at = calls.iter().rposition(|(_, args)| args.windows(2).any(|pair| pair[0] == "termyx-quote" && pair[1] == "close")).expect("close");
        assert!(shot_at < close_at, "screenshot must run before session close");
        assert!(!calls.iter().any(|(_, args)| args.get(1).map(String::as_str) == Some("termyx-quote-read")));
        assert!(!calls.iter().any(|(_, args)| args.iter().any(|arg| arg == "windows.update" || arg == "windows.remove" || arg == "tabs.remove")));
    }

    #[test]
    fn quote_compose_route_reaches_the_attachment_check() {
        let missing = r#"{"stage":"composer","path":"/compose/post","composer":true,"composer_kind":"unknown","card":false,"quote_testids":0,"clicked_post":false,"attach":"textareas 2 in-dialog-textareas 0 dialogs 2 page-quotes 0 ancestors DIV:-:- | d1 box no post no quotes 0 target-links 0 testids none | d2 box no post no quotes 0 target-links 0 testids none","message":"No quote attachment for 20. Post was not clicked."}"#;
        let runner = Arc::new(ScriptedRunner::new());
        runner.script(
            "opencli",
            &["browser", "--help"],
            "Usage: opencli browser <session> <command>\nbrowser <session> open <url>\n--window background\n",
        );
        runner.script(
            "opencli",
            &["browser", "termyx-quote", "open"],
            r#"{"ok":true,"data":{"url":"https://x.com/compose/post","page":"B94452EA22F530887AF90E615F8D7031"}}"#,
        );
        runner.script(
            "opencli",
            &["browser", "termyx-quote", "tab", "select"],
            r#"{"ok":true,"data":{"selected":"B94452EA22F530887AF90E615F8D7031"}}"#,
        );
        runner.script("opencli", &["browser", "termyx-quote", "close"], "ok");
        runner.script(
            "opencli",
            &["browser", "termyx-quote", "tab", "list"],
            r#"{"ok":true,"data":[{"page":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"},{"page":"B94452EA22F530887AF90E615F8D7031"}]}"#,
        );
        runner.script("opencli", &["browser", "termyx-quote", "eval"], "https://x.com/compose/post");
        let vis = crate::publish::quote_visibility_expr("20");
        runner.script(
            "opencli",
            &["browser", "termyx-quote", "eval", vis.as_str()],
            r#"{"ok":true,"data":"{\"v\":\"visible\",\"h\":false,\"p\":false,\"f\":false,\"t\":true,\"m\":false,\"c\":true,\"host\":\"x.com\"}"}"#,
        );
        runner.script(
            "opencli",
            &["browser", "termyx-quote", "click"],
            r#"{"ok":true,"data":{"clicked":true,"click_method":"cdp"}}"#,
        );
        runner.script(
            "opencli",
            &["browser", "termyx-quote", "eval", "eval(atob(window.__qxSnap))"],
            missing,
        );
        runner.script(
            "opencli",
            &["browser", "chrome", "call", "tabGroups.query"],
            r#"{"ok":true,"data":[{"id":9,"title":"OpenCLI Browser","windowId":5}]}"#,
        );
        runner.script(
            "opencli",
            &["browser", "chrome", "call", "tabs.query"],
            r#"{"ok":true,"data":[{"id":3,"groupId":9}]}"#,
        );
        runner.script(
            "opencli",
            &["browser", "termyx-quote", "screenshot"],
            "Screenshot saved",
        );
        let err = inspect_quote_attachment("opencli", runner.as_ref(), "20").unwrap_err();
        assert!(err.contains("stage composer"), "{err}");
        assert!(!err.starts_with("stage visibility"), "{err}");
        assert!(err.contains("No quote attachment"), "{err}");
        assert!(err.contains("route compose"), "{err}");
        assert!(err.contains("in-dialog-textareas 0"), "{err}");
        assert!(err.contains("target-links 0"), "{err}");
        assert!(!err.contains("card yes"), "{err}");
        assert!(err.contains("owned group OpenCLI Browser groups 1 tabs 1"), "{err}");
        assert!(err.contains("session list is not chrome clean"), "{err}");
        assert!(err.contains("session close acknowledged"), "{err}");
        assert!(err.contains("open echo url-key no"), "{err}");
        assert!(err.contains("screenshot saved"), "{err}");
        assert!(err.contains("quote-inspect.png"), "{err}");
        let calls = runner.calls.lock().unwrap().clone();
        let encoded = b64_encode(crate::publish::quote_snap_script("20").as_bytes());
        assert!(calls.iter().any(|(_, args)| {
            args.iter().any(|arg| arg.starts_with("window.__qxSnap='") && encoded.starts_with(arg.trim_start_matches("window.__qxSnap='").trim_end_matches('\'')))
        }));
        assert!(calls.iter().any(|(_, args)| args.iter().any(|arg| arg == "eval(atob(window.__qxSnap))")));
        assert!(!calls.iter().any(|(_, args)| args.iter().any(|arg| arg == "windows.update" || arg == "windows.remove" || arg == "tabs.remove")));
        assert!(calls.iter().any(|(_, args)| {
            args.windows(3).any(|triple| {
                triple[0] == "tab" && triple[1] == "close" && triple[2] == "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
            })
        }));
        assert!(!calls.iter().any(|(_, args)| {
            args.windows(3).any(|triple| {
                triple[0] == "tab" && triple[1] == "close" && triple[2] == "B94452EA22F530887AF90E615F8D7031"
            })
        }));
        let shot_at = calls
            .iter()
            .position(|(_, args)| args.iter().any(|arg| arg == "screenshot"))
            .expect("screenshot");
        let close_at = calls
            .iter()
            .rposition(|(_, args)| args.windows(2).any(|pair| pair[0] == "termyx-quote" && pair[1] == "close"))
            .expect("session close");
        assert!(shot_at < close_at, "screenshot must run before session close");
    }

    #[test]
    fn quote_native_click_is_not_sent_when_the_owned_page_stays_hidden() {
        let runner = Arc::new(ScriptedRunner::new());
        runner.script(
            "opencli",
            &["browser", "--help"],
            "Usage: opencli browser open <url> --session <name>\n--window <mode>\n",
        );
        runner.script(
            "opencli",
            &["browser", "tab", "list"],
            r#"{"ok":true,"data":[]}"#,
        );
        runner.script(
            "opencli",
            &["browser", "open"],
            r#"{"ok":true,"data":{"url":"https://x.com/cryptoXRPwhale/status/2118971448614334959","page":"A03BB6CCAE9402E75FCA7CB1AAED51FA"}}"#,
        );
        runner.script("opencli", &["browser", "tab", "select"], "ok");
        runner.script(
            "opencli",
            &["browser", "eval", "location.href"],
            "https://x.com/cryptoXRPwhale/status/2118971448614334959",
        );
        let vis = crate::publish::quote_visibility_expr("2118971448614334959");
        runner.script(
            "opencli",
            &["browser", "eval", vis.as_str()],
            r#"{"ok":true,"data":"{\"v\":\"hidden\",\"h\":true,\"p\":false,\"f\":true,\"t\":true,\"m\":true,\"host\":\"x.com\"}"}"#,
        );
        runner.script(
            "opencli",
            &["browser", "chrome", "call", "tabs.query"],
            r#"{"ok":true,"data":[{"id":3,"windowId":5,"groupId":9,"active":true,"url":"https://x.com/cryptoXRPwhale/status/2118971448614334959"},{"id":4,"windowId":8,"groupId":-1,"active":true,"url":"https://example.com/unrelated"}]}"#,
        );
        runner.script(
            "opencli",
            &["browser", "chrome", "call", "tabGroups.get"],
            r#"{"ok":true,"data":{"id":9,"title":"OpenCLI Browser","windowId":5}}"#,
        );
        runner.script(
            "opencli",
            &["browser", "chrome", "call", "windows.get"],
            r#"{"ok":true,"data":{"id":5,"state":"minimized","focused":false,"type":"normal","left":120,"top":80,"width":1280,"height":900}}"#,
        );
        runner.script("opencli", &["browser", "close"], "ok");
        let err = inspect_quote_attachment(
            "opencli",
            runner.as_ref(),
            "2118971448614334959",
        )
        .unwrap_err();
        assert!(err.contains("stage visibility"), "{err}");
        assert!(err.contains("window foreground"), "{err}");
        assert!(err.contains("tabs before 0"), "{err}");
        assert!(err.contains("field page"), "{err}");
        assert!(err.contains("A03BB6CCAE9402E75FCA7CB1AAED51FA"), "{err}");
        assert!(err.contains("visibility hidden"), "{err}");
        assert!(err.contains("path yes"), "{err}");
        assert!(err.contains("top yes"), "{err}");
        assert!(err.contains("focus yes"), "{err}");
        assert!(err.contains("bounds onscreen"), "{err}");
        assert!(err.contains("group OpenCLI Browser"), "{err}");
        assert!(err.contains("window state minimized"), "{err}");
        assert!(err.contains("focused no"), "{err}");
        assert!(err.contains("windows.update not called"), "{err}");
        assert!(err.contains("ok selected absent"), "{err}");
        assert!(err.contains("native click was not sent"), "{err}");
        assert!(err.contains("Post was not clicked"), "{err}");
        let calls = runner.calls.lock().unwrap().clone();
        assert!(calls.iter().any(|(_, args)| {
            args.windows(2).any(|pair| pair[0] == "--window" && pair[1] == "foreground")
                && args.iter().any(|arg| arg == "open")
        }));
        assert!(calls.iter().any(|(_, args)| args.iter().any(|arg| arg == "select")));
        assert!(calls.iter().any(|(_, args)| {
            args.windows(2).any(|pair| pair[0] == "--tab" && pair[1] == "A03BB6CCAE9402E75FCA7CB1AAED51FA")
                && args.iter().any(|arg| arg.contains("document.visibilityState"))
        }));
        assert!(!calls.iter().any(|(_, args)| args.iter().any(|arg| arg == "click")));
        assert!(!calls.iter().any(|(_, args)| args.iter().any(|arg| arg == "windows.update")));
        assert!(!calls.iter().any(|(_, args)| {
            args.windows(2).any(|pair| pair[0] == "--window" && pair[1] == "background")
        }));
    }

    #[test]
    fn quote_open_without_a_page_id_does_not_read_visibility() {
        let runner = Arc::new(ScriptedRunner::new());
        runner.script(
            "opencli",
            &["browser", "--help"],
            "Usage: opencli browser open <url> --session <name>\n--window <mode>\n",
        );
        runner.script("opencli", &["browser", "tab", "list"], r#"{"ok":true,"data":[]}"#);
        runner.script(
            "opencli",
            &["browser", "open"],
            r#"{"ok":true,"data":{"url":"https://x.com/i/status/2118971448614334959"}}"#,
        );
        runner.script("opencli", &["browser", "close"], "ok");
        let err = inspect_quote_attachment("opencli", runner.as_ref(), "2118971448614334959").unwrap_err();
        assert!(err.contains("stage open"), "{err}");
        assert!(err.contains("target none"), "{err}");
        assert!(err.contains("target field absent"), "{err}");
        assert!(err.contains("page absent"), "{err}");
        assert!(err.contains("visibility open not-read"), "{err}");
        assert!(err.contains("native click was not sent"), "{err}");
        let calls = runner.calls.lock().unwrap().clone();
        assert!(!calls.iter().any(|(_, args)| args.iter().any(|arg| arg.contains("document.visibilityState"))));
        assert!(!calls.iter().any(|(_, args)| args.iter().any(|arg| arg == "click")));
        assert!(!calls.iter().any(|(_, args)| args.iter().any(|arg| arg == "select")));
    }

    #[test]
    fn opencli_output_contract_rejects_exit0_failure_envelopes() {
        let out = |status: i32, stdout: &str, stderr: &str| CommandOutput {
            status,
            stdout: stdout.into(),
            stderr: stderr.into(),
        };
        // Real success envelope passes.
        assert!(
            check_opencli_output(
                &out(0, r#"[{"status":"success","message":"posted"}]"#, ""),
                "test"
            )
            .is_ok()
        );
        // Exit 0 but ok:false (Jeremy's quote failure shape): reject,
        // humanized, with no raw JSON leaking into the message.
        let err = check_opencli_output(
            &out(
                0,
                r#"{"ok":false,"error":{"code":"command_exec","message":"Quote target did not render in the composer."}}"#,
                "",
            ),
            "test",
        )
        .unwrap_err();
        assert_eq!(err, "X didn't attach the quoted tweet.");
        // Non-zero exit with a JSON dump on stderr: same treatment.
        let err = check_opencli_output(
            &out(1, "", r#"{"ok":false,"error":{"message":"boom"}}"#),
            "test",
        )
        .unwrap_err();
        assert!(!err.contains("ok:false") && !err.contains('{'));
        // A status:"error" row is a failure too.
        assert!(check_opencli_output(&out(0, r#"[{"status":"error"}]"#, ""), "test").is_err());
    }

    #[test]
    fn follow_unfollow_verbs_args_and_failure_envelope() {
        use std::sync::Mutex as StdMutex;
        struct RecRunner {
            calls: StdMutex<Vec<Vec<String>>>,
            fail: bool,
        }
        impl crate::io::CommandRunner for RecRunner {
            fn exists(&self, _program: &str) -> bool {
                true
            }
            fn run(&self, _program: &str, args: &[String]) -> Result<CommandOutput, String> {
                self.calls.lock().unwrap().push(args.to_vec());
                if self.fail {
                    return Ok(CommandOutput {
                        status: 0,
                        stdout: r#"{"ok":false,"error":{"code":"command_exec","message":"User not found."}}"#.into(),
                        stderr: String::new(),
                    });
                }
                let verb = args.get(1).map(String::as_str).unwrap_or("");
                let stdout = match verb {
                    "follow" => {
                        r#"[{"status":"success","message":"Successfully followed @jack."}]"#
                    }
                    "unfollow" => {
                        r#"[{"status":"success","message":"Successfully unfollowed @jack."}]"#
                    }
                    _ => r#"[{"status":"success","message":"Tweet successfully liked."}]"#,
                };
                Ok(CommandOutput {
                    status: 0,
                    stdout: stdout.into(),
                    stderr: String::new(),
                })
            }
        }
        let rec = Arc::new(RecRunner {
            calls: StdMutex::new(Vec::new()),
            fail: false,
        });
        let publisher = OpenCliPublisher {
            bin: "opencli".into(),
            runner: rec.clone(),
            post_ok: StdMutex::new(Some(true)),
        };
        // Handle is normalized (@ stripped) and rides the follow verb.
        publisher
            .set_follow("@jack", true)
            .expect("follow should succeed");
        publisher
            .set_follow("jack", false)
            .expect("unfollow should succeed");
        let calls = rec.calls.lock().unwrap();
        assert_eq!(calls[0][1], "follow");
        assert_eq!(calls[0][2], "jack");
        assert_eq!(calls[1][1], "unfollow");
        drop(calls);
        // An ok:false envelope is a failure even with exit 0.
        let failing = OpenCliPublisher {
            bin: "opencli".into(),
            runner: Arc::new(RecRunner {
                calls: StdMutex::new(Vec::new()),
                fail: true,
            }),
            post_ok: StdMutex::new(Some(true)),
        };
        assert!(failing.set_follow("jack", true).is_err());
        assert!(
            failing.set_follow("", true).is_err(),
            "empty handle must not reach the CLI"
        );
    }

    #[test]
    fn viewer_following_reads_only_real_bool_flags() {
        // Boolean flag on the user object (or legacy) is honored.
        let p = parse_profile(r#"{"username":"a","following":true}"#, "a").unwrap();
        assert_eq!(p.viewer_following, Some(true));
        let p = parse_profile(r#"{"username":"a","legacy":{"following":false}}"#, "a").unwrap();
        assert_eq!(p.viewer_following, Some(false));
        // The following COUNT (a number) is not the flag.
        let p = parse_profile(
            r#"{"username":"a","following":120,"followers_count":5}"#,
            "a",
        )
        .unwrap();
        assert_eq!(p.viewer_following, None);
        assert_eq!(p.following, Some(120));
        // No flag anywhere -> unknown, never guessed.
        let p = parse_profile(r#"{"username":"a"}"#, "a").unwrap();
        assert_eq!(p.viewer_following, None);
    }

    #[test]
    fn parse_thread_reads_in_reply_to_and_payload_flags() {
        let json = r#"[{"id":"1","username":"ada","text":"root","likes":3},{"id":"2","username":"bob","text":"reply","likes":1,"in_reply_to_status_id":"1","favorited":true,"user":{"username":"bob","name":"Bob","following":true}}]"#;
        let thread = parse_thread(json, "1").unwrap();
        assert_eq!(thread.root.id, "1");
        assert_eq!(thread.replies.len(), 1);
        assert_eq!(thread.replies[0].in_reply_to.as_deref(), Some("1"));
        assert_eq!(thread.replies[0].viewer_liked, Some(true));
        assert_eq!(thread.replies[0].viewer_follows_author, Some(true));
        let numbered = r#"[{"id":"9","text":"root","username":"ada"},{"id":"10","text":"child","username":"bob","in_reply_to_status_id":9}]"#;
        let numbered = parse_thread(numbered, "9").unwrap();
        assert_eq!(numbered.replies[0].in_reply_to.as_deref(), Some("9"));
    }

    #[test]
    fn mock_thread_returns_nested_replies_under_the_clicked_id() {
        let served = MockProvider.thread("1001").unwrap();
        assert_eq!(served.value.root.id, "1001");
        assert!(
            served
                .value
                .replies
                .iter()
                .any(|post| post.in_reply_to.as_deref() == Some("1001"))
        );
        assert!(
            served
                .value
                .replies
                .iter()
                .any(|post| post.in_reply_to.as_deref() == Some("2001"))
        );
        let tree = crate::thread::build_reply_tree(
            "1001",
            &served.value.replies,
            crate::sort::ResultSort::Popular,
        );
        assert_eq!(tree[0].post.id, "2001");
        assert_eq!(tree[1].post.id, "2006");
        assert_eq!(tree[0].children[0].post.id, "2002");
        let deep = crate::thread::walk_thread(&tree);
        let fifth = deep.iter().find(|node| node.post.id == "2005").unwrap();
        assert_eq!(fifth.depth, 5);
        assert_eq!(fifth.display_depth(), crate::thread::MAX_DISPLAY_DEPTH);
    }

    #[test]
    fn mock_mentions_and_inbox_sort_popular_then_newest() {
        let mentions = MockProvider.mentions("termy", 10).unwrap().value;
        let popular = crate::sorted_posts(&mentions, crate::ResultSort::Popular);
        let newest = crate::sorted_posts(&mentions, crate::ResultSort::Newest);
        assert_eq!(popular[0].id, "3101");
        assert_eq!(newest[0].id, "3103");
        assert!(popular[0].likes > newest[0].likes);
        let inbox = MockProvider.inbox_replies("termy", 10).unwrap().value;
        assert_eq!(
            crate::sorted_posts(&inbox, crate::ResultSort::Popular)[0].id,
            "4101"
        );
        assert_eq!(
            crate::sorted_posts(&inbox, crate::ResultSort::Newest)[0].id,
            "4103"
        );
    }

    #[test]
    fn engage_verbs_match_opencli_help() {
        use crate::publish::EngageAction;
        assert_eq!(EngageAction::Like.opencli_verb(), "like");
        assert_eq!(EngageAction::Unlike.opencli_verb(), "unlike");
        assert_eq!(EngageAction::Retweet.opencli_verb(), "retweet");
        assert_eq!(EngageAction::Unretweet.opencli_verb(), "unretweet");
        assert_eq!(EngageAction::Quote.opencli_verb(), "quote");
    }

    #[test]
    fn dedupe_posts_preserves_order_and_drops_later_dupes() {
        let mk = |id: &str, text: &str| Post {
            id: id.into(),
            author_handle: "ada".into(),
            author_name: "Ada".into(),
            text: text.into(),
            ..Post::default()
        };
        let out = dedupe_posts(vec![
            mk("1", "a"),
            mk("2", "b"),
            mk("1", "a-dupe"),
            mk("", "no-id-1"),
            mk("3", "c"),
            mk("2", "b-dupe"),
            mk("", "no-id-2"),
        ]);
        let ids: Vec<&str> = out.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["1", "2", "", "3", ""]);
        assert_eq!(out[0].text, "a");
    }

    #[test]
    fn opencli_following_feed_passes_type_flag() {
        let runner = Arc::new(ScriptedRunner::new());
        runner.script(
            "opencli",
            &["twitter", "timeline", "--type", "following"],
            r#"[{"id":"7","text":"from following","username":"ada"}]"#,
        );
        let service = Service::free(
            &quiet_config(),
            runner.clone(),
            Arc::new(crate::io::MapHttp::default()),
        );
        let feed = service.timeline_feed(TimelineFeed::Following, 160).unwrap();
        assert_eq!(feed.status.provider, ProviderId::OpenCli);
        assert_eq!(feed.value[0].text, "from following");
        let calls = runner.calls.lock().unwrap().clone();
        assert!(
            calls
                .iter()
                .any(|(_, args)| args.join(" ").contains("--type following")),
            "calls: {calls:?}"
        );
    }

    #[test]
    fn opencli_wins_before_twitter_and_mock() {
        let runner = Arc::new(ScriptedRunner::new());
        runner.script(
            "opencli",
            &["twitter", "timeline"],
            r#"[{"id":"9","text":"from chrome","username":"ada","likes":3}]"#,
        );
        runner.script(
            "twitter",
            &["feed"],
            r#"[{"id":"8","text":"from cookie","screen_name":"bea"}]"#,
        );
        let service = Service::free(
            &quiet_config(),
            runner.clone(),
            Arc::new(crate::io::MapHttp::default()),
        );
        let timeline = service.timeline(10).unwrap();
        assert_eq!(timeline.status.provider, ProviderId::OpenCli);
        assert_eq!(timeline.value[0].text, "from chrome");
        assert!(timeline.status.session.unwrap().contains(SESSION_OWN));
        let calls = runner.calls.lock().unwrap().clone();
        assert!(calls.iter().all(|(program, _)| program == "opencli"));
    }

    #[test]
    fn failed_opencli_falls_through_to_twitter_cli() {
        let runner = Arc::new(ScriptedRunner::new());
        runner.fail("opencli", &["twitter", "search"], "bridge offline");
        runner.script(
            "twitter",
            &["search"],
            r#"[{"id":"4","text":"cookie hit","screenName":"cy"}]"#,
        );
        let service = Service::free(
            &quiet_config(),
            runner,
            Arc::new(crate::io::MapHttp::default()),
        );
        let found = service.search("cookie", true, 5).unwrap();
        assert_eq!(found.status.provider, ProviderId::TwitterCli);
        assert_eq!(found.value[0].author_handle, "cy");
        assert!(
            found
                .status
                .attempts
                .iter()
                .any(|attempt| attempt.provider == ProviderId::OpenCli
                    && attempt.kind == AttemptKind::Failed)
        );
    }

    #[test]
    fn popular_ranks_engagement_and_trends_can_be_derived() {
        let runner = Arc::new(ScriptedRunner::new());
        runner.install("opencli");
        runner.fail("opencli", &["twitter", "trending"], "no trends");
        runner.script(
            "opencli",
            &["twitter", "search"],
            r#"[{"id":"1","text":"quiet #gpui","likes":1},{"id":"2","text":"loud #gpui","likes":50,"reposts":4}]"#,
        );
        let service = Service::free(
            &quiet_config(),
            runner,
            Arc::new(crate::io::MapHttp::default()),
        );
        let popular = service.popular("gpui", 2).unwrap();
        assert_eq!(popular.value[0].id, "2");
        assert!(popular.status.note.contains("engagement"));
        let trends = service.trends(Some("nyc"), 5).unwrap();
        assert_eq!(trends.status.provider, ProviderId::OpenCli);
        assert!(
            trends.status.note.contains("derived")
                || trends
                    .value
                    .iter()
                    .any(|trend| trend.name.contains("gpui") || trend.name.contains("#gpui"))
        );
    }

    #[test]
    fn enabling_the_official_api_puts_it_first() {
        let mut config = quiet_config();
        config.official_enabled = true;
        let service = Service::free(
            &config,
            Arc::new(ScriptedRunner::new()),
            Arc::new(crate::io::MapHttp::default()),
        );
        assert_eq!(service.reader_ids()[0], ProviderId::OfficialApi);
        assert_eq!(service.publisher_ids()[0], ProviderId::OfficialApi);
    }

    #[test]
    fn publish_order_ends_at_the_web_intent() {
        let service = Service::free(
            &quiet_config(),
            Arc::new(ScriptedRunner::new()),
            Arc::new(crate::io::MapHttp::default()),
        );
        assert_eq!(
            service.publisher_ids(),
            vec![ProviderId::OpenCli, ProviderId::TwitterCli]
        );
        let intent = IntentPublisher {
            open_browser: false,
            opener: Arc::new(crate::io::RecordingOpener::default()),
        };
        assert!(intent.available());
        assert_eq!(intent.id(), ProviderId::WebIntent);
    }

    #[test]
    fn publish_skips_adapters_whose_help_has_no_post_command() {
        let runner = Arc::new(ScriptedRunner::new());
        runner.script(
            "opencli",
            &["twitter", "--help"],
            "profile, search, timeline, thread\n",
        );
        runner.script(
            "twitter",
            &["--help"],
            "  search      Search tweets\n  post        Post a new tweet.\n",
        );
        let service = Service::free(
            &quiet_config(),
            runner.clone(),
            Arc::new(crate::io::MapHttp::default()),
        );
        assert!(!service.publishers[0].available());
        assert!(service.publishers[1].available());
        let calls = runner.calls.lock().unwrap().clone();
        assert!(calls.iter().all(|(_, args)| {
            !args
                .iter()
                .any(|arg| arg == "post" || arg == "reply" || arg == "login")
        }));
    }

    #[test]
    fn opencli_column_json_parses_author_strings_and_topics() {
        let runner = Arc::new(ScriptedRunner::new());
        runner.script(
            "opencli",
            &["twitter", "timeline"],
            r#"[{"id":"9","author":"ada","text":"from chrome","likes":3,"retweets":1,"replies":2,"views":10}]"#,
        );
        runner.script(
            "opencli",
            &["twitter", "trending"],
            r##"[{"rank":1,"topic":"#gpui","category":"Tech"}]"##,
        );
        let service = Service::free(
            &quiet_config(),
            runner,
            Arc::new(crate::io::MapHttp::default()),
        );
        let timeline = service.timeline(5).unwrap();
        assert_eq!(timeline.status.provider, ProviderId::OpenCli);
        assert_eq!(timeline.value[0].author_handle, "ada");
        assert_eq!(timeline.value[0].reposts, 1);
        assert_eq!(timeline.value[0].replies, 2);
        let trends = service.trends(None, 5).unwrap();
        assert_eq!(trends.value[0].name, "#gpui");
    }

    #[test]
    fn platform_lanes_parse_their_real_payload_shapes() {
        // TikTok: {desc, author, url, plays, likes} -> caption title + stats.
        let tiktok = r#"[{"rank": 1, "desc": "rust macros explained", "author": "devguy", "url": "https://www.tiktok.com/@devguy/video/1", "plays": 1200, "likes": 90}]"#;
        let hits = parse_tiktok(tiktok);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].source, "tiktok");
        assert_eq!(hits[0].title, "rust macros explained");
        assert!(hits[0].excerpt.contains("@devguy"));
        assert!(hits[0].excerpt.contains("1200 plays"));

        // Facebook: {title, text, url} mixed results.
        let facebook = r#"[{"index": 1, "title": "Rust Devs Group", "text": "A group for rust developers", "url": "https://facebook.com/groups/rust"}]"#;
        let hits = parse_facebook(facebook);
        assert_eq!(hits[0].source, "facebook");
        assert_eq!(hits[0].title, "Rust Devs Group");
        assert_eq!(hits[0].excerpt, "A group for rust developers");

        // Pinterest rows flow through the generic parser with its label.
        let pinterest = r#"[{"pinId": "9", "title": "Terminal setup", "description": "dark desk", "pinner": "amy", "url": "https://pinterest.com/pin/9"}]"#;
        let hits = parse_research("pinterest", pinterest, None);
        assert_eq!(hits[0].source, "pinterest");
        assert_eq!(hits[0].title, "Terminal setup");
        assert_eq!(hits[0].excerpt, "dark desk");
    }

    #[test]
    fn profile_eval_payload_splits_pinned_from_latest() {
        // Normalized rows in the exact shape profile_eval_js returns.
        let json = r##"{
          "posts": [
            {"id": "30", "author": "someone", "text": "newest", "likes": 3},
            {"id": "20", "author": "someone", "text": "the pinned one", "likes": 9}
          ],
          "pinned": {"id": "20", "author": "someone", "text": "the pinned one", "likes": 9}
        }"##;
        let (latest, pinned) = parse_profile_eval(json).unwrap();
        assert_eq!(pinned.len(), 1);
        assert_eq!(pinned[0].id, "20");
        assert_eq!(
            latest.len(),
            2,
            "eval keeps both; dedupe happens at assembly"
        );
        let deduped = crate::model::exclude_pinned(&latest, &pinned);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].id, "30", "pinned renders once, in Pinned only");
    }

    #[test]
    fn retweet_rows_parse_the_embedded_original() {
        // Shape emitted by the GraphQL eval extract for a repost: the
        // outer row is the repost (author = reposter), `retweeted`
        // carries the full original tweet.
        let json = r##"[
          {"id": "50", "author": "reposter", "text": "RT @orig: hello world https://t.co/x", "likes": 1, "retweets": 2,
           "retweeted": {"id": "40", "author": "orig", "text": "hello world", "likes": 99, "retweets": 12, "replies": 3, "views": 5000, "media_urls": ["https://pbs.twimg.com/media/abc.jpg"]}},
          {"id": "51", "author": "plain", "text": "no embed", "likes": 0}
        ]"##;
        let posts = parse_posts(json).unwrap();
        assert_eq!(posts.len(), 2);
        let repost = &posts[0];
        assert_eq!(repost.author_handle, "reposter");
        let inner = repost.retweeted.as_ref().expect("embedded original");
        assert_eq!(inner.id, "40");
        assert_eq!(inner.author_handle, "orig");
        assert_eq!(inner.text, "hello world");
        assert_eq!(inner.likes, 99);
        assert_eq!(inner.media.len(), 1);
        // Actions on a repost target the original; plain posts target themselves.
        assert_eq!(repost.engage_target().id, "40");
        assert_eq!(posts[1].engage_target().id, "51");
        assert!(posts[1].retweeted.is_none());
    }

    #[test]
    fn repost_keeps_outer_id_and_original_media_quote_stays_distinct() {
        let json = r##"[
          {"id":"50","author":"reposter","text":"RT @orig: hello https://t.co/x","likes":1,
           "retweeted":{"id":"40","author":"orig","text":"hello from the original","likes":9,
             "media_urls":["https://video.twimg.com/amplify_video/9/vid/avc1/320x568/a.mp4"],
             "media_posters":["https://pbs.twimg.com/amplify_video_thumb/9/img/p.jpg"]}},
          {"id":"51","author":"other","text":"RT @orig: hello https://t.co/x","likes":2,
           "retweeted":{"id":"40","author":"orig","text":"hello from the original","likes":9,
             "media_urls":["https://pbs.twimg.com/media/still.jpg"]}},
          {"id":"70","author":"quoter","text":"my words on their post","likes":3,
           "quoted":{"id":"40","author":"orig","text":"hello from the original","likes":9,
             "media_urls":["https://pbs.twimg.com/media/q.jpg"]}}
        ]"##;
        let posts = parse_posts(json).unwrap();
        let kept = dedupe_posts(posts);
        assert_eq!(kept.len(), 3, "two reposts of one tweet stay two rows");
        assert_eq!(kept[0].id, "50");
        assert_eq!(kept[0].engage_target().id, "40");
        assert_eq!(kept[0].engage_target().author_handle, "orig");
        assert_eq!(kept[0].card_text(), "hello from the original");
        assert_eq!(kept[0].engage_target().media.len(), 1);
        assert_eq!(kept[1].id, "51");
        assert_eq!(kept[1].engage_target().id, "40");
        assert_ne!(kept[0].id, kept[1].id);
        let quote = &kept[2];
        assert!(quote.retweeted.is_none(), "a quote is not a repost");
        assert_eq!(quote.engage_target().id, "70");
        assert_eq!(quote.card_text(), "my words on their post");
        let quoted = quote.quoted.as_ref().expect("quoted original");
        assert_eq!(quoted.author_handle, "orig");
        assert_eq!(quoted.media.len(), 1);
    }

    #[test]
    fn referenced_original_hydrates_and_deleted_original_stays_a_link() {
        struct ById;
        impl XProvider for ById {
            fn id(&self) -> ProviderId { ProviderId::Mock }
            fn available(&self) -> bool { true }
            fn lookup(&self, _: &str) -> Result<Served<Profile>, String> { Err("stub".into()) }
            fn thread(&self, _: &str) -> Result<Served<Thread>, String> { Err("stub".into()) }
            fn search(&self, _: &str, _: bool, _: usize) -> Result<Served<Vec<Post>>, String> { Err("stub".into()) }
            fn timeline(&self, _: usize) -> Result<Served<Vec<Post>>, String> { Err("stub".into()) }
            fn trends(&self, _: Option<&str>, _: usize) -> Result<Served<Vec<Trend>>, String> { Err("stub".into()) }
            fn post_by_id(&self, id: &str) -> Result<Served<Post>, String> {
                if id == "40" {
                    return Ok(served(
                        post("40", "orig", "Orig", "full original with a photo", 4, 0, 0, 0),
                        "by original id",
                    ));
                }
                Err(format!("unavailable {id}"))
            }
        }
        let service = Service { readers: vec![Box::new(ById)], publishers: Vec::new() };
        let live = post_from_value(
            &serde_json::json!({
                "id": "50",
                "author": "reposter",
                "text": "RT @orig: https://t.co/abc",
                "referenced_status_id": "40"
            }),
            &[],
        ).unwrap();
        assert!(live.retweeted.is_none());
        let mut posts = vec![live.clone()];
        let mut cache = std::collections::HashMap::new();
        assert_eq!(service.resolve_bare_reposts(&mut posts, &mut cache, 4), 1);
        assert_eq!(posts[0].id, "50", "row id stays the repost");
        assert_eq!(posts[0].engage_target().id, "40");
        assert_eq!(posts[0].card_text(), "full original with a photo");
        assert!(cache.contains_key("40") || cache.contains_key("50"));

        let gone = post_from_value(
            &serde_json::json!({
                "id": "80",
                "author": "reposter",
                "text": "RT @gone: https://t.co/zzz",
                "retweeted_status_id_str": "99",
                "retweeted_status_result": {"__typename": "TweetUnavailable", "rest_id": "99"}
            }),
            &[],
        ).unwrap();
        assert!(gone.retweeted.is_none(), "tombstone is not a fabricated tweet");
        assert_eq!(gone.unavailable_original_url.as_deref(), Some("https://x.com/i/status/99"));
        assert_eq!(gone.card_text(), "");
        let mut missing = vec![gone.clone()];
        let mut missing_cache = std::collections::HashMap::new();
        assert_eq!(service.resolve_bare_reposts(&mut missing, &mut missing_cache, 4), 0);
        assert!(missing[0].retweeted.is_none());
        assert_eq!(missing[0].unavailable_original_url.as_deref(), Some("https://x.com/i/status/99"));
        assert!(missing[0].card_text().is_empty());
        let _ = live;
        let _ = gone;
    }

    #[test]
    fn write_timeout_error_tells_user_to_check_before_retrying() {
        let msg = write_timeout_error("command timed out".to_string(), "posting", POST_TIMEOUT);
        assert!(msg.contains("150s"), "{msg}");
        assert!(msg.contains("check X before retrying"), "{msg}");
        // Non-timeout failures pass through untouched.
        assert_eq!(
            write_timeout_error("boom".to_string(), "posting", POST_TIMEOUT),
            "boom"
        );
        // Budgets cover the CLI's own worst case (post: ~13s pre-submit
        // + its 90s submit-confirmation poll) so the CLI's
        // check-before-retry verdict surfaces instead of our kill;
        // anything tighter truncates it, anything near the old 180s
        // blanket reads as an infinite hang.
        assert_eq!(POST_TIMEOUT.as_secs(), 150);
        assert_eq!(ENGAGE_TIMEOUT.as_secs(), 90);
    }

    #[test]
    fn bare_repost_detection_and_card_text() {
        let mut bare = post_from_value(
            &serde_json::json!({"id": "60", "author": "reposter", "text": "RT @orig: hello world"}),
            &[],
        )
        .unwrap();
        assert_eq!(bare.bare_repost_handle(), Some("orig"));
        assert_eq!(bare.card_text(), "hello world");
        // A mention that merely starts a post is not a repost marker.
        bare.text = "RT @orig hello world".into();
        assert_eq!(bare.bare_repost_handle(), None);
        // With the original embedded, the card shows the original text
        // and nothing counts as bare anymore.
        bare.text = "RT @orig: hello world".into();
        let mut original = bare.clone();
        original.id = "59".into();
        original.author_handle = "orig".into();
        original.text = "hello world".into();
        bare.retweeted = Some(Box::new(original));
        assert_eq!(bare.bare_repost_handle(), None);
        assert_eq!(bare.card_text(), "hello world");
        assert_eq!(bare.engage_target().id, "59");
    }

    struct StubById {
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl XProvider for StubById {
        fn id(&self) -> ProviderId {
            ProviderId::Mock
        }
        fn available(&self) -> bool {
            true
        }
        fn lookup(&self, _handle: &str) -> Result<Served<Profile>, String> {
            Err("stub".into())
        }
        fn thread(&self, _id: &str) -> Result<Served<Thread>, String> {
            Err("stub".into())
        }
        fn search(
            &self,
            _query: &str,
            _recent: bool,
            _limit: usize,
        ) -> Result<Served<Vec<Post>>, String> {
            Err("stub".into())
        }
        fn timeline(&self, _limit: usize) -> Result<Served<Vec<Post>>, String> {
            Err("stub".into())
        }
        fn trends(
            &self,
            _place: Option<&str>,
            _limit: usize,
        ) -> Result<Served<Vec<Trend>>, String> {
            Err("stub".into())
        }
        fn post_by_id(&self, id: &str) -> Result<Served<Post>, String> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut outer = post_from_value(
                &serde_json::json!({"id": id, "author": "reposter", "text": "RT @orig: hello"}),
                &[],
            )
            .unwrap();
            let mut original = outer.clone();
            original.id = "59".into();
            original.author_handle = "orig".into();
            original.text = "hello".into();
            outer.retweeted = Some(Box::new(original));
            Ok(served(outer, "stub by-id"))
        }
    }

    #[test]
    fn resolve_bare_reposts_fetches_once_and_caches() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let service = Service {
            readers: vec![Box::new(StubById {
                calls: calls.clone(),
            })],
            publishers: Vec::new(),
        };
        let fetch_count = || calls.load(std::sync::atomic::Ordering::SeqCst);
        let bare = || {
            post_from_value(
                &serde_json::json!({"id": "60", "author": "reposter", "text": "RT @orig: hello"}),
                &[],
            )
            .unwrap()
        };
        let mut posts = vec![bare(), bare()];
        let mut cache = std::collections::HashMap::new();
        // Both copies of the same repost resolve from ONE fetch.
        assert_eq!(service.resolve_bare_reposts(&mut posts, &mut cache, 6), 2);
        assert_eq!(posts[0].engage_target().id, "59");
        assert_eq!(posts[1].engage_target().id, "59");
        assert_eq!(fetch_count(), 1, "same repost id fetched exactly once");
        // Second pass: cache hit, no new fetch.
        let mut again = vec![bare()];
        assert_eq!(service.resolve_bare_reposts(&mut again, &mut cache, 6), 1);
        assert_eq!(fetch_count(), 1, "cache hit must not refetch");
        // Fetch budget respected: with max 0 nothing is fetched or filled.
        let mut fresh = vec![bare()];
        let mut fresh_cache = std::collections::HashMap::new();
        fresh[0].id = "61".into();
        assert_eq!(
            service.resolve_bare_reposts(&mut fresh, &mut fresh_cache, 0),
            0
        );
        assert!(fresh[0].retweeted.is_none());
        assert!(fresh_cache.is_empty());
    }

    #[test]
    fn tweet_by_id_eval_uses_rest_id_and_parses_embed() {
        let js = tweet_by_id_eval_js("12345");
        assert!(js.contains("TweetResultByRestId"), "by-id op");
        assert!(
            js.contains("retweeted_status_result"),
            "by-id unwraps reposts"
        );
        assert!(js.contains("12345"), "id substituted");
        let stdout = r##"{"post": {"id": "60", "author": "reposter", "text": "RT @orig: hi",
          "retweeted": {"id": "59", "author": "orig", "text": "hi", "likes": 7}}}"##;
        let post = parse_tweet_by_id_eval(stdout).unwrap();
        assert_eq!(post.id, "60");
        assert_eq!(post.engage_target().id, "59");
        assert_eq!(post.engage_target().likes, 7);
        let err =
            parse_tweet_by_id_eval(r#"{"error": "tweet not found", "post": null}"#).unwrap_err();
        assert!(err.contains("not found"), "honest error: {err}");
    }

    #[test]
    fn eval_scripts_unwrap_retweeted_status() {
        let timeline = timeline_eval_js(TimelineFeed::ForYou, None, 20);
        assert!(
            timeline.contains("tw.retweeted_status_result"),
            "timeline unwraps reposts from the tweet result (GraphQL carries it there, not on legacy)"
        );
        let profile = profile_eval_js("someone", 30);
        assert!(
            profile.contains("tw.retweeted_status_result"),
            "profile unwraps reposts from the tweet result"
        );
        let tweet = tweet_by_id_eval_js("1");
        for script in [&timeline, &profile, &tweet] {
            assert!(
                script.contains("preview_image_url"),
                "video poster uses the provider still, not only the mp4"
            );
            assert!(
                script.contains("media_url_https, m.media_url, m.preview_image_url"),
                "poster candidates stay the fields the provider actually sends"
            );
        }
    }

    #[test]
    fn profile_eval_error_with_no_posts_is_a_failure() {
        let json = r#"{"error": "profile posts unavailable", "posts": [], "pinned": null}"#;
        assert!(parse_profile_eval(json).is_err());
    }

    #[test]
    fn profile_eval_js_finds_pin_via_dom_and_tweet_result() {
        // Guard the mechanism: pinned comes from the GraphQL pin entry,
        // cross-checked against the user's pinned id list — if either
        // string disappears the feature silently died, so fail loudly.
        let js = profile_eval_js("someone", 30);
        assert!(js.contains("Pinned"), "dom pin marker");
        assert!(js.contains("TweetResultByRestId"), "pinned row source");
        assert!(js.contains("article"), "dom scan");
        assert!(!js.contains("UserByScreenName"), "422-prone op removed");
        assert!(js.contains("\"someone\""), "handle injected");
    }

    #[test]
    fn opencli_trending_payload_parses_rank_topic_category() {
        // Exact shape of `opencli twitter trending -f json` (1.8.8/2.x):
        // rank/topic/category only — X dropped post counts, so volume
        // must come out None, never fabricated.
        let json = r##"[
          {"rank": 1, "topic": "Election Night", "category": "Politics"},
          {"rank": 2, "topic": "#NFLSunday", "category": "Sports"},
          {"rank": 3, "topic": "Local Fest", "category": "Trending in United States"}
         ]"##;
        let trends = parse_trends(json, Some("worldwide")).unwrap();
        assert_eq!(trends.len(), 3);
        assert_eq!(trends[0].name, "Election Night");
        assert_eq!(trends[0].rank, Some(1));
        assert_eq!(trends[0].category.as_deref(), Some("Politics"));
        assert_eq!(trends[0].volume, None);
        assert_eq!(
            trends[2].category.as_deref(),
            Some("Trending in United States")
        );
    }

    #[test]
    fn profile_parses_avatar_and_banner_only_from_payload() {
        let with = parse_profile(
            r#"[{"screen_name":"ada","name":"Ada","profile_image_url_https":"https://img/a.jpg","legacy":{"profile_banner_url":"https://img/b.jpg"}}]"#,
            "ada",
        )
        .unwrap();
        assert_eq!(with.avatar_url.as_deref(), Some("https://img/a.jpg"));
        assert_eq!(with.banner_url.as_deref(), Some("https://img/b.jpg"));
        let without = parse_profile(r#"[{"screen_name":"ada","name":"Ada"}]"#, "ada").unwrap();
        assert_eq!(without.avatar_url, None, "no invented avatar URL");
        assert_eq!(without.banner_url, None);
    }

    #[test]
    fn opencli_array_profile_parses_name_bio_and_counts() {
        let live = parse_profile(
            r#"[{"screen_name":"Jasper_Black","name":"MAGABRO","bio":"links","followers":303,"following":426,"tweets":4391,"likes":998,"verified":true,"created_at":"Mon May 02 03:33:07 +0000 2011"}]"#,
            "Jasper_Black",
        )
        .unwrap();
        assert_eq!(live.followers, Some(303));
        assert_eq!(live.following, Some(426));
        assert_eq!(live.tweets, Some(4391));
        assert_eq!(live.likes, Some(998));
        assert_eq!(live.verified, Some(true));
        assert_eq!(
            live.created_at.as_deref(),
            Some("Mon May 02 03:33:07 +0000 2011")
        );
        let runner = Arc::new(ScriptedRunner::new());
        runner.script(
            "opencli",
            &["twitter", "profile"],
            r#"[{"screen_name":"ada","name":"Ada Lovelace","bio":"first programmer","followers":293,"following":404}]"#,
        );
        let service = Service::free(
            &quiet_config(),
            runner,
            Arc::new(crate::io::MapHttp::default()),
        );
        let profile = service.lookup("ada").unwrap().value;
        assert_eq!(profile.handle, "ada");
        assert_eq!(profile.name, "Ada Lovelace");
        assert_eq!(profile.bio, "first programmer");
        assert_eq!(profile.followers, Some(293));
        assert_eq!(profile.following, Some(404));
        assert_eq!(profile.tweets, None);
        assert_eq!(profile.likes, None);
    }

    #[test]
    fn help_listing_requires_a_command_token() {
        assert!(help_lists_command("post <text> [options]", "post"));
        assert!(help_lists_command("accept, post, profile", "post"));
        assert!(help_lists_command(
            "  post        Post a new tweet.",
            "post"
        ));
        assert!(help_lists_command(
            "lists, login, mute-word, notifications, post, profile, quote, reply,\n  post <text> [options]               [write] Post a new tweet/thread",
            "post"
        ));
        assert!(!help_lists_command(
            "This tool does not post tweets.",
            "post"
        ));
        assert!(!help_lists_command("profile, search, timeline", "post"));
    }

    #[test]
    fn opencli_reply_threads_to_target_with_silent_background_args() {
        // Real reply mechanism (OpenCLI 1.8.8 `twitter reply --help`):
        // `twitter reply <url> <text> [--image <one path>]`. The publish
        // gate must thread to the target id and stay in the background.
        let runner = Arc::new(ScriptedRunner::new());
        runner.script(
            "opencli",
            &["twitter", "reply"],
            r#"[{"status":"success","message":"Reply posted successfully.","url":"https://x.com/ada/status/999"}]"#,
        );
        let config = quiet_config();
        let publisher = OpenCliPublisher::new(&config, runner.clone());
        let outcome = crate::publish::confirm_and_publish_with_media(
            "nice post",
            Some("123"),
            crate::publish::ConfirmDecision::Yes,
            false,
            &publisher,
            280,
            &[],
        );
        assert!(matches!(outcome, crate::publish::GateOutcome::Sent { .. }));
        let calls = runner.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1, "{calls:?}");
        let args = &calls[0].1;
        assert_eq!(args[0], "twitter");
        assert_eq!(args[1], "reply");
        assert_eq!(args[2], "https://x.com/i/status/123", "{args:?}");
        assert_eq!(args[3], "nice post");
        for flag in [
            "--window",
            "background",
            "--site-session",
            "ephemeral",
            "--keep-tab",
            "false",
        ] {
            assert!(args.iter().any(|a| a == flag), "missing {flag}: {args:?}");
        }
        assert!(!args.iter().any(|a| a == "--images"), "{args:?}");
    }

    #[test]
    fn opencli_reply_rejects_two_images_before_calling() {
        let dir = tempfile::tempdir().unwrap();
        let mut media = Vec::new();
        for name in ["a.png", "b.png"] {
            let path = dir.path().join(name);
            std::fs::write(&path, b"png").unwrap();
            media.push(path);
        }
        let runner = Arc::new(ScriptedRunner::new());
        runner.install("opencli");
        let config = quiet_config();
        let publisher = OpenCliPublisher::new(&config, runner.clone());
        let error = publisher
            .publish(&["hi".to_string()], Some("123"), &media)
            .unwrap_err();
        assert!(error.contains("1 image"), "{error}");
        assert!(runner.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn opencli_reply_dry_run_never_calls_the_publisher() {
        let runner = Arc::new(ScriptedRunner::new());
        runner.install("opencli");
        let config = quiet_config();
        let publisher = OpenCliPublisher::new(&config, runner.clone());
        let outcome = crate::publish::confirm_and_publish_with_media(
            "draft reply",
            Some("123"),
            crate::publish::ConfirmDecision::Yes,
            true,
            &publisher,
            280,
            &[],
        );
        assert!(matches!(
            outcome,
            crate::publish::GateOutcome::DryRun { .. }
        ));
        assert!(runner.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn research_query_reaches_the_rss_search_url() {
        // The panel's typed query must flow verbatim (percent-encoded)
        // into the backing searches — no hardcoded topic substitutions.
        let runner = ScriptedRunner::new();
        runner.script(
            "curl",
            &["-fsSL"],
            "<rss><item><title>Quantum dots paper</title><link>https://example.com/q</link></item></rss>",
        );
        let hits = research_topic("quantum dots", &runner, &quiet_config());
        assert!(hits.iter().any(|hit| hit.title == "Quantum dots paper"));
        let calls = runner.calls.lock().unwrap();
        assert!(
            calls.iter().any(|(_bin, args)| args
                .iter()
                .any(|a| a.contains("reddit.com/search.rss?q=quantum%20dots"))),
            "typed query must reach the RSS search URL: {calls:?}"
        );
    }

    #[test]
    fn rss_titles_are_research_hits() {
        let runner = ScriptedRunner::new();
        runner.script(
            "curl",
            &["-fsSL"],
            "<rss><item><title>GPUI notes</title><link>https://example.com/g</link></item></rss>",
        );
        let hits = research_topic("gpui", &runner, &quiet_config());
        assert!(
            hits.iter()
                .any(|hit| hit.source == "rss" && hit.title == "GPUI notes")
        );
    }
}
