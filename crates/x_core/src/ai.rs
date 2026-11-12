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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatModelChoice {
    pub model: String,
    /// Models still unused, local names before `:cloud` names.
    pub later: Vec<String>,
    pub substituted: bool,
}

/// Pick a chat model on the configured endpoint.
///
/// An explicit `ai.model` is used as written. The implicit default (`llama3.2`)
/// is kept only when the server lists it. Otherwise the first listed model is
/// used. A dead server is `free model unreachable at <host>`. An empty catalog
/// is `model <name> not installed`.
pub fn choose_chat_model(config: &XConfig, http: &dyn Http) -> Result<ChatModelChoice, String> {
    let (base, preferred) = config.ai_endpoint();
    let host = endpoint_host(&base);
    if !config.ai_model.trim().is_empty() {
        return Ok(ChatModelChoice {
            model: preferred,
            later: Vec::new(),
            substituted: false,
        });
    }
    let Ok(models) = list_model_ids(http, &base) else {
        return Err(format!("free model unreachable at {host}"));
    };
    let ordered = prefer_local_models(models, &preferred);
    let Some(model) = ordered.first().cloned() else {
        return Err(format!("model {preferred} not installed"));
    };
    Ok(ChatModelChoice {
        substituted: !model_matches(&model, &preferred),
        later: ordered.into_iter().skip(1).collect(),
        model,
    })
}

/// Local names first. A `:cloud` model is used only when nothing local is listed.
pub fn prefer_local_models(models: Vec<String>, preferred: &str) -> Vec<String> {
    let mut local = Vec::new();
    let mut cloud = Vec::new();
    for name in models {
        let name = name.trim().to_string();
        if name.is_empty() {
            continue;
        }
        if is_cloud_model(&name) {
            cloud.push(name);
        } else {
            local.push(name);
        }
    }
    let mut ordered = Vec::new();
    if let Some(index) = local.iter().position(|name| model_matches(name, preferred)) {
        ordered.push(local.remove(index));
    }
    ordered.append(&mut local);
    if ordered.is_empty()
        && let Some(index) = cloud.iter().position(|name| model_matches(name, preferred))
    {
        ordered.push(cloud.remove(index));
    }
    ordered.append(&mut cloud);
    ordered
}

pub fn is_cloud_model(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.contains(":cloud") || name.ends_with("-cloud")
}

/// A paid or unauthorized model. The caller may try one other model, then stop.
pub fn blocks_free_model(note: &str) -> bool {
    let note = note.to_lowercase();
    note.contains("402")
        || note.contains("401")
        || note.contains("payment required")
        || note.contains("insufficient")
        || note.contains("unauthorized")
}

fn endpoint_host(base: &str) -> String {
    let trimmed = base.trim().trim_end_matches('/');
    let without_scheme = trimmed.split("://").nth(1).unwrap_or(trimmed);
    without_scheme
        .split('/')
        .next()
        .unwrap_or(without_scheme)
        .to_string()
}

fn model_matches(installed: &str, wanted: &str) -> bool {
    let installed = installed.trim();
    let wanted = wanted.trim();
    installed.eq_ignore_ascii_case(wanted)
        || installed
            .split(':')
            .next()
            .is_some_and(|name| name.eq_ignore_ascii_case(wanted))
}

fn list_model_ids(http: &dyn Http, base: &str) -> Result<Vec<String>, String> {
    let mut saw_answer = false;
    let openai = format!("{}/models", base.trim().trim_end_matches('/'));
    if let Ok(body) = http.request(get_json(&openai)) {
        saw_answer = true;
        let ids = parse_openai_models(&body);
        if !ids.is_empty() {
            return Ok(ids);
        }
    }
    let host = endpoint_host(base);
    if host.contains("11434") {
        let tags = format!("http://{host}/api/tags");
        if let Ok(body) = http.request(get_json(&tags)) {
            saw_answer = true;
            let ids = parse_ollama_tags(&body);
            if !ids.is_empty() {
                return Ok(ids);
            }
        }
    }
    if saw_answer {
        Ok(Vec::new())
    } else {
        Err(format!("free model unreachable at {host}"))
    }
}

fn get_json(url: &str) -> HttpRequest {
    HttpRequest {
        method: "GET",
        url: url.to_string(),
        body: None,
        bearer: None,
        content_type: None,
        headers: Vec::new(),
    }
}

