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
pub mod publish;
pub mod splash;
pub mod text;

pub use config::XConfig;
pub use model::{Post, Profile, ProviderId, ResearchHit, Served, StatusLine, Thread, Trend};
pub use publish::{ConfirmDecision, GateOutcome, PublishPlan};
pub use text::{intent_url, split_thread, weighted_len, WeightedCount};
