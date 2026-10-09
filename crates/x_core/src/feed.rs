use crate::model::{Post, StatusLine};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RankMode {
    Hot,
    Upcoming,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedEffect {
    Stay,
    Search { topic: String, recent: bool },
    Post { text: String, reply_to: String },
    Quit,
}

#[derive(Debug, Clone)]
pub struct FeedOpts {
    pub topic: Option<String>,
    pub mode: RankMode,
    pub limit: usize,
    pub keys: Option<String>,
    pub dry_run: bool,
}

#[derive(Debug, Clone)]
pub struct Feed {
    posts: Vec<Post>,
    cursor: usize,
    mode: RankMode,
    topic: Option<String>,
    reply_open: bool,
    draft: String,
    previewed: bool,
    status: String,
    quota: String,
    notice: String,
}

impl Feed {
    pub fn new(
        posts: Vec<Post>,
        status: &StatusLine,
        mode: RankMode,
        topic: Option<String>,
    ) -> Self {
        let mut feed = Self {
            posts,
            cursor: 0,
            mode,
            topic,
            reply_open: false,
            draft: String::new(),
            previewed: false,
            status: status.render(),
            quota: quota_label(status),
            notice: String::new(),
        };
        feed.rank();
        feed
    }

    pub fn replace(&mut self, posts: Vec<Post>, status: &StatusLine, topic: Option<String>) {
        self.posts = posts;
        self.topic = topic;
        self.cursor = 0;
        self.reply_open = false;
        self.draft.clear();
        self.previewed = false;
        self.status = status.render();
        self.quota = quota_label(status);
        self.rank();
    }

    pub fn apply(&mut self, raw: &str) -> FeedEffect {
        let key = raw.trim();
        if key.is_empty() {
            return FeedEffect::Stay;
        }
        if self.reply_open {
            return self.apply_reply(key);
        }
        if let Some(topic) = key.strip_prefix('/') {
            let topic = topic.trim();
            if topic.is_empty() {
                self.notice = "type /subject to search".into();
                return FeedEffect::Stay;
            }
            self.topic = Some(topic.to_string());
            self.notice = format!("searching {topic}");
            return FeedEffect::Search {
                topic: topic.to_string(),
                recent: self.mode == RankMode::Upcoming,
            };
        }
        match key {
            "j" | "down" => self.move_cursor(1),
            "k" | "up" => self.move_cursor(-1),
            "h" | "hot" => {
                self.mode = RankMode::Hot;
                self.rank();
                self.notice = "hot: engagement, then newer".into();
            }
            "u" | "upcoming" => {
                self.mode = RankMode::Upcoming;
                self.rank();
                self.notice = "upcoming: newer, then engagement".into();
            }
            "r" => {
                if self.posts.is_empty() {
                    self.notice = "no tweet selected".into();
                } else {
                    self.reply_open = true;
                    self.draft.clear();
                    self.previewed = false;
                    self.notice = "reply box open. type a line, then p, then y".into();
                }
            }
            "q" | "quit" => return FeedEffect::Quit,
            _ => self.notice = format!("unknown key `{key}`"),
        }
        FeedEffect::Stay
    }

    fn apply_reply(&mut self, key: &str) -> FeedEffect {
        match key {
            "p" => {
                if self.draft.trim().is_empty() {
                    self.notice = "reply is empty".into();
                    self.previewed = false;
                } else {
                    self.previewed = true;
                    self.notice = "preview only. y posts. esc cancels.".into();
                }
                FeedEffect::Stay
            }
            "y" => {
                if !self.previewed {
                    self.notice = "press p to preview before y".into();
                    return FeedEffect::Stay;
                }
                let Some(text) = self.ready_text() else {
                    self.notice = "reply is empty".into();
                    return FeedEffect::Stay;
                };
                let Some(reply_to) = self.selected_id() else {
                    self.notice = "no tweet selected".into();
                    return FeedEffect::Stay;
                };
                self.reply_open = false;
                self.previewed = false;
                self.draft.clear();
                self.notice = "confirm accepted".into();
                FeedEffect::Post { text, reply_to }
            }
            "esc" => {
                self.reply_open = false;
                self.previewed = false;
                self.draft.clear();
                self.notice = "reply cancelled. nothing posted".into();
                FeedEffect::Stay
            }
            "backspace" => {
                self.draft.pop();
                self.previewed = false;
                FeedEffect::Stay
            }
            other => {
                let text = other.strip_prefix("type:").unwrap_or(other);
                self.draft = text.to_string();
                self.previewed = false;
                self.notice = "draft updated. p previews. nothing posted.".into();
                FeedEffect::Stay
            }
        }
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        let mode = match self.mode {
            RankMode::Hot => "hot",
            RankMode::Upcoming => "upcoming",
        };
        let subject = self.topic.as_deref().unwrap_or("home timeline");
        out.push_str(&format!(
            "{status} | {mode} | {subject} | {quota}\n",
            status = self.status,
            quota = self.quota
        ));
        if self.posts.is_empty() {
            out.push_str("(no tweets)\n");
        }
        for (index, post) in self.posts.iter().enumerate() {
            let mark = if index == self.cursor { '>' } else { ' ' };
            let when = post.created_at.as_deref().unwrap_or("-");
            let text = one_line(&post.text);
            out.push_str(&format!(
                "{mark}{num} @{handle}  {eng}  {when}  {text}\n",
                num = index + 1,
                handle = post.author_handle,
                eng = post.engagement(),
            ));
        }
        if self.reply_open {
            let target = self.selected().map_or_else(
                || "(none)".into(),
                |post| format!("@{} {}", post.author_handle, post.url),
            );
            out.push_str(&format!("reply to {target}\n"));
            out.push_str(&format!("draft: {}\n", self.draft));
            if self.previewed {
                let count = crate::text::weighted_len(&self.draft).weighted;
                out.push_str(&format!(
                    "preview ({count} weighted chars). not posted.\n{draft}\n",
                    draft = self.draft
                ));
            }
        }
        if !self.notice.is_empty() {
            out.push_str(&format!("{}\n", self.notice));
        }
        out.push_str(
            "keys: j/k move | h hot | u upcoming | /subject search | r reply | p preview | y post | esc cancel | q quit\n",
        );
        out
    }

    fn rank(&mut self) {
        let mode = self.mode;
        self.posts.sort_by(|left, right| match mode {
            RankMode::Hot => right
                .engagement()
                .cmp(&left.engagement())
                .then(recency_key(right).cmp(recency_key(left)))
                .then(left.id.cmp(&right.id)),
            RankMode::Upcoming => recency_key(right)
                .cmp(recency_key(left))
                .then(right.engagement().cmp(&left.engagement()))
                .then(left.id.cmp(&right.id)),
        });
        if self.cursor >= self.posts.len() {
            self.cursor = self.posts.len().saturating_sub(1);
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        if self.posts.is_empty() {
            return;
        }
        let next = self.cursor as isize + delta;
        let max = self.posts.len() as isize - 1;
        self.cursor = next.clamp(0, max) as usize;
    }

    fn selected(&self) -> Option<&Post> {
        self.posts.get(self.cursor)
    }

    fn selected_id(&self) -> Option<String> {
        self.selected().map(|post| post.id.clone())
    }

    fn ready_text(&self) -> Option<String> {
        let text = self.draft.trim();
        if text.is_empty() {
            None
        } else {
            Some(text.to_string())
        }
    }
}

pub fn split_keys(spec: &str) -> Vec<String> {
    spec.split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

pub fn quota_label(status: &StatusLine) -> String {
    let blob = format!(
        "{} {} {}",
        status.note,
        status.credits,
        status
            .attempts
            .iter()
            .map(|attempt| attempt.detail.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    );
    if let Some(count) = remaining_quota(&blob) {
        format!("quota: {count} remaining")
    } else {
        "quota: not reported".into()
    }
}

fn remaining_quota(blob: &str) -> Option<u64> {
    let lower = blob.to_ascii_lowercase();
    for needle in ["remaining=", "remaining:", "rate_limit_remaining "] {
        if let Some(index) = lower.find(needle) {
            let rest = lower[index + needle.len()..].trim_start();
            let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
            if let Ok(count) = digits.parse::<u64>() {
                return Some(count);
            }
        }
    }
    None
}

fn recency_key(post: &Post) -> &str {
    post.created_at.as_deref().unwrap_or("")
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn replay(feed: &mut Feed, keys: &[String]) -> Vec<FeedEffect> {
    keys.iter().map(|key| feed.apply(key)).collect()
}

pub struct RunInput {
    pub topic: Option<String>,
    pub mode: RankMode,
    pub limit: usize,
    pub keys: Option<String>,
    pub dry_run: bool,
    pub interactive: bool,
}

enum Step {
    Continue,
    Quit,
    Failed,
}

pub fn run<L, P>(input: RunInput, mut load: L, mut post: P) -> i32
where
    L: FnMut(Option<&str>, bool, usize) -> Result<(Vec<Post>, StatusLine), String>,
    P: FnMut(&str, &str, bool) -> i32,
{
    let recent = input.mode == RankMode::Upcoming;
    let (posts, status) = match load(input.topic.as_deref(), recent, input.limit) {
        Ok(loaded) => loaded,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    let mut feed = Feed::new(posts, &status, input.mode, input.topic);
    println!("{}", feed.render());
    if let Some(spec) = &input.keys {
        let keys = split_keys(spec);
        let code = play(
            &mut feed,
            &keys,
            &mut load,
            &mut post,
            input.limit,
            input.dry_run,
        );
        println!("{}", feed.render());
        return code;
    }
    if !input.interactive {
        return 0;
    }
    loop {
        print!("> ");
        let _ = std::io::Write::flush(&mut std::io::stdout());
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() || line.is_empty() {
            return 0;
        }
        match step(
            &mut feed,
            line.trim(),
            &mut load,
            &mut post,
            input.limit,
            input.dry_run,
        ) {
            Step::Continue => println!("{}", feed.render()),
            Step::Quit => return 0,
            Step::Failed => return 1,
        }
    }
}

fn play<L, P>(
    feed: &mut Feed,
    keys: &[String],
    load: &mut L,
    post: &mut P,
    limit: usize,
    dry_run: bool,
) -> i32
where
    L: FnMut(Option<&str>, bool, usize) -> Result<(Vec<Post>, StatusLine), String>,
    P: FnMut(&str, &str, bool) -> i32,
{
    for key in keys {
        match step(feed, key, load, post, limit, dry_run) {
            Step::Failed => return 1,
            Step::Quit => return 0,
            Step::Continue => {}
        }
    }
    0
}

fn step<L, P>(
    feed: &mut Feed,
    key: &str,
    load: &mut L,
    post: &mut P,
    limit: usize,
    dry_run: bool,
) -> Step
where
    L: FnMut(Option<&str>, bool, usize) -> Result<(Vec<Post>, StatusLine), String>,
    P: FnMut(&str, &str, bool) -> i32,
{
    match feed.apply(key) {
        FeedEffect::Stay => Step::Continue,
        FeedEffect::Quit => Step::Quit,
        FeedEffect::Post { text, reply_to } => {
            if post(&text, &reply_to, dry_run) == 0 {
                Step::Continue
            } else {
                Step::Failed
            }
        }
        FeedEffect::Search { topic, recent } => match load(Some(&topic), recent, limit) {
            Ok((posts, status)) => {
                feed.replace(posts, &status, Some(topic));
                Step::Continue
            }
            Err(error) => {
                eprintln!("{error}");
                Step::Failed
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ProviderId, StatusLine};

    fn post(id: &str, likes: u64, created: &str) -> Post {
        Post {
            id: id.into(),
            url: format!("https://x.com/a/status/{id}"),
            author_handle: "a".into(),
            author_name: "A".into(),
            text: format!("tweet {id}"),
            created_at: Some(created.into()),
            likes,
            reposts: 0,
            replies: 0,
            bookmarks: 0,
            views: 0,
        }
    }

    fn status(note: &str) -> StatusLine {
        StatusLine::free(
            ProviderId::OpenCli,
            Some("uses your own logged-in session"),
            note,
        )
    }

    #[test]
    fn hot_ranks_engagement_then_newer() {
        let posts = vec![
            post("old-big", 10, "2026-10-01T00:00:00Z"),
            post("new-small", 1, "2026-10-08T00:00:00Z"),
            post("new-big", 10, "2026-10-08T00:00:00Z"),
        ];
        let feed = Feed::new(posts, &status("search"), RankMode::Hot, None);
        let ids: Vec<_> = feed.posts.iter().map(|post| post.id.as_str()).collect();
        assert_eq!(ids, ["new-big", "old-big", "new-small"]);
    }

    #[test]
    fn upcoming_ranks_newer_then_engagement() {
        let posts = vec![
            post("old-big", 50, "2026-10-01T00:00:00Z"),
            post("new-small", 1, "2026-10-08T00:00:00Z"),
        ];
        let feed = Feed::new(posts, &status("search"), RankMode::Upcoming, None);
        assert_eq!(feed.posts[0].id, "new-small");
        assert_eq!(feed.posts[1].id, "old-big");
    }

    #[test]
    fn y_does_not_post_until_preview() {
        let mut feed = Feed::new(
            vec![post("1001", 3, "2026-10-08T00:00:00Z")],
            &status("timeline"),
            RankMode::Upcoming,
            None,
        );
        assert_eq!(feed.apply("r"), FeedEffect::Stay);
        assert_eq!(feed.apply("type:hello from the terminal"), FeedEffect::Stay);
        assert_eq!(feed.apply("y"), FeedEffect::Stay);
        assert!(feed.render().contains("press p to preview"));
        assert_eq!(feed.apply("p"), FeedEffect::Stay);
        assert!(feed.render().contains("not posted"));
        assert_eq!(
            feed.apply("y"),
            FeedEffect::Post {
                text: "hello from the terminal".into(),
                reply_to: "1001".into(),
            }
        );
    }

    #[test]
    fn esc_and_dry_keys_never_emit_post() {
        let mut feed = Feed::new(
            vec![post("1001", 3, "2026-10-08T00:00:00Z")],
            &status("timeline"),
            RankMode::Upcoming,
            None,
        );
        let effects = replay(&mut feed, &split_keys("r;type:nope;p;esc;y"));
        assert!(
            effects
                .iter()
                .all(|effect| !matches!(effect, FeedEffect::Post { .. }))
        );
        assert!(!feed.reply_open);
    }

    #[test]
    fn topic_key_asks_for_one_search() {
        let mut feed = Feed::new(
            vec![post("1", 1, "2026-10-08T00:00:00Z")],
            &status("timeline"),
            RankMode::Hot,
            None,
        );
        assert_eq!(
            feed.apply("/generator sizing"),
            FeedEffect::Search {
                topic: "generator sizing".into(),
                recent: false,
            }
        );
    }

    #[test]
    fn quota_is_shown_only_when_the_path_reports_it() {
        assert_eq!(quota_label(&status("search")), "quota: not reported");
        assert_eq!(
            quota_label(&status("x-rate-limit remaining=17")),
            "quota: 17 remaining"
        );
        let feed = Feed::new(
            Vec::new(),
            &status("remaining: 4"),
            RankMode::Upcoming,
            None,
        );
        assert!(feed.render().contains("quota: 4 remaining"));
    }
}
