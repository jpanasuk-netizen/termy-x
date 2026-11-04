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
}

impl Post {
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
    pub posts: Vec<Post>,
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
            }
        })
        .collect()
}
