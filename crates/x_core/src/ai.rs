use crate::config::XConfig;
use crate::drafts::{new_id, timestamp};
use crate::io::{Http, HttpRequest};
use crate::model::{Draft, Tone};

const PROMPTS: &str = include_str!("../prompts/default.toml");

pub fn draft_variants(
    config: &XConfig,
    http: &dyn Http,
    idea: &str,
    tone: Tone,
    n: usize,
) -> Result<Vec<Draft>, String> {
    let n = n.clamp(1, 6);
    if config.ai_configured() {
        let (system, user) = prompts_for(tone, idea, n);
        let body = serde_json::json!({
            "model": config.ai_model,
            "temperature": 0.7,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user}
            ]
        })
        .to_string();
        let mut url = config.ai_base_url.trim_end_matches('/').to_string();
        url.push_str("/chat/completions");
        let response = http.request(HttpRequest {
            method: "POST",
            url,
            body: Some(body),
            bearer: config.api_key(),
            content_type: Some("application/json".into()),
        })?;
        let text = completion_text(&response)?;
        let variants = split_variants(&text);
        if variants.is_empty() {
            return Err("the model returned an empty draft".into());
        }
        return Ok(variants
            .into_iter()
            .take(n)
            .map(|text| draft(text, tone, "model"))
            .collect());
    }
    Ok(template_variants(idea, tone, n))
}

fn draft(text: String, tone: Tone, source: &str) -> Draft {
    Draft {
        id: new_id(),
        text,
        tone: Some(tone.as_str().to_string()),
        source: source.to_string(),
        created_at: timestamp(),
        reply_to: None,
    }
}

pub fn template_variants(idea: &str, tone: Tone, n: usize) -> Vec<Draft> {
    (0..n)
        .map(|index| {
            let text = match tone {
                Tone::Punchy => format!("Unpopular draft {index}: {idea}. Still a draft."),
                Tone::Informative => {
                    format!("Notes on {idea}. Variant {index}. Nothing here has been posted.")
                }
                Tone::Thread => format!("{idea}\n\nPart {index} stays under the limit until you edit it."),
                Tone::Reply => format!("Replying on {idea} — variant {index}. Confirm before it goes anywhere."),
            };
            draft(text, tone, "template")
        })
        .collect()
}

fn prompts_for(tone: Tone, idea: &str, n: usize) -> (String, String) {
    let section = tone.as_str();
    let system = prompt_value(section, "system")
        .unwrap_or_else(|| "You write drafts. You never post.".into());
    let user = prompt_value(section, "user").unwrap_or_else(|| "Idea: {idea}".into());
    let fill = |text: String| {
        text.replace("{idea}", idea)
            .replace("{n}", &n.to_string())
            .replace("{tone}", tone.as_str())
    };
    (fill(system), fill(user))
}

fn prompt_value(section: &str, key: &str) -> Option<String> {
    let mut current = String::new();
    for line in PROMPTS.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
            current = name.to_string();
            continue;
        }
        if current == section {
            if let Some((found, value)) = line.split_once('=') {
                if found.trim() == key {
                    return Some(unquote(value.trim()));
                }
            }
        }
    }
    None
}

fn unquote(value: &str) -> String {
    value.trim_matches('"').replace("\\\"", "\"")
}

fn completion_text(body: &str) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_str(body).map_err(|error| error.to_string())?;
    value
        .pointer("/choices/0/message/content")
        .and_then(|content| content.as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "chat response had no message content".to_string())
}

fn split_variants(text: &str) -> Vec<String> {
    text.split("\n---\n")
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::MapHttp;
    #[test]
    fn unconfigured_ai_returns_labeled_templates_and_does_not_post() {
        let config = XConfig::default();
        let http = MapHttp::default();
        let drafts = draft_variants(&config, &http, "ship the panel", Tone::Punchy, 3).unwrap();
        assert_eq!(drafts.len(), 3);
        assert!(drafts.iter().all(|draft| draft.source == "template"));
        assert!(drafts.iter().all(|draft| draft.text.contains("ship the panel")));
        assert!(http.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn configured_ai_parses_variants_and_stays_a_draft() {
        let mut config = XConfig::default();
        config.ai_base_url = "http://127.0.0.1:9/v1".into();
        config.ai_model = "local-free".into();
        let http = MapHttp::default();
        http.routes.lock().unwrap().insert(
            "http://127.0.0.1:9/v1/chat/completions".into(),
            Ok(r#"{"choices":[{"message":{"content":"one\n---\ntwo\n---\nthree"}}]}"#.into()),
        );
        let drafts = draft_variants(&config, &http, "idea", Tone::Informative, 3).unwrap();
        assert_eq!(
            drafts
                .iter()
                .map(|draft| draft.text.as_str())
                .collect::<Vec<_>>(),
            vec!["one", "two", "three"]
        );
        assert!(drafts.iter().all(|draft| draft.source == "model"));
    }

    #[test]
    fn prompt_file_has_the_four_tones() {
        for tone in ["punchy", "informative", "thread", "reply"] {
            assert!(PROMPTS.contains(&format!("[{tone}]")), "{tone}");
        }
    }
}
