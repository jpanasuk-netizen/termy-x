use crate::config::XConfig;
use crate::drafts::{new_id, timestamp};
use crate::io::{Http, HttpRequest};
use crate::model::{Draft, Tone};

const PROMPTS: &str = include_str!("../prompts/default.toml");

/// Draft `n` variants. Never posts anything.
///
/// Uses the configured OpenAI-compatible endpoint. With nothing configured it
/// tries the free local default (Ollama) and, if that is not running, returns
/// offline template drafts labelled `template`.
pub fn draft_variants(
    config: &XConfig,
    http: &dyn Http,
    idea: &str,
    tone: Tone,
    n: usize,
) -> Result<Vec<Draft>, String> {
    let n = n.clamp(1, 6);
    match model_variants(config, http, idea, tone, n) {
        Ok(variants) => Ok(variants),
        Err(error) if config.ai_configured() => Err(error),
        Err(_) => Ok(template_variants(idea, tone, n)),
    }
}

fn model_variants(
    config: &XConfig,
    http: &dyn Http,
    idea: &str,
    tone: Tone,
    n: usize,
) -> Result<Vec<Draft>, String> {
    let (base, model) = config.ai_endpoint();
    let (system, user) = prompts_for(tone, idea, n);
    let body = serde_json::json!({
        "model": model,
        "temperature": 0.7,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user}
        ]
    })
    .to_string();
    let mut url = base.trim_end_matches('/').to_string();
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
    Ok(variants
        .into_iter()
        .take(n)
        .map(|text| draft(text, tone, "model"))
        .collect())
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

/// Offline drafts used when no model is reachable. Clearly labelled.
pub fn template_variants(idea: &str, tone: Tone, n: usize) -> Vec<Draft> {
    let idea = idea.trim().trim_end_matches('.');
    let options: Vec<String> = match tone {
        Tone::Punchy => vec![
            format!("{idea}."),
            format!("Hot take: {idea}."),
            format!("{idea}. That's it. That's the post."),
            format!("Nobody talks about this enough: {idea}."),
        ],
        Tone::Informative => vec![
            format!("Quick note on {idea}: here's what changed and why it matters."),
            format!("{idea}, in three points:\n1.\n2.\n3."),
            format!("What I learned about {idea} this week."),
        ],
        Tone::Thread => vec![
            format!("A thread on {idea} 🧵\n\n1/"),
            format!("{idea}.\n\nHere's how it works, step by step 👇"),
        ],
        Tone::Reply => vec![
            format!("Agree on {idea}. One thing I'd add:"),
            format!("Interesting take on {idea}. Have you tried the opposite?"),
        ],
    };
    (0..n)
        .map(|index| draft(options[index % options.len()].clone(), tone, "template"))
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
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            current = name.to_string();
            continue;
        }
        if current == section
            && let Some((found, value)) = line.split_once('=')
            && found.trim() == key
        {
            return Some(unquote(value.trim()));
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
    fn unconfigured_ai_tries_free_local_default_then_templates() {
        let config = XConfig::default();
        let http = MapHttp::default();
        let drafts = draft_variants(&config, &http, "ship the panel", Tone::Punchy, 3).unwrap();
        assert_eq!(drafts.len(), 3);
        assert!(drafts.iter().all(|draft| draft.source == "template"));
        assert!(
            drafts
                .iter()
                .all(|draft| draft.text.contains("ship the panel"))
        );
        let calls = http.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "one probe of the free local endpoint");
        assert!(calls[0].contains("127.0.0.1:11434/v1/chat/completions"));
    }

    #[test]
    fn configured_ai_errors_are_reported_not_hidden() {
        let config = XConfig {
            ai_base_url: "http://127.0.0.1:9/v1".into(),
            ai_model: "m".into(),
            ..Default::default()
        };
        let http = MapHttp::default();
        assert!(draft_variants(&config, &http, "idea", Tone::Punchy, 2).is_err());
    }

    #[test]
    fn configured_ai_parses_variants_and_stays_a_draft() {
        let config = XConfig {
            ai_base_url: "http://127.0.0.1:9/v1".into(),
            ai_model: "local-free".into(),
            ..Default::default()
        };
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