fn parse_openai_models(body: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    value
        .pointer("/data")
        .and_then(|item| item.as_array())
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row.get("id").and_then(|id| id.as_str()))
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn parse_ollama_tags(body: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    value
        .pointer("/models")
        .and_then(|item| item.as_array())
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row.get("name").and_then(|name| name.as_str()))
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// One completion from the configured free endpoint. No template fallback.
pub fn chat_completion(
    config: &XConfig,
    http: &dyn Http,
    system: &str,
    user: &str,
) -> Result<String, String> {
    let (base, model) = config.ai_endpoint();
    let body = serde_json::json!({
        "model": model,
        "temperature": 0.4,
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
        headers: Vec::new(),
    })?;
    let text = completion_text(&response)?;
    if let Some(key) = config.api_key() {
        return Ok(text.replace(&key, "[redacted]"));
    }
    Ok(text)
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
        headers: Vec::new(),
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
    fn default_ollama_uses_the_first_installed_model_when_llama_is_missing() {
        let config = XConfig::default();
        let http = MapHttp::default();
        http.routes.lock().unwrap().insert(
            "http://127.0.0.1:11434/v1/models".into(),
            Ok(r#"{"data":[{"id":"qwen2.5:latest"}]}"#.into()),
        );
        let choice = choose_chat_model(&config, &http).unwrap();
        assert!(choice.substituted);
        assert_eq!(choice.model, "qwen2.5:latest");
    }

    #[test]
    fn a_local_model_is_chosen_ahead_of_cloud_models() {
        let config = XConfig::default();
        let http = MapHttp::default();
        http.routes.lock().unwrap().insert(
            "http://127.0.0.1:11434/v1/models".into(),
            Ok(
                r#"{"data":[{"id":"minimax-m3:cloud"},{"id":"llama3.2"},{"id":"glm-5.2:cloud"}]}"#
                    .into(),
            ),
        );
        let choice = choose_chat_model(&config, &http).unwrap();
        assert_eq!(choice.model, "llama3.2");
        assert!(choice.later.iter().all(|name| is_cloud_model(name)));
    }

    #[test]
    fn only_cloud_models_leave_one_backup() {
        let config = XConfig::default();
        let http = MapHttp::default();
        http.routes.lock().unwrap().insert(
            "http://127.0.0.1:11434/v1/models".into(),
            Ok(r#"{"data":[{"id":"minimax-m3:cloud"},{"id":"glm-5.2:cloud"}]}"#.into()),
        );
        let choice = choose_chat_model(&config, &http).unwrap();
        assert_eq!(choice.model, "minimax-m3:cloud");
        assert_eq!(choice.later, vec!["glm-5.2:cloud".to_string()]);
        assert!(blocks_free_model("http 402: payment required"));
        assert!(!blocks_free_model("connection refused"));
    }

    #[test]
    fn a_dead_local_endpoint_names_the_host() {
        let config = XConfig::default();
        let http = MapHttp::default();
        let error = choose_chat_model(&config, &http).unwrap_err();
        assert_eq!(error, "free model unreachable at 127.0.0.1:11434");
    }

    #[test]
    fn an_empty_catalog_says_the_default_model_is_not_installed() {
        let config = XConfig::default();
        let http = MapHttp::default();
        http.routes.lock().unwrap().insert(
            "http://127.0.0.1:11434/v1/models".into(),
            Ok(r#"{"data":[]}"#.into()),
        );
        http.routes.lock().unwrap().insert(
            "http://127.0.0.1:11434/api/tags".into(),
            Ok(r#"{"models":[]}"#.into()),
        );
        let error = choose_chat_model(&config, &http).unwrap_err();
        assert_eq!(error, "model llama3.2 not installed");
    }

    #[test]
    fn llama_with_a_tag_still_counts_as_installed() {
        let config = XConfig::default();
        let http = MapHttp::default();
        http.routes.lock().unwrap().insert(
            "http://127.0.0.1:11434/api/tags".into(),
            Ok(r#"{"models":[{"name":"llama3.2:latest"}]}"#.into()),
        );
        let choice = choose_chat_model(&config, &http).unwrap();
        assert!(!choice.substituted);
        assert_eq!(choice.model, "llama3.2:latest");
    }

    fn prompt_file_has_the_four_tones() {
        for tone in ["punchy", "informative", "thread", "reply"] {
            assert!(PROMPTS.contains(&format!("[{tone}]")), "{tone}");
        }
    }
}
