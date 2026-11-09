//! Bulk like / follow planning: skip, cap, and injectable pacing.
//!
//! Live OpenCLI writes stay on the panel. This module never talks to X.
//! Tests inject a sleeper and rng so they do not sleep.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::model::{Post, ProviderId, strip_handle};
use crate::publish::{EngageAction, Publisher};

pub const DEFAULT_BULK_CAP: usize = 30;
pub const MIN_BULK_CAP: usize = 25;
pub const MAX_BULK_CAP: usize = 50;
pub const PACE_MIN_MS: u64 = 3_000;
pub const PACE_MAX_MS: u64 = 8_000;

pub fn clamp_bulk_cap(n: usize) -> usize {
    n.clamp(MIN_BULK_CAP, MAX_BULK_CAP)
}

pub fn cap_stop_note(cap: usize) -> String {
    format!("Stopped at cap ({cap}). Run again later.")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BulkKind {
    LikeReplies,
    FollowRepliers,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkTarget {
    pub post_id: String,
    pub url: String,
    pub author_handle: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkPlan {
    pub kind: BulkKind,
    pub targets: Vec<BulkTarget>,
    /// Already-liked, already-followed, self, or duplicate authors.
    pub skipped: usize,
    pub cap: usize,
    /// True when eligible actions were truncated to the cap.
    pub capped: bool,
    /// Eligible before the cap was applied.
    pub eligible: usize,
}

impl BulkPlan {
    pub fn action_count(&self) -> usize {
        self.targets.len()
    }

    pub fn confirm_summary(&self) -> String {
        let n = self.action_count();
        let action = match self.kind {
            BulkKind::LikeReplies => {
                format!("like {n} {}", if n == 1 { "reply" } else { "replies" })
            }
            BulkKind::FollowRepliers => {
                format!("follow {n} {}", if n == 1 { "replier" } else { "repliers" })
            }
        };
        format!(
            "This will {action} (skipped {}, cap {}). Nothing happens until Confirm.",
            self.skipped, self.cap
        )
    }
}

pub fn plan_bulk(
    kind: BulkKind,
    replies: &[Post],
    own_handle: Option<&str>,
    cap: usize,
) -> BulkPlan {
    let cap = clamp_bulk_cap(cap);
    let mut skipped = 0usize;
    let mut targets = Vec::new();
    let mut seen_authors: HashSet<String> = HashSet::new();

    for post in replies {
        if same_handle(&post.author_handle, own_handle) {
            skipped += 1;
            continue;
        }
        match kind {
            BulkKind::LikeReplies => {
                if post.viewer_liked == Some(true) {
                    skipped += 1;
                    continue;
                }
                targets.push(target_from(post));
            }
            BulkKind::FollowRepliers => {
                let key = strip_handle(&post.author_handle).to_ascii_lowercase();
                if !seen_authors.insert(key.clone()) {
                    skipped += 1;
                    continue;
                }
                let already = replies.iter().any(|other| {
                    strip_handle(&other.author_handle).eq_ignore_ascii_case(&key)
                        && other.viewer_follows_author == Some(true)
                });
                if already {
                    skipped += 1;
                    continue;
                }
                targets.push(target_from(post));
            }
        }
    }

    let eligible = targets.len();
    let capped = eligible > cap;
    if capped {
        targets.truncate(cap);
    }
    BulkPlan {
        kind,
        targets,
        skipped,
        cap,
        capped,
        eligible,
    }
}

fn target_from(post: &Post) -> BulkTarget {
    let url = if post.url.is_empty() {
        crate::model::post_url(&post.author_handle, &post.id)
    } else {
        post.url.clone()
    };
    BulkTarget {
        post_id: post.id.clone(),
        url,
        author_handle: post.author_handle.clone(),
    }
}

fn same_handle(handle: &str, own: Option<&str>) -> bool {
    own.is_some_and(|own| strip_handle(handle).eq_ignore_ascii_case(strip_handle(own)))
}

pub trait DelayRng {
    fn delay_ms(&mut self) -> u64;
}

/// Wall-clock jitter in the 3–8 s band. Tests pass a [`SequenceRng`].
pub struct WallRng;

impl DelayRng for WallRng {
    fn delay_ms(&mut self) -> u64 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos() as u64);
        let span = PACE_MAX_MS - PACE_MIN_MS + 1;
        PACE_MIN_MS + (nanos % span)
    }
}

/// Returns the next value, clamped into the 3–8 s band.
pub struct SequenceRng {
    pub values: Vec<u64>,
    pub index: usize,
}

impl DelayRng for SequenceRng {
    fn delay_ms(&mut self) -> u64 {
        let value = self.values.get(self.index).copied().unwrap_or(PACE_MIN_MS);
        self.index += 1;
        value.clamp(PACE_MIN_MS, PACE_MAX_MS)
    }
}

pub trait Sleeper {
    fn sleep(&self, ms: u64);
}

pub struct ThreadSleeper;

impl Sleeper for ThreadSleeper {
    fn sleep(&self, ms: u64) {
        if ms > 0 {
            std::thread::sleep(Duration::from_millis(ms));
        }
    }
}

/// Records requested delays and never sleeps. For unit tests.
#[derive(Default)]
pub struct RecordingSleeper {
    pub sleeps: Mutex<Vec<u64>>,
}

impl Sleeper for RecordingSleeper {
    fn sleep(&self, ms: u64) {
        self.sleeps.lock().expect("sleeper").push(ms);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BulkStop {
    Completed,
    Stopped,
    Cap,
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkOutcome {
    pub done: usize,
    pub total: usize,
    pub stop: BulkStop,
    pub delays_ms: Vec<u64>,
}

impl BulkOutcome {
    pub fn note(&self, cap: usize) -> String {
        match &self.stop {
            BulkStop::Completed => format!("Done ({}/{}).", self.done, self.total),
            BulkStop::Stopped => format!("Stopped ({}/{}).", self.done, self.total),
            BulkStop::Cap => cap_stop_note(cap),
            BulkStop::Error(error) => {
                format!("Stopped on error ({}/{}): {error}", self.done, self.total)
            }
        }
    }
}

/// Run the plan on a background-friendly path. Delay is between actions
/// (not before the first). Stops on the first publisher error, on cap
/// (plan already truncated), or when `should_stop` returns true.
pub fn run_bulk(
    plan: &BulkPlan,
    publisher: &dyn Publisher,
    rng: &mut dyn DelayRng,
    sleeper: &dyn Sleeper,
    mut progress: impl FnMut(usize, usize),
    should_stop: impl Fn() -> bool,
) -> BulkOutcome {
    let total = plan.targets.len();
    let mut delays_ms = Vec::new();
    let mut done = 0usize;
    for (index, target) in plan.targets.iter().enumerate() {
        if should_stop() {
            return BulkOutcome {
                done,
                total,
                stop: BulkStop::Stopped,
                delays_ms,
            };
        }
        if index > 0 {
            let ms = rng.delay_ms().clamp(PACE_MIN_MS, PACE_MAX_MS);
            sleeper.sleep(ms);
            delays_ms.push(ms);
            if should_stop() {
                return BulkOutcome {
                    done,
                    total,
                    stop: BulkStop::Stopped,
                    delays_ms,
                };
            }
        }
        let result = match plan.kind {
            BulkKind::LikeReplies => publisher.engage(EngageAction::Like, &target.url, None, &[]),
            BulkKind::FollowRepliers => publisher.set_follow(&target.author_handle, true),
        };
        match result {
            Ok(_) => {
                done += 1;
                progress(done, total);
            }
            Err(error) => {
                return BulkOutcome {
                    done,
                    total,
                    stop: BulkStop::Error(error),
                    delays_ms,
                };
            }
        }
    }
    let stop = if plan.capped {
        BulkStop::Cap
    } else {
        BulkStop::Completed
    };
    BulkOutcome {
        done,
        total,
        stop,
        delays_ms,
    }
}

/// In-memory like/follow log for tests and mock preview. Never talks to X.
#[derive(Clone, Default)]
pub struct MockWriteLog {
    likes: Arc<Mutex<Vec<String>>>,
    follows: Arc<Mutex<Vec<String>>>,
    fail_next: Arc<Mutex<Option<String>>>,
}

impl MockWriteLog {
    pub fn likes(&self) -> Vec<String> {
        self.likes.lock().expect("likes").clone()
    }

    pub fn follows(&self) -> Vec<String> {
        self.follows.lock().expect("follows").clone()
    }

    pub fn fail_next(&self, error: impl Into<String>) {
        *self.fail_next.lock().expect("fail") = Some(error.into());
    }

    fn take_fail(&self) -> Result<(), String> {
        if let Some(error) = self.fail_next.lock().expect("fail").take() {
            Err(error)
        } else {
            Ok(())
        }
    }
}

impl Publisher for MockWriteLog {
    fn id(&self) -> ProviderId {
        ProviderId::Mock
    }

    fn available(&self) -> bool {
        true
    }

    fn session_note(&self) -> Option<String> {
        Some("mock: in-memory, nothing sent to X".into())
    }

    fn engage(
        &self,
        action: EngageAction,
        target_url: &str,
        _text: Option<&str>,
        _media: &[PathBuf],
    ) -> Result<String, String> {
        self.take_fail()?;
        let mut likes = self.likes.lock().expect("likes");
        match action {
            EngageAction::Like => {
                if !likes.iter().any(|u| u == target_url) {
                    likes.push(target_url.to_string());
                }
                Ok(format!("mock liked {target_url}"))
            }
            EngageAction::Unlike => {
                likes.retain(|u| u != target_url);
                Ok(format!("mock unliked {target_url}"))
            }
            EngageAction::Retweet | EngageAction::Unretweet | EngageAction::Quote => {
                Ok(format!("mock {action:?} recorded locally"))
            }
        }
    }

    fn set_follow(&self, handle: &str, follow: bool) -> Result<String, String> {
        self.take_fail()?;
        let handle = strip_handle(handle);
        if handle.is_empty() {
            return Err("no handle to follow — nothing was sent".into());
        }
        let mut follows = self.follows.lock().expect("follows");
        let key = handle.to_string();
        if follow {
            if !follows.iter().any(|h| h.eq_ignore_ascii_case(&key)) {
                follows.push(key.clone());
            }
            Ok(format!("mock following @{key}"))
        } else {
            follows.retain(|h| !h.eq_ignore_ascii_case(&key));
            Ok(format!("mock unfollowed @{key}"))
        }
    }

    fn publish(
        &self,
        _parts: &[String],
        _reply_to: Option<&str>,
        _media: &[PathBuf],
    ) -> Result<String, String> {
        Err("mock never posts to X".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::sample_thread;

    fn post(id: &str, handle: &str, liked: Option<bool>, follows: Option<bool>) -> Post {
        Post {
            id: id.into(),
            url: format!("https://x.com/{handle}/status/{id}"),
            author_handle: handle.into(),
            author_name: handle.into(),
            viewer_liked: liked,
            viewer_follows_author: follows,
            ..Post::default()
        }
    }

    #[test]
    fn clamp_keeps_cap_in_25_50() {
        assert_eq!(clamp_bulk_cap(30), 30);
        assert_eq!(clamp_bulk_cap(1), 25);
        assert_eq!(clamp_bulk_cap(80), 50);
        assert_eq!(clamp_bulk_cap(25), 25);
        assert_eq!(clamp_bulk_cap(50), 50);
    }

    #[test]
    fn like_plan_skips_already_liked_and_self() {
        let replies = vec![
            post("1", "ada", Some(false), None),
            post("2", "bob", Some(true), None),
            post("3", "me", None, None),
            post("4", "ada", None, None),
        ];
        let plan = plan_bulk(BulkKind::LikeReplies, &replies, Some("me"), 30);
        assert_eq!(
            plan.targets
                .iter()
                .map(|t| t.post_id.as_str())
                .collect::<Vec<_>>(),
            vec!["1", "4"]
        );
        assert_eq!(plan.skipped, 2);
        assert!(!plan.capped);
    }

    #[test]
    fn follow_plan_skips_self_already_followed_and_duplicate_authors() {
        let replies = vec![
            post("1", "ada", None, Some(false)),
            post("2", "bob", None, Some(true)),
            post("3", "Ada", None, None),
            post("4", "@me", None, None),
            post("5", "cara", None, None),
        ];
        let plan = plan_bulk(BulkKind::FollowRepliers, &replies, Some("me"), 30);
        assert_eq!(
            plan.targets
                .iter()
                .map(|t| t.author_handle.as_str())
                .collect::<Vec<_>>(),
            vec!["ada", "cara"]
        );
        // bob already followed, Ada duplicate of ada, @me self
        assert_eq!(plan.skipped, 3);
    }

    #[test]
    fn cap_truncates_eligible_actions() {
        let replies: Vec<Post> = (0..40)
            .map(|i| post(&format!("{i}"), &format!("u{i}"), None, None))
            .collect();
        let plan = plan_bulk(BulkKind::LikeReplies, &replies, None, 30);
        assert_eq!(plan.action_count(), 30);
        assert_eq!(plan.eligible, 40);
        assert!(plan.capped);
        assert_eq!(plan.cap, 30);
        let tight = plan_bulk(BulkKind::LikeReplies, &replies, None, 10);
        assert_eq!(tight.action_count(), 25, "10 clamps up to 25");
    }

    #[test]
    fn sample_thread_fixture_skips_liked_followed_and_self() {
        let thread = sample_thread("1001");
        let likes = plan_bulk(BulkKind::LikeReplies, &thread.replies, Some("termy"), 30);
        assert!(
            !likes.targets.iter().any(|t| t.post_id == "2006"),
            "already-liked sibling skipped"
        );
        assert!(
            !likes.targets.iter().any(|t| t.author_handle == "termy"),
            "self skipped"
        );
        assert_eq!(likes.action_count(), 4);

        let follows = plan_bulk(BulkKind::FollowRepliers, &thread.replies, Some("termy"), 30);
        assert!(
            !follows
                .targets
                .iter()
                .any(|t| t.author_handle.eq_ignore_ascii_case("draft_desk")),
            "already-followed author skipped (and not duplicated)"
        );
        assert!(
            !follows
                .targets
                .iter()
                .any(|t| t.author_handle.eq_ignore_ascii_case("termy"))
        );
        let handles: Vec<_> = follows
            .targets
            .iter()
            .map(|t| t.author_handle.as_str())
            .collect();
        assert_eq!(handles, vec!["blue_swift", "panel_notes", "swift_fan"]);
    }

    #[test]
    fn mock_log_records_likes_and_follows_in_memory() {
        let log = MockWriteLog::default();
        log.engage(EngageAction::Like, "https://x.com/a/status/1", None, &[])
            .unwrap();
        log.set_follow("ada", true).unwrap();
        log.set_follow("@Ada", true).unwrap();
        assert_eq!(log.likes(), vec!["https://x.com/a/status/1"]);
        assert_eq!(log.follows(), vec!["ada"]);
    }

    #[test]
    fn run_bulk_likes_without_sleeping_and_paces_between_actions() {
        let replies = vec![post("1", "ada", None, None), post("2", "bob", None, None)];
        let plan = plan_bulk(BulkKind::LikeReplies, &replies, None, 30);
        let log = MockWriteLog::default();
        let sleeper = RecordingSleeper::default();
        let mut rng = SequenceRng {
            values: vec![4_000, 99_000],
            index: 0,
        };
        let mut ticks = Vec::new();
        let outcome = run_bulk(
            &plan,
            &log,
            &mut rng,
            &sleeper,
            |done, total| ticks.push((done, total)),
            || false,
        );
        assert_eq!(outcome.stop, BulkStop::Completed);
        assert_eq!(outcome.done, 2);
        assert_eq!(ticks, vec![(1, 2), (2, 2)]);
        assert_eq!(sleeper.sleeps.lock().unwrap().clone(), vec![4_000]);
        assert_eq!(outcome.delays_ms, vec![4_000]);
        assert_eq!(log.likes().len(), 2);
    }

    #[test]
    fn run_bulk_stops_on_first_error_and_on_should_stop() {
        let replies = vec![
            post("1", "ada", None, None),
            post("2", "bob", None, None),
            post("3", "cara", None, None),
        ];
        let plan = plan_bulk(BulkKind::FollowRepliers, &replies, None, 30);
        let log = MockWriteLog::default();
        log.fail_next("opencli exploded");
        let sleeper = RecordingSleeper::default();
        let mut rng = SequenceRng {
            values: vec![3_000],
            index: 0,
        };
        let outcome = run_bulk(&plan, &log, &mut rng, &sleeper, |_, _| {}, || false);
        assert!(matches!(outcome.stop, BulkStop::Error(_)));
        assert_eq!(outcome.done, 0);
        assert!(log.follows().is_empty());

        let log = MockWriteLog::default();
        let outcome = run_bulk(
            &plan,
            &log,
            &mut SequenceRng {
                values: vec![3_000, 3_000],
                index: 0,
            },
            &RecordingSleeper::default(),
            |_, _| {},
            || true,
        );
        assert_eq!(outcome.stop, BulkStop::Stopped);
        assert_eq!(outcome.done, 0);
        assert!(log.follows().is_empty());
    }

    #[test]
    fn finishing_a_capped_plan_reports_cap_note() {
        let replies: Vec<Post> = (0..40)
            .map(|i| post(&format!("{i}"), &format!("u{i}"), None, None))
            .collect();
        let plan = plan_bulk(BulkKind::LikeReplies, &replies, None, 30);
        let log = MockWriteLog::default();
        let outcome = run_bulk(
            &plan,
            &log,
            &mut SequenceRng {
                values: vec![3_000; 40],
                index: 0,
            },
            &RecordingSleeper::default(),
            |_, _| {},
            || false,
        );
        assert_eq!(outcome.stop, BulkStop::Cap);
        assert_eq!(outcome.done, 30);
        assert_eq!(outcome.note(plan.cap), cap_stop_note(30));
        assert!(outcome.note(plan.cap).contains("Run again later"));
    }

    #[test]
    fn confirm_summary_names_the_count_after_skips_and_the_cap() {
        let replies = vec![
            post("1", "ada", None, None),
            post("2", "bob", Some(true), None),
        ];
        let plan = plan_bulk(BulkKind::LikeReplies, &replies, None, 30);
        let text = plan.confirm_summary();
        assert!(text.contains("1"), "{text}");
        assert!(text.contains("cap 30"), "{text}");
        assert!(text.contains("skipped 1"), "{text}");
        assert!(text.contains("Nothing happens until Confirm"), "{text}");
    }
}
