//! OpenCLI JSON shapes used by Trends Desk.
//!
//! OpenCLI 2.0.2 wraps search rows as `{ "ok": true, "data": [ ... ] }`.
//! A parser that only accepts a bare array, `.tweets`, or `.results` reads
//! that envelope as zero posts. This module accepts the envelope, the older
//! keys, and a bare array, and it returns the provider error when `ok` is false.

use serde::Deserialize;
use serde_json::Value;

use super::domain::SourcePost;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    Malformed(String),
    Provider(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(message) | Self::Provider(message) => f.write_str(message),
        }
    }
}

/// First JSON value in `text`, skipping a banner the CLI may print ahead of it.
pub fn parse_json_blob(text: &str) -> Result<Value, ParseError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(ParseError::Malformed("empty OpenCLI output".into()));
    }
    let bytes = trimmed.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'{' && *byte != b'[' {
            continue;
        }
        let slice = &trimmed[index..];
        let mut deserializer = serde_json::Deserializer::from_str(slice);
        if let Ok(value) = Value::deserialize(&mut deserializer) {
            return Ok(value);
        }
    }
    Err(ParseError::Malformed("OpenCLI did not return JSON".into()))
}

/// Unwrap one OpenCLI payload. `ok: false` is an error, never an empty success.
pub fn unwrap_opencli_data(value: Value) -> Result<Value, ParseError> {
    unwrap_opencli_data_depth(value, 0)
}

fn unwrap_opencli_data_depth(value: Value, depth: usize) -> Result<Value, ParseError> {
    if depth > 3 {
        return Ok(value);
    }
    let Value::Object(map) = &value else {
        return Ok(value);
    };
    if map.get("ok").and_then(Value::as_bool) == Some(false) {
        let message = map
            .get("error")
            .and_then(Value::as_str)
            .or_else(|| map.get("message").and_then(Value::as_str))
            .or_else(|| map.get("data").and_then(Value::as_str))
            .unwrap_or("OpenCLI returned ok=false");
        return Err(ParseError::Provider(message.chars().take(400).collect()));
    }
    if let Some(data) = map.get("data").cloned() {
        return match data {
            Value::String(text) => match parse_json_blob(&text) {
                Ok(inner) => unwrap_opencli_data_depth(inner, depth + 1),
                Err(_) => Ok(Value::String(text)),
            },
            other => unwrap_opencli_data_depth(other, depth + 1),
        };
    }
    if let Some(rows) = first_array(&value, &["tweets", "results", "items"]) {
        return Ok(Value::Array(rows));
    }
    Ok(value)
}

fn first_array(value: &Value, keys: &[&str]) -> Option<Vec<Value>> {
    for key in keys {
        if let Some(rows) = value.get(*key).and_then(Value::as_array) {
            return Some(rows.clone());
        }
    }
    None
}

/// Post rows from an unwrapped payload. An empty list is empty, not an error.
pub fn as_post_list(value: Value) -> Result<Vec<Value>, ParseError> {
    let unwrapped = unwrap_opencli_data(value)?;
    if let Some(rows) = unwrapped.as_array() {
        return Ok(rows.clone());
    }
    if let Some(rows) = first_array(&unwrapped, &["tweets", "results", "items"]) {
        return Ok(rows);
    }
    Ok(Vec::new())
}

