//! Headless X client for Termy X.
//!
//! Official API access is optional and off by default. Reads fall through
//! OpenCLI, then twitter-cli (the Agent Reach path), then fixture data.
//! Publishing falls through OpenCLI, twitter-cli, then an X web intent.

pub mod action_state;
pub mod agents;
pub mod ai;
pub mod analytics_stats;
pub mod auth;
pub mod bulk;
pub mod cli;
pub mod config;
pub mod connect;
pub mod doctor;
pub mod draft_edit;
pub mod drafts;
pub mod feature_line;
pub mod feed;
pub mod feeds;
pub mod followers;
pub mod handoff;
pub mod inbox;
pub mod io;
pub mod model;
mod parse_borrowed;
pub mod platform;
pub mod premium;
pub mod providers;
pub mod publish;
pub mod queue;
pub mod reply_guy;
pub mod reply_guy_multi;
pub mod sort;
pub mod splash;
pub mod supabase_auth;
pub mod supabase_defaults;
pub mod supabase_sync;
pub mod stats;
pub mod text;
pub mod thread;
pub mod time;

pub use bulk::{
    BulkKind, BulkPlan, BulkTarget, DEFAULT_BULK_CAP, MAX_BULK_CAP, MIN_BULK_CAP, clamp_bulk_cap,
    plan_bulk,
};
pub use config::XConfig;
pub use connect::Connection;
pub use model::{Post, Profile, ProviderId, ResearchHit, Served, StatusLine, Thread, Trend};
pub use premium::{ResolvedLimit, resolve_char_limit, resolve_char_limit_simple};
pub use publish::{ConfirmDecision, GateOutcome, PublishPlan};
pub use sort::{LIST_SORT_KEY, ResultSort, cmp_posts, sort_posts, sorted_posts};
pub use stats::{
    AccountStats, ProfileTotals, TWEET_WINDOW_CAP, TweetWindowStats, account_stats_from_profile,
    fmt_count, fmt_created, fmt_rate, fmt_verified, profile_totals_from_json, render_account_stats,
    tweet_window_from_json, tweet_window_from_posts,
};
pub use text::{
    PREMIUM_CHAR_LIMIT, STANDARD_CHAR_LIMIT, WEIGHTED_LIMIT, WeightedCount, intent_url,
    split_thread, split_thread_limited, weighted_len, weighted_len_limited,
};
pub use thread::{MAX_DISPLAY_DEPTH, ThreadNode, build_reply_tree, walk_thread};
