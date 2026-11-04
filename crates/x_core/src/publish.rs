use crate::model::{ProviderId, SESSION_OWN};
use crate::text::{WeightedCount, intent_url};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmDecision {
    Yes,
    No,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishPlan {
    pub text: String,
    pub parts: Vec<String>,
    pub count: WeightedCount,
    pub provider: ProviderId,
    pub session: Option<String>,
    pub intent_urls: Vec<String>,
    pub dry_run: bool,
    /// Local image paths to attach (jpg/png/gif/webp). Max 4 for OpenCLI.
    pub media: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateOutcome {
    Rejected { plan: PublishPlan },
    DryRun { plan: PublishPlan },
    Sent { plan: PublishPlan, receipt: String },
}

pub trait Publisher: Send + Sync {
    fn id(&self) -> ProviderId;
    fn available(&self) -> bool;
    fn session_note(&self) -> Option<String>;
    fn publish(
        &self,
        parts: &[String],
        reply_to: Option<&str>,
        media: &[PathBuf],
    ) -> Result<String, String>;
    /// True when this publisher can upload the given media paths itself.
    fn supports_media(&self) -> bool {
        false
    }
}

pub struct IntentPublisher {
    pub open_browser: bool,
    pub opener: std::sync::Arc<dyn crate::io::Opener>,
}

impl Publisher for IntentPublisher {
    fn id(&self) -> ProviderId {
        ProviderId::WebIntent
    }

    fn available(&self) -> bool {
        true
    }

    fn session_note(&self) -> Option<String> {
        Some("you click Post in the browser".to_string())
    }

    fn publish(
        &self,
        parts: &[String],
        reply_to: Option<&str>,
        media: &[PathBuf],
    ) -> Result<String, String> {
        let urls = intent_urls(parts, reply_to);
        if self.open_browser
            && let Some(url) = urls.first()
        {
            self.opener.open(url)?;
        }
        if media.is_empty() {
            Ok(format!("opened web intent for {} part(s)", parts.len()))
        } else {
            let list = media
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            Ok(format!(
                "opened web intent for {} part(s). Attach these images in the browser compose UI (web intent cannot upload files): {list}",
                parts.len()
            ))
        }
    }
}

pub fn intent_urls(parts: &[String], reply_to: Option<&str>) -> Vec<String> {
    parts
        .iter()
        .enumerate()
        .map(|(index, part)| {
            let reply = if index == 0 { reply_to } else { None };
            intent_url(part, reply)
        })
        .collect()
}

pub fn build_plan(
    text: &str,
    reply_to: Option<&str>,
    publisher: &dyn Publisher,
    dry_run: bool,
) -> PublishPlan {
    build_plan_limited(
        text,
        reply_to,
        publisher,
        dry_run,
        crate::text::STANDARD_CHAR_LIMIT,
    )
}

pub fn build_plan_limited(
    text: &str,
    reply_to: Option<&str>,
    publisher: &dyn Publisher,
    dry_run: bool,
    char_limit: usize,
) -> PublishPlan {
    build_plan_with_media(text, reply_to, publisher, dry_run, char_limit, &[])
}

pub fn build_plan_with_media(
    text: &str,
    reply_to: Option<&str>,
    publisher: &dyn Publisher,
    dry_run: bool,
    char_limit: usize,
    media: &[PathBuf],
) -> PublishPlan {
    let parts = crate::text::split_thread_limited(text, char_limit);
    let parts = if parts.is_empty() {
        vec![String::new()]
    } else {
        parts
    };
    let media = media.to_vec();
    let session = match (
        publisher.session_note(),
        media.is_empty(),
        publisher.supports_media(),
    ) {
        (Some(note), false, false) => Some(format!(
            "{note}. Media will need a manual attach in the browser if this path cannot upload files."
        )),
        (Some(note), _, _) => Some(note),
        (None, false, false) => {
            Some("media listed below; provider may require manual attach in the browser".into())
        }
        _ => None,
    };
    PublishPlan {
        count: crate::text::weighted_len_limited(text.trim(), char_limit),
        intent_urls: intent_urls(&parts, reply_to),
        text: text.trim().to_string(),
        parts,
        provider: publisher.id(),
        session,
        dry_run,
        media,
    }
}

/// Publish only after an explicit yes. Dry-run never calls the publisher.
pub fn confirm_and_publish(
    text: &str,
    reply_to: Option<&str>,
    decision: ConfirmDecision,
    dry_run: bool,
    publisher: &dyn Publisher,
) -> GateOutcome {
    confirm_and_publish_limited(
        text,
        reply_to,
        decision,
        dry_run,
        publisher,
        crate::text::STANDARD_CHAR_LIMIT,
    )
}

pub fn confirm_and_publish_limited(
    text: &str,
    reply_to: Option<&str>,
    decision: ConfirmDecision,
    dry_run: bool,
    publisher: &dyn Publisher,
    char_limit: usize,
) -> GateOutcome {
    confirm_and_publish_with_media(
        text,
        reply_to,
        decision,
        dry_run,
        publisher,
        char_limit,
        &[],
    )
}

pub fn confirm_and_publish_with_media(
    text: &str,
    reply_to: Option<&str>,
    decision: ConfirmDecision,
    dry_run: bool,
    publisher: &dyn Publisher,
    char_limit: usize,
    media: &[PathBuf],
) -> GateOutcome {
    let plan = build_plan_with_media(text, reply_to, publisher, dry_run, char_limit, media);
    if decision != ConfirmDecision::Yes {
        return GateOutcome::Rejected { plan };
    }
    if dry_run {
        return GateOutcome::DryRun { plan };
    }
    match publisher.publish(&plan.parts, reply_to, &plan.media) {
        Ok(receipt) => GateOutcome::Sent { plan, receipt },
        Err(error) => GateOutcome::Rejected {
            plan: PublishPlan {
                session: Some(error),
                ..plan
            },
        },
    }
}

pub fn first_available(publishers: &[Box<dyn Publisher>]) -> Option<&dyn Publisher> {
    publishers
        .iter()
        .find(|publisher| publisher.available())
        .map(|publisher| publisher.as_ref())
}

pub fn session_label(provider: ProviderId) -> Option<String> {
    match provider {
        ProviderId::OpenCli | ProviderId::TwitterCli => Some(SESSION_OWN.to_string()),
        ProviderId::WebIntent => Some("you click Post in the browser".to_string()),
        ProviderId::Mock | ProviderId::OfficialApi => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::percent_encode;
    use std::sync::{Arc, Mutex};

    struct RecordingPublisher {
        calls: Arc<Mutex<Vec<Vec<String>>>>,
    }

    impl Publisher for RecordingPublisher {
        fn id(&self) -> ProviderId {
            ProviderId::Mock
        }
        fn available(&self) -> bool {
            true
        }
        fn session_note(&self) -> Option<String> {
            None
        }
        fn publish(
            &self,
            parts: &[String],
            _reply_to: Option<&str>,
            _media: &[PathBuf],
        ) -> Result<String, String> {
            self.calls.lock().expect("calls").push(parts.to_vec());
            Ok("posted".to_string())
        }
    }

    #[test]
    fn no_and_dry_run_never_publish() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let publisher = RecordingPublisher {
            calls: calls.clone(),
        };
        let text = "exact post text";
        let rejected = confirm_and_publish(text, None, ConfirmDecision::No, false, &publisher);
        assert!(matches!(rejected, GateOutcome::Rejected { .. }));
        let dry = confirm_and_publish(text, None, ConfirmDecision::Yes, true, &publisher);
        match dry {
            GateOutcome::DryRun { plan } => {
                assert_eq!(plan.text, text);
                assert_eq!(plan.count.weighted, text.len());
                assert!(plan.dry_run);
            }
            other => panic!("expected dry run, got {other:?}"),
        }
        assert!(calls.lock().expect("calls").is_empty());
    }

    #[test]
    fn yes_without_dry_run_publishes_the_exact_text() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let publisher = RecordingPublisher {
            calls: calls.clone(),
        };
        let outcome = confirm_and_publish(
            "ship it",
            Some("55"),
            ConfirmDecision::Yes,
            false,
            &publisher,
        );
        assert!(matches!(outcome, GateOutcome::Sent { .. }));
        assert_eq!(
            calls.lock().expect("calls").clone(),
            vec![vec!["ship it".to_string()]]
        );
    }

    #[test]
    fn intent_urls_encode_spaces_newlines_and_reply_ids() {
        let url = crate::text::intent_url("hello world\nnext & more", Some("99 1"));
        assert_eq!(
            url,
            "https://x.com/intent/post?text=hello%20world%0Anext%20%26%20more&in_reply_to=99%201"
        );
        assert_eq!(percent_encode("a b"), "a%20b");
        assert!(!url.contains('+'));
        let urls = intent_urls(&["one".into(), "two".into()], Some("7"));
        assert_eq!(urls.len(), 2);
        assert!(urls[0].contains("in_reply_to=7"));
        assert!(!urls[1].contains("in_reply_to"));
    }
}
