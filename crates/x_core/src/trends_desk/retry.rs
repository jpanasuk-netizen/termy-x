//! Short retries for a busy chat provider. The key never enters these strings.

use std::cell::RefCell;
use std::time::Duration;

pub const RETRY_BACKOFF: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(5)];
pub const VYCE_RETRYING: &str = "Vyce busy, retrying…";

thread_local! {
    static RETRY_TX: RefCell<Option<std::sync::mpsc::Sender<String>>> = const { RefCell::new(None) };
}

pub struct RetryBinding;

impl Drop for RetryBinding {
    fn drop(&mut self) {
        RETRY_TX.with(|slot| *slot.borrow_mut() = None);
    }
}

pub fn bind_retry_notice(tx: Option<std::sync::mpsc::Sender<String>>) -> RetryBinding {
    RETRY_TX.with(|slot| *slot.borrow_mut() = tx);
    RetryBinding
}

fn notify_retry() {
    RETRY_TX.with(|slot| {
        if let Some(tx) = slot.borrow().as_ref() {
            let _ = tx.send(VYCE_RETRYING.to_string());
        }
    });
}

pub fn is_retryable(error: &str) -> bool {
    let lower = error.to_lowercase();
    lower.contains("503")
        || lower.contains("429")
        || lower.contains("service_unavailable")
        || lower.contains("too many requests")
        || lower.contains("rate limit")
}

pub fn flatten_error(error: &str) -> String {
    let flat = error.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = flat.chars();
    let head: String = chars.by_ref().take(500).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

pub fn upstream_message(error: &str) -> String {
    if let Some(json_at) = error.find('{')
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(&error[json_at..])
        && let Some(message) = value
            .pointer("/error/message")
            .and_then(|item| item.as_str())
    {
        let message = message.trim();
        if !message.is_empty() {
            return message.to_string();
        }
    }
    flatten_error(error)
}

pub fn vyce_busy_line(error: &str) -> String {
    let code = if error.contains("429") { "429" } else { "503" };
    let message = upstream_message(error);
    let headline = format!("Vyce glm-5.3-flash busy ({code}), try again or pick another provider");
    if message.is_empty() || message == headline {
        headline
    } else {
        format!("{headline}. {message}")
    }
}

/// First try, then up to two more after 2s and 5s. Tests skip the sleep.
pub fn hermes_failure_note(error: &str) -> String {
    if is_retryable(error) && error.to_lowercase().contains("unavailable") {
        return vyce_busy_line(error);
    }
    if is_retryable(error) {
        return format!("Hermes · {}", flatten_error(error));
    }
    format!("Hermes · {}", super::takes::specific_transport_error(error))
}

pub fn provider_failure_note(label: &str, model: &str, error: &str) -> String {
    if is_retryable(error)
        && (model == super::agents::VYCE_MODEL || error.to_lowercase().contains("unavailable"))
    {
        return vyce_busy_line(error);
    }
    format!("{label} · {}", flatten_error(error))
}

pub fn retry_result(mut call: impl FnMut() -> Result<String, String>) -> Result<String, String> {
    let mut last = String::new();
    for attempt in 0..3 {
        match call() {
            Ok(text) => return Ok(text),
            Err(error) if is_retryable(&error) && attempt < 2 => {
                notify_retry();
                sleep_backoff(attempt);
                last = error;
            }
            Err(error) => return Err(error),
        }
    }
    Err(last)
}

fn sleep_backoff(attempt: usize) {
    let delay = RETRY_BACKOFF
        .get(attempt)
        .copied()
        .unwrap_or(Duration::from_secs(5));
    #[cfg(not(test))]
    if !delay.is_zero() {
        std::thread::sleep(delay);
    }
    #[cfg(test)]
    {
        let _ = delay;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VYCE_503: &str = r#"http 503: {"error":{"message":"This model is currently unavailable. Please try again later.","code":"service_unavailable"}}"#;

    #[test]
    fn busy_line_keeps_the_whole_upstream_sentence() {
        assert_eq!(RETRY_BACKOFF[0], Duration::from_secs(2));
        assert_eq!(RETRY_BACKOFF[1], Duration::from_secs(5));
        let line = vyce_busy_line(VYCE_503);
        assert!(
            line.starts_with("Vyce glm-5.3-flash busy (503), try again or pick another provider")
        );
        assert!(line.contains("This model is currently unavailable. Please try again later."));
        assert!(line.contains("currently unavailable"));
        let logged = crate::trends_desk::agents::take_log_line("vyce", &line);
        assert!(logged.contains("Please try again later"));
        assert!(logged.contains("This model is currently unavailable"));
        assert!(!logged.contains("api-key"));
    }

    #[test]
    fn retryable_503_runs_twice_more_then_stops() {
        let mut calls = 0;
        let err = retry_result(|| {
            calls += 1;
            Err(VYCE_503.to_string())
        })
        .unwrap_err();
        assert_eq!(calls, 3);
        assert!(err.contains("Please try again later"));
        calls = 0;
        let ok = retry_result(|| {
            calls += 1;
            if calls < 3 {
                Err("429 too many requests".into())
            } else {
                Ok("pong".into())
            }
        })
        .unwrap();
        assert_eq!(ok, "pong");
        assert_eq!(calls, 3);
        calls = 0;
        let denied = retry_result(|| {
            calls += 1;
            Err("http 401 unauthorized".into())
        })
        .unwrap_err();
        assert_eq!(calls, 1);
        assert!(denied.contains("401"));
    }
}
