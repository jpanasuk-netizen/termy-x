//! Headless X client for Termy X.
//!
//! Official API access is optional and off by default. Reads fall through
//! OpenCLI, then twitter-cli (the Agent Reach path), then fixture data.
//! Publishing falls through OpenCLI, twitter-cli, then an X web intent.

pub mod ai;
pub mod auth;
pub mod cli;
pub mod config;
pub mod doctor;
pub mod drafts;
pub mod io;
pub mod model;
pub mod providers;
pub mod premium;
pub mod publish;
pub mod splash;
pub mod text;

pub use config::XConfig;
pub use model::{Post, Profile, ProviderId, ResearchHit, Served, StatusLine, Thread, Trend};
pub use publish::{ConfirmDecision, GateOutcome, PublishPlan};
pub use premium::{resolve_char_limit, resolve_char_limit_simple, ResolvedLimit};
pub use text::{
    intent_url, split_thread, split_thread_limited, weighted_len, weighted_len_limited,
    PREMIUM_CHAR_LIMIT, STANDARD_CHAR_LIMIT, WEIGHTED_LIMIT, WeightedCount,
};
