//! Local state for Inbox, Mentions, Feeds, Stats, Followers, Queue, Analyze.

use termy_x::XConfig;
use termy_x::analytics_stats::{AccountReport, Heatmap, PeriodCompare};
use termy_x::feeds::{EngageFeed, FeedStore};
use termy_x::followers::{FollowerDiff, FollowerLog};
use termy_x::inbox::{InboxState, InboxStore};
use termy_x::model::Post;
use termy_x::queue::QueuedReply;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InboxFilter {
    All,
    Waiting,
    Done,
}

pub struct DeskState {
    pub inbox_posts: Vec<Post>,
    pub inbox_store: InboxStore,
    pub inbox_filter: InboxFilter,
    pub inbox_error: Option<String>,
    pub mentions_posts: Vec<Post>,
    pub mentions_error: Option<String>,
    pub feed_store: FeedStore,
    pub feed_selected: Option<String>,
    pub feed_query: String,
    pub feed_filter: String,
    pub feed_posts: Vec<Post>,
    pub feed_error: Option<String>,
    pub stats: Option<PeriodCompare>,
    pub stats_heat: Option<Heatmap>,
    pub stats_top: Vec<Post>,
    pub stats_days: u32,
    pub stats_error: Option<String>,
    pub follower_log: FollowerLog,
    pub follower_diff: Option<FollowerDiff>,
    pub follower_error: Option<String>,
    pub queue: Vec<QueuedReply>,
    pub queue_text: String,
    pub queue_reply_to: String,
    pub analyze_handle: String,
    pub analyze: Option<AccountReport>,
    pub analyze_error: Option<String>,
    pub loading: bool,
    pub drafts_for: Option<String>,
}

impl DeskState {
    pub fn load(config: &XConfig) -> Self {
        let inbox_store = termy_x::inbox::load_inbox(&config.inbox_path());
        let feed_store = termy_x::feeds::load_feeds(&config.feeds_path());
        let follower_log = termy_x::followers::load_log(&config.followers_path());
        let follower_diff = termy_x::followers::diff_latest(&follower_log);
        let queue = termy_x::queue::load_queue(&config.queue_path());
        Self {
            inbox_posts: Vec::new(),
            inbox_store,
            inbox_filter: InboxFilter::Waiting,
            inbox_error: None,
            mentions_posts: Vec::new(),
            mentions_error: None,
            feed_store,
            feed_selected: None,
            feed_query: String::new(),
            feed_filter: String::new(),
            feed_posts: Vec::new(),
            feed_error: None,
            stats: None,
            stats_heat: None,
            stats_top: Vec::new(),
            stats_days: 7,
            stats_error: None,
            follower_log,
            follower_diff,
            follower_error: None,
            queue,
            queue_text: String::new(),
            queue_reply_to: String::new(),
            analyze_handle: String::new(),
            analyze: None,
            analyze_error: None,
            loading: false,
            drafts_for: None,
        }
    }

    pub fn visible_inbox(&self) -> Vec<(Post, InboxState)> {
        let items = termy_x::inbox::apply_states(&self.inbox_posts, &self.inbox_store);
        let want = match self.inbox_filter {
            InboxFilter::All => None,
            InboxFilter::Waiting => Some(InboxState::Waiting),
            InboxFilter::Done => Some(InboxState::Done),
        };
        termy_x::inbox::filter_items(&items, want)
            .into_iter()
            .map(|item| (item.post, item.state))
            .collect()
    }

    pub fn selected_feed(&self) -> Option<&EngageFeed> {
        let id = self.feed_selected.as_deref()?;
        termy_x::feeds::find_feed(&self.feed_store, id)
    }

    pub fn persist_inbox(&self, config: &XConfig) -> Result<(), String> {
        termy_x::inbox::save_inbox(&config.inbox_path(), &self.inbox_store)
    }

    pub fn persist_feeds(&self, config: &XConfig) -> Result<(), String> {
        termy_x::feeds::save_feeds(&config.feeds_path(), &self.feed_store)
    }

    pub fn persist_followers(&self, config: &XConfig) -> Result<(), String> {
        termy_x::followers::save_log(&config.followers_path(), &self.follower_log)
    }

    pub fn persist_queue(&self, config: &XConfig) -> Result<(), String> {
        termy_x::queue::save_queue(&config.queue_path(), &self.queue)
    }
}
