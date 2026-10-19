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
    fn search(&self, query: &str, recent: bool, limit: usize) -> Result<Served<Vec<Post>>, String>;
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

/// A page of timeline posts plus the opaque bottom cursor that fetches
/// the next (older) page. `next_cursor: None` means the source is dry.
#[derive(Debug, Clone, PartialEq)]
pub struct TimelinePage {
    pub posts: Vec<Post>,
    pub next_cursor: Option<String>,
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
        if config.provider.is_none() || config.provider == Some(ProviderId::Mock) {
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

    pub fn search(
        &self,
        query: &str,
        recent: bool,
        limit: usize,
    ) -> Result<Served<Vec<Post>>, String> {
        self.read(|reader| reader.search(query, recent, limit))
    }

    pub fn timeline(&self, limit: usize) -> Result<Served<Vec<Post>>, String> {
        self.read(|reader| reader.timeline(limit))
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
                followers: 128,
                following: 40,
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
        let posts = sample_posts();
        let root = posts
            .iter()
            .find(|post| post.id == id)
            .cloned()
            .unwrap_or_else(|| posts[0].clone());
        let replies = posts.into_iter().skip(1).take(2).collect();
        Ok(served(Thread { root, replies }, "fixture thread"))
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
    fn timeline(&self, limit: usize) -> Result<Served<Vec<Post>>, String> {
        let mut posts = sample_posts();
        posts.truncate(limit);
        Ok(served(posts, "fixture timeline"))
    }
    fn timeline_feed(
        &self,
        feed: TimelineFeed,
        limit: usize,
    ) -> Result<Served<Vec<Post>>, String> {
        let mut posts = sample_posts();
        posts.truncate(limit);
        Ok(served(posts, match feed {
            TimelineFeed::ForYou => "fixture timeline (for you)",
            TimelineFeed::Following => "fixture timeline (following)",
        }))
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
    vec![
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
    ]
}

fn fixtures() -> Vec<Profile> {
    vec![Profile {
        handle: "termy".into(),
        name: "Termy".into(),
        bio: "Native terminal. The X panel is a fork feature for Jeremy.".into(),
        followers: 4096,
        following: 120,
        posts: sample_posts().into_iter().take(2).collect(),
        pinned: Vec::new(),
    }]
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
        media: Vec::new(),
        retweeted: None,
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
            &["browser", session, "open", open_url, "--window", "background"],
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
            match self.browser_cmd(
                &["browser", session, "eval", &arg],
                Duration::from_secs(30),
            ) {
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
                "30".into(),
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
        let mut latest = if latest.is_empty() { eval_latest } else { latest };
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
    fn timeline_feed(
        &self,
        feed: TimelineFeed,
        limit: usize,
    ) -> Result<Served<Vec<Post>>, String> {
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
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
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
        let _ = last;
        Ok(format!("{} part(s) posted{media_note}", parts.len()))
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
    /// the same silent-background shim as posts/replies. Quote does
    /// NOT use `twitter quote`: current x.com no longer renders the
    /// quoted card on the compose page that command drives, so it
    /// always aborts (verified live). A quote here is a normal post
    /// whose text ends with the source tweet URL, which X renders as
    /// a native quote embed — same post pipeline as Compose.
    fn engage_opencli(
        &self,
        action: crate::publish::EngageAction,
        target_url: &str,
        text: Option<&str>,
        media: &[PathBuf],
    ) -> Result<String, String> {
        use crate::publish::EngageAction;
        let mut args: Vec<String> = vec![
            "twitter".into(),
            action.opencli_verb().into(),
            target_url.into(),
        ];
        if action == EngageAction::Quote {
            let text = text.unwrap_or_default().trim();
            if text.is_empty() {
                return Err("write your quote first — nothing was posted".into());
            }
            crate::publish::validate_media_paths(media)?;
            if media.len() > 1 {
                return Err(
                    "Quotes can attach 1 image (OpenCLI limit) — remove the extra image first. Nothing was posted."
                        .into(),
                );
            }
            // Post path (see the note above): user's words + the
            // source URL; media rides the post `--images` handling.
            let combined = crate::publish::quote_post_text(text, target_url);
            if combined.is_empty() {
                return Err("write your quote first — nothing was posted".into());
            }
            return self.publish(&[combined], None, media);
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
        check_opencli_output(&output, "opencli engage")
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
            if let Some(id) = tweet_id_from_output(&output.stdout) {
                previous = Some(id);
            }
        }
        Ok(format!("twitter-cli posted {} part(s)", parts.len()))
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
    if let Some(ok) = global.lock().expect("post support global").get(&key).copied() {
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
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
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
    format!(
        r##"(async () => {{
  const FEED = {feed_js};
  const WANT = {count};
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
        let best = null;
        for (const v of (m.video_info && m.video_info.variants) || []) {{
          if (v.content_type === "video/mp4" && v.url && (!best || (v.bitrate || 0) > (best.bitrate || 0))) best = v;
        }}
        urls.push(best ? best.url : m.media_url_https);
        posters.push(m.media_url_https);
        durations.push((m.video_info && m.video_info.duration_millis) || 0);
      }} else {{
        urls.push(m.media_url_https); posters.push(m.media_url_https); durations.push(0);
      }}
    }}
    return {{has_media: urls.length > 0, media_urls: urls, media_posters: posters, media_durations: durations}};
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
    const rt = (l.retweeted_status_result && l.retweeted_status_result.result) || null;
    return {{id: tw.rest_id, author: screenName, author_verified: Boolean((u && u.is_blue_verified) || (u && u.legacy && u.legacy.verified)), text: noteText || l.full_text || "", likes: l.favorite_count || 0, retweets: l.retweet_count || 0, replies: l.reply_count || 0, views: tw.views && tw.views.count ? parseInt(tw.views.count, 10) : 0, created_at: l.created_at || "", url: "https://x.com/" + screenName + "/status/" + tw.rest_id, retweeted: rt && (rt.legacy || rt.tweet) ? extract(rt, new Set()) : null, ...mediaOf(l)}};
  }}
  function parsePage(data, seen) {{
    const tweets = [];
    let next = null;
    const instructions = (data && data.data && data.data.home && data.data.home.home_timeline_urt && data.data.home.home_timeline_urt.instructions) || [];
    for (const inst of instructions) {{
      for (const entry of inst.entries || []) {{
        const c = entry.content || {{}};
        if (c.cursorType === "Bottom" || (entry.entryId || "").startsWith("cursor-bottom-")) {{ if (c.value) next = c.value; continue; }}
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
  for (let page = 0; page < 14 && out.length < WANT; page++) {{
    const vars = {{count: Math.min(40, WANT - out.length + 5), includePromotedContent: false, latestControlAvailable: true, requestContext: "launch"}};
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
    let value: Value = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("timeline session returned unreadable output: {e}"))?;
    let next_cursor = value
        .get("nextCursor")
        .and_then(Value::as_str)
        .map(str::to_string);
    let posts = match value.get("posts") {
        Some(rows) if rows.is_array() => {
            parse_posts(&rows.to_string()).unwrap_or_default()
        }
        _ => Vec::new(),
    };
    if let Some(error) = value.get("error").and_then(Value::as_str)
        && posts.is_empty()
    {
        return Err(if error == "http-429" {
            "X rate-limited the session (HTTP 429) — wait a few minutes and try Load more again".into()
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
        let best = null;
        for (const v of (m.video_info && m.video_info.variants) || []) {
          if (v.content_type === "video/mp4" && v.url && (!best || (v.bitrate || 0) > (best.bitrate || 0))) best = v;
        }
        urls.push(best ? best.url : m.media_url_https);
        posters.push(m.media_url_https);
        durations.push((m.video_info && m.video_info.duration_millis) || 0);
      } else {
        urls.push(m.media_url_https); posters.push(m.media_url_https); durations.push(0);
      }
    }
    return {has_media: urls.length > 0, media_urls: urls, media_posters: posters, media_durations: durations};
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
    const rt = (l.retweeted_status_result && l.retweeted_status_result.result) || null;
    return {id: tw.rest_id, author: screenName, author_verified: Boolean((u && u.is_blue_verified) || (u && u.legacy && u.legacy.verified)), text: noteText || l.full_text || "", likes: l.favorite_count || 0, retweets: l.retweet_count || 0, replies: l.reply_count || 0, views: tw.views && tw.views.count ? parseInt(tw.views.count, 10) : 0, created_at: l.created_at || "", url: "https://x.com/" + screenName + "/status/" + tw.rest_id, retweeted: rt && (rt.legacy || rt.tweet) ? extract(rt, new Set()) : null, ...mediaOf(l)};
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
    let value: Value = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("profile session returned unreadable output: {e}"))?;
    if let Some(diag) = value.get("diag") {
        log_raw_provider_error(
            "profile diag",
            &serde_json::to_string(diag).unwrap_or_else(|_| "?".to_string()),
        );
    }
    let posts = match value.get("posts") {
        Some(rows) if rows.is_array() => parse_posts(&rows.to_string()).unwrap_or_default(),
        _ => Vec::new(),
    };
    let pinned = match value.get("pinned") {
        Some(row) if row.is_object() => {
            parse_posts(&serde_json::json!([row]).to_string()).unwrap_or_default()
        }
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
            "https://api.x.com/2/users/by/username/{handle}?user.fields=description,public_metrics"
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
    fn timeline_feed(
        &self,
        feed: TimelineFeed,
        limit: usize,
    ) -> Result<Served<Vec<Post>>, String> {
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
            })?;
            if let Some(id) = serde_json::from_str::<Value>(&response)
                .ok()
                .and_then(|value| value["data"]["id"].as_str().map(str::to_string))
            {
                ids.push(id);
            }
        }
        Ok(format!("official API posted {} part(s)", parts.len()))
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
            hits.extend(parse_research("pinterest", &output.stdout, Some(SESSION_OWN)));
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
    if hits.is_empty() {
        hits.push(ResearchHit {
            source: "fixture".into(),
            title: format!("What people are saying about {topic}"),
            url: "https://example.com/research".into(),
            excerpt: "No Reddit, YouTube, or web reader was available. This fixture is safe to draft from."
                .into(),
            session: None,
        });
    }
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
    let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let users = users_by_id(&value);
    let posts = array_of(&value)
        .into_iter()
        .filter_map(|item| post_from_value(item, &users))
        .collect::<Vec<_>>();
    if posts.is_empty() {
        Err("no posts in provider response".into())
    } else {
        Ok(posts)
    }
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
    let metrics = node.get("public_metrics").unwrap_or(node);
    let posts = parse_posts(text).unwrap_or_default();
    Ok(Profile {
        handle,
        name,
        bio,
        followers: number(metrics, &["followers_count", "followers"]),
        following: number(metrics, &["following_count", "friends_count", "following"]),
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
    let text = first_str(item, &["text", "full_text", "rawContent", "content"])?.to_string();
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
        text,
        id,
        media: media_from_value(item),
        retweeted: first_embedded(item, &["retweeted", "retweeted_status", "retweeted_tweet"])
            .and_then(|inner| post_from_value(inner, &[]))
            .map(Box::new),
    })
}

/// First embedded-tweet object among `keys` (GraphQL eval rows emit
/// `retweeted`; some API-shaped payloads use the other names).
fn first_embedded<'a>(item: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .filter_map(|key| item.get(*key))
        .find(|value| value.is_object())
}

/// Media from provider payloads. Verified live against OpenCLI
/// `twitter timeline -f json` (2026-10-18): rows carry `media_urls`
/// (photo JPGs on pbs.twimg.com, mp4s on video.twimg.com) with parallel
/// `media_posters` thumbnails. Also accepts official-API style
/// `includes.media` / `attachments` and simple `media`/`images` arrays.
fn media_from_value(item: &Value) -> Vec<crate::model::MediaItem> {
    use crate::model::{MediaItem, MediaKind};
    let kind_of = |url: &str| -> MediaKind {
        let lower = url.to_ascii_lowercase();
        if lower.contains("/tweet_video/") || lower.ends_with(".gif") {
            MediaKind::Gif
        } else if lower.contains("video.twimg.com")
            || lower.contains(".mp4")
            || lower.contains(".m3u8")
        {
            MediaKind::Video
        } else {
            MediaKind::Photo
        }
    };
    let mut out: Vec<MediaItem> = Vec::new();
    if let Some(urls) = item.get("media_urls").and_then(Value::as_array) {
        let posters = item.get("media_posters").and_then(Value::as_array);
        for (i, url) in urls.iter().filter_map(Value::as_str).enumerate() {
            if url.is_empty() { continue; }
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
            out.push(MediaItem { kind, url: url.to_string(), poster, width, height });
        }
        return out;
    }
    if let Some(list) = item.get("media").and_then(Value::as_array)
        .or_else(|| item.get("images").and_then(Value::as_array))
    {
        for entry in list {
            let (url, poster) = match entry {
                Value::String(u) => (u.clone(), None),
                Value::Object(_) => (
                    first_str(entry, &["url", "media_url", "media_url_https", "preview_image_url"]).unwrap_or("").to_string(),
                    first_str(entry, &["preview_image_url", "poster", "thumbnail"]).map(str::to_string),
                ),
                _ => (String::new(), None),
            };
            if url.is_empty() { continue; }
            let dims = crate::model::dims_from_media_url(&url);
            let (width, height) = match (
                entry.get("width").and_then(Value::as_u64),
                entry.get("height").and_then(Value::as_u64),
            ) {
                (Some(w), Some(h)) => (Some(w as u32), Some(h as u32)),
                _ => match dims { Some((w, h)) => (Some(w), Some(h)), None => (None, None) },
            };
            let kind = match first_str(entry, &["type", "kind"]) {
                Some("video") | Some("animated_gif") => if first_str(entry, &["type"]) == Some("animated_gif") { MediaKind::Gif } else { MediaKind::Video },
                _ => kind_of(&url),
            };
            let poster = poster.or_else(|| (kind == MediaKind::Photo).then(|| url.clone()));
            out.push(MediaItem { kind, url, poster, width, height });
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
        assert_eq!(posts[0].media[0].poster.as_deref(), Some("https://pbs.twimg.com/amplify_video_thumb/9/img/p.jpg"));
        assert_eq!(posts[1].media[0].kind, MediaKind::Photo);
        assert_eq!(posts[1].media[0].poster.as_deref(), Some("https://pbs.twimg.com/media/ABC.jpg"));
        assert!(posts[2].media.is_empty());
        // Video URLs state their size (/480x270/ style segment, here 320x568 in fixture? use parsed below)
        assert_eq!(posts[0].media[0].width, Some(320));
        assert_eq!(posts[0].media[0].height, Some(568));
        assert_eq!(posts[1].media[0].width, None);
    }

    #[test]
    fn dims_from_media_url_parses_only_plausible_segments() {
        use crate::model::dims_from_media_url;
        assert_eq!(dims_from_media_url("https://video.twimg.com/amplify_video/9/vid/avc1/480x270/a.mp4?tag=29"), Some((480, 270)));
        assert_eq!(dims_from_media_url("https://pbs.twimg.com/media/ABC.jpg"), None);
        assert_eq!(dims_from_media_url("https://pbs.twimg.com/media/ABC.jpg?name=4096x4096"), None);
        assert_eq!(dims_from_media_url("https://x.com/1x1/a.jpg"), None);
    }
}

fn first_str<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
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
    fn default_readers_skip_the_official_api_and_land_on_mock() {
        let runner = Arc::new(ScriptedRunner::new());
        let http = Arc::new(crate::io::MapHttp::default());
        let service = Service::free(&quiet_config(), runner, http);
        assert_eq!(
            service.reader_ids(),
            vec![
                ProviderId::OpenCli,
                ProviderId::TwitterCli,
                ProviderId::Mock
            ]
        );
        assert!(!service.reader_ids().contains(&ProviderId::OfficialApi));
        let timeline = service.timeline(2).unwrap();
        assert_eq!(timeline.status.provider, ProviderId::Mock);
        assert!(timeline.status.render().contains("no X API credits"));
        assert_eq!(timeline.value.len(), 2);
        assert!(
            timeline
                .status
                .attempts
                .iter()
                .any(|attempt| attempt.provider == ProviderId::OpenCli
                    && attempt.kind == AttemptKind::Skipped)
        );
    }

    #[test]
    fn append_new_posts_only_appends_unseen_ids() {
        let mk = |id: &str| Post {
            id: id.into(),
            url: String::new(),
            author_handle: "a".into(),
            author_name: "a".into(),
            text: "t".into(),
            created_at: None,
            likes: 0,
            reposts: 0,
            replies: 0,
            bookmarks: 0,
            views: 0,
            media: Vec::new(),
            retweeted: None,
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
        assert!(parse_timeline_eval(r#"{"error":"http-429","posts":[],"nextCursor":null}"#)
            .unwrap_err()
            .contains("429"));
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
    fn quote_engages_via_post_path_with_source_url() {
        use crate::publish::EngageAction;
        use std::sync::Mutex as StdMutex;
        struct RecRunner {
            calls: StdMutex<Vec<Vec<String>>>,
            fail: bool,
        }
        impl CommandRunner for RecRunner {
            fn exists(&self, _program: &str) -> bool {
                true
            }
            fn run(&self, _program: &str, args: &[String]) -> Result<CommandOutput, String> {
                self.calls.lock().unwrap().push(args.to_vec());
                if self.fail {
                    return Ok(CommandOutput {
                        status: 0,
                        stdout: r#"{"ok":false,"error":{"code":"command_exec","message":"Quote target did not render in the composer."}}"#.into(),
                        stderr: String::new(),
                    });
                }
                Ok(CommandOutput {
                    status: 0,
                    stdout: r#"[{"status":"success","id":"9","url":"https://x.com/me/status/9"}]"#.into(),
                    stderr: String::new(),
                })
            }
        }
        let rec = Arc::new(RecRunner { calls: StdMutex::new(Vec::new()), fail: false });
        let publisher = OpenCliPublisher {
            bin: "opencli".into(),
            runner: rec.clone(),
            post_ok: StdMutex::new(Some(true)),
        };
        let url = "https://x.com/jack/status/20";
        publisher
            .engage(EngageAction::Quote, url, Some("my words"), &[])
            .expect("quote via post path should succeed");
        let calls = rec.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let args = &calls[0];
        assert_eq!(args[1], "post", "quote must ride the post command: {args:?}");
        assert!(!args.iter().any(|a| a == "quote"), "old quote verb must not be used: {args:?}");
        assert_eq!(args[2], format!("my words\n{url}"));
        drop(calls);
        // And a failure envelope on a like comes back humanized.
        let rec2 = Arc::new(RecRunner { calls: StdMutex::new(Vec::new()), fail: true });
        let publisher2 = OpenCliPublisher {
            bin: "opencli".into(),
            runner: rec2,
            post_ok: StdMutex::new(Some(true)),
        };
        let err = publisher2
            .engage(EngageAction::Like, url, None, &[])
            .unwrap_err();
        assert_eq!(err, "X didn't attach the quoted tweet.");
    }

    #[test]
    fn opencli_output_contract_rejects_exit0_failure_envelopes() {
        let out = |status: i32, stdout: &str, stderr: &str| CommandOutput {
            status,
            stdout: stdout.into(),
            stderr: stderr.into(),
        };
        // Real success envelope passes.
        assert!(check_opencli_output(
            &out(0, r#"[{"status":"success","message":"posted"}]"#, ""),
            "test"
        )
        .is_ok());
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
        let err = check_opencli_output(&out(1, "", r#"{"ok":false,"error":{"message":"boom"}}"#), "test")
            .unwrap_err();
        assert!(!err.contains("ok:false") && !err.contains('{'));
        // A status:"error" row is a failure too.
        assert!(check_opencli_output(&out(0, r#"[{"status":"error"}]"#, ""), "test").is_err());
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

    fn dedupe_posts_preserves_order_and_drops_later_dupes() {
        let mk = |id: &str, text: &str| Post {
            id: id.into(),
            url: String::new(),
            author_handle: "ada".into(),
            author_name: "Ada".into(),
            text: text.into(),
            created_at: None,
            likes: 0,
            reposts: 0,
            replies: 0,
            bookmarks: 0,
            views: 0,
            media: Vec::new(),
            retweeted: None,
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
            calls.iter().any(|(_, args)| args.join(" ").contains("--type following")),
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
        assert_eq!(latest.len(), 2, "eval keeps both; dedupe happens at assembly");
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
    fn write_timeout_error_tells_user_to_check_before_retrying() {
        let msg = write_timeout_error(
            "command timed out".to_string(),
            "posting",
            POST_TIMEOUT,
        );
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
    fn eval_scripts_unwrap_retweeted_status() {
        let timeline = timeline_eval_js(TimelineFeed::ForYou, None, 20);
        assert!(timeline.contains("retweeted_status_result"), "timeline unwraps reposts");
        let profile = profile_eval_js("someone", 30);
        assert!(profile.contains("retweeted_status_result"), "profile unwraps reposts");
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
        assert_eq!(trends[2].category.as_deref(), Some("Trending in United States"));
    }

    #[test]
    fn opencli_array_profile_parses_name_bio_and_counts() {
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
        assert_eq!(profile.followers, 293);
        assert_eq!(profile.following, 404);
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
            r#"[{"status":"ok","url":"https://x.com/ada/status/999"}]"#,
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
