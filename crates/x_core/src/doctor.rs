use crate::config::XConfig;
use crate::io::CommandRunner;
use crate::model::SESSION_OWN;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorReport {
    pub lines: Vec<String>,
}

impl DoctorReport {
    pub fn render(&self) -> String {
        self.lines.join("\n")
    }

    pub fn as_json(&self) -> String {
        serde_json::json!({
            "ok": true,
            "credits": "none",
            "lines": self.lines,
        })
        .to_string()
    }
}

pub fn doctor(config: &XConfig, runner: &dyn CommandRunner) -> DoctorReport {
    let opencli = runner.exists(&config.opencli_bin);
    let twitter = runner.exists(&config.twitter_bin);
    let agent = runner.exists(&config.agent_reach_bin);
    let youtube = runner.exists(&config.youtube_bin);
    let curl = runner.exists("curl");
    let mut lines = vec![
        "Termy X doctor".into(),
        "No X API credits are required. The official API is off unless you set official_api.enabled = true.".into(),
        String::new(),
        "Free read path (first available wins):".into(),
        probe("opencli", opencli, &format!("{SESSION_OWN} (Chrome)")),
        probe("twitter-cli", twitter, SESSION_OWN),
        probe(
            "agent-reach",
            agent,
            "optional router; X reads still go through twitter-cli",
        ),
        probe("mock", true, "fixture data, always available"),
        String::new(),
        "Free publish path:".into(),
        probe("opencli twitter post", opencli, "OpenCLI supports post and reply"),
        probe("twitter post", twitter, "twitter-cli supports post and reply"),
        probe(
            "X web intent",
            true,
            "always available — you click Post in the browser",
        ),
        String::new(),
        "Research (feeds AI drafts, not X credits):".into(),
        probe("opencli reddit", opencli, SESSION_OWN),
        probe("yt-dlp", youtube, "public captions and search"),
        probe("curl + Jina Reader", curl, "https://r.jina.ai/"),
        String::new(),
        format!(
            "Official X API: {}",
            if config.official_enabled {
                "enabled — reads and writes can spend credits"
            } else {
                "disabled (no credits needed)"
            }
        ),
        format!(
            "AI drafts: {}",
            if config.ai_configured() {
                format!("{} · model {}", config.ai_base_url, config.ai_model)
            } else {
                format!(
                    "not configured. Set {} to a free OpenAI-compatible endpoint (Ollama http://127.0.0.1:11434/v1, Groq, or an OpenRouter :free model). No paid default.",
                    config.ai_base_url_env
                )
            }
        ),
        format!(
            "Background art: {}",
            if config.background_art { "on" } else { "off" }
        ),
        format!("Config directory: {}", config.config_dir.display()),
    ];
    if !opencli && !twitter {
        lines.push(
            "Neither free reader is installed. Commands still run on mock fixtures, and publish falls back to the web intent."
                .into(),
        );
    }
    DoctorReport { lines }
}

fn probe(name: &str, ready: bool, detail: &str) -> String {
    format!(
        "  {name:<22} {}  {detail}",
        if ready { "READY  " } else { "MISSING" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::ScriptedRunner;

    #[test]
    fn doctor_says_credits_are_not_required() {
        let config = XConfig::default();
        let report = doctor(&config, &ScriptedRunner::new());
        let text = report.render();
        assert!(text.contains("No X API credits are required"));
        assert!(text.contains("disabled (no credits needed)"));
        assert!(text.contains(SESSION_OWN));
        assert!(text.contains("web intent"));
        assert!(text.contains("Background art: on"));
        assert!(!config.official_enabled);
    }
}