pub fn posts_from_stdout(text: &str) -> Result<Vec<SourcePost>, ParseError> {
    let value = parse_json_blob(text)?;
    let rows = as_post_list(value)?;
    Ok(rows.iter().filter_map(SourcePost::from_value).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!([
            {
                "id": "1",
                "author": "alice",
                "text": "Hello from a test post about courts",
                "created_at": "Wed Nov 11 16:00:00 +0000 2026",
                "likes": 10,
                "url": "https://x.com/alice/status/1"
            },
            {
                "id": "2",
                "author": "bob",
                "text": "Another post",
                "likes": 3,
                "url": "https://twitter.com/bob/status/2"
            }
        ])
    }

    fn ids(rows: &[Value]) -> Vec<String> {
        rows.iter()
            .map(|row| {
                row.get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn unwraps_bare_array() {
        let rows = as_post_list(sample()).unwrap();
        assert_eq!(ids(&rows), ["1", "2"]);
    }

    #[test]
    fn unwraps_ok_data_envelope() {
        let rows = as_post_list(json!({"ok": true, "data": sample()})).unwrap();
        assert_eq!(ids(&rows), ["1", "2"]);
    }

    #[test]
    fn unwraps_tweets_results_and_nested() {
        assert_eq!(
            ids(&as_post_list(json!({"tweets": sample()})).unwrap()),
            ["1", "2"]
        );
        assert_eq!(
            ids(&as_post_list(json!({"results": sample()})).unwrap()),
            ["1", "2"]
        );
        assert_eq!(
            ids(&as_post_list(json!({"ok": true, "data": {"tweets": sample()}})).unwrap()),
            ["1", "2"]
        );
        assert_eq!(
            ids(&as_post_list(json!({"ok": true, "data": {"results": sample()}})).unwrap()),
            ["1", "2"]
        );
        assert_eq!(
            ids(&as_post_list(json!({"ok": true, "data": {"items": sample()}})).unwrap()),
            ["1", "2"]
        );
    }

    #[test]
    fn empty_envelope_is_empty_not_an_error() {
        let rows = as_post_list(json!({"ok": true, "data": []})).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn legacy_keys_miss_the_envelope() {
        let envelope = json!({"ok": true, "data": sample()});
        assert!(!envelope.is_array());
        let missed = envelope
            .get("tweets")
            .or_else(|| envelope.get("results"))
            .and_then(Value::as_array)
            .map(|rows| rows.len())
            .unwrap_or(0);
        assert_eq!(missed, 0);
        assert_eq!(as_post_list(envelope).unwrap().len(), 2);
    }

    #[test]
    fn ok_false_is_a_provider_error() {
        let error = unwrap_opencli_data(json!({
            "ok": false,
            "error": "OpenCLI search returned no posts"
        }))
        .unwrap_err();
        assert_eq!(
            error,
            ParseError::Provider("OpenCLI search returned no posts".into())
        );
    }

    #[test]
    fn malformed_stdout_is_not_an_empty_success() {
        assert!(matches!(
            posts_from_stdout("").unwrap_err(),
            ParseError::Malformed(_)
        ));
        assert!(matches!(
            posts_from_stdout("{").unwrap_err(),
            ParseError::Malformed(_)
        ));
        assert!(matches!(
            posts_from_stdout("not json").unwrap_err(),
            ParseError::Malformed(_)
        ));
    }

    #[test]
    fn banner_before_json_still_parses() {
        let text = "opencli: ready\n{\"ok\":true,\"data\":[]}\n";
        let posts = posts_from_stdout(text).unwrap();
        assert!(posts.is_empty());
    }

    #[test]
    fn double_encoded_data_string_unwraps() {
        let inner = sample().to_string();
        let rows = as_post_list(json!({"ok": true, "data": inner})).unwrap();
        assert_eq!(ids(&rows), ["1", "2"]);
    }

    #[test]
    fn preserves_timestamp_and_normalizes_source_url() {
        let posts = posts_from_stdout(&json!({"ok": true, "data": sample()}).to_string()).unwrap();
        assert_eq!(
            posts[0].created_at.as_deref(),
            Some("Wed Nov 11 16:00:00 +0000 2026")
        );
        assert_eq!(posts[0].url, "https://x.com/alice/status/1");
        assert_eq!(posts[0].source_url, "https://x.com/alice/status/1");
        assert_eq!(posts[1].source_url, "https://twitter.com/bob/status/2");
        assert_eq!(posts[1].url, "https://x.com/bob/status/2");
    }
}
