use crate::config::XConfig;
use crate::io::{CommandRunner, Http, HttpRequest};
use crate::model::SESSION_OWN;
use crate::providers::help_lists_command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Probe {
    Working,
    Missing,
    NotLoggedIn,
}

impl Probe {
    fn as_str(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Missing => "missing",
            Self::NotLoggedIn => "not logged in",
        }
    }
}

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

/// Read-only status. This never posts and never starts a login flow.
pub fn doctor(config: &XConfig, runner: &dyn CommandRunner, http: &dyn Http) -> DoctorReport {
    let opencli = shell_probe(
        runner,
        &config.opencli_bin,
        &[
            "twitter".into(),
            "whoami".into(),
            "-f".into(),
            "json".into(),
            "--window".into(),
            "background".into(),
        ],
        &[
            "twitter".into(),
            "--help".into(),
        ],
    );
    let twitter = shell_probe(
        runner,
        &config.twitter_bin,
        &["status".into(), "--json".into()],
        &["--help".into()],
    );
    let mut lines = vec![
        "Termy X doctor".into(),
        "No X API credits are needed.".into(),
        format!(
            "Official X API: {}",
            if config.official_enabled { "on" } else { "off" }
        ),
        String::new(),
        probe_line("OpenCLI", opencli.0, &format!("{SESSION_OWN} · {}", opencli.1)),
        probe_line(
            "twitter-cli / Agent Reach",
            twitter.0,
            &format!("{SESSION_OWN} · {}", twitter.1),
        ),
        probe_line("mock", Probe::Working, "fixture data, always available"),
        probe_line(
            "intent",
            Probe::Working,
            "https://x.com/intent/post — always available",
        ),
        String::new(),
        ai_line(config, http),
        format!(
            "Background art: {} · opacity {:.2}",
            if config.background_art { "on" } else { "off" },
            config.background_opacity
        ),
        format!(
            "Post length: {} weighted (Premium long-form is 25000; free tier 280). Override with char_limit or TERMY_X_CHAR_LIMIT.",
            config.char_limit_or_fallback()
        ),
        format!("Config directory: {}", config.config_dir.display()),
    ];
    if opencli.0 == Probe::Missing && twitter.0 == Probe::Missing {
        lines.push(
            "Neither free reader is installed. Reads still use mock fixtures, and publish uses the web intent."
                .into(),
        );
    }
    DoctorReport { lines }
}

fn probe_line(name: &str, probe: Probe, detail: &str) -> String {
    format!("{name}: {} · {detail}", probe.as_str())
}

fn shell_probe(
    runner: &dyn CommandRunner,
    bin: &str,
    session_args: &[String],
    help_args: &[String],
) -> (Probe, String) {
    if !runner.exists(bin) {
        return (Probe::Missing, "not on PATH".into());
    }
    let posting = match runner.run(bin, help_args) {
        Ok(output) if help_lists_command(&format!("{}\n{}", output.stdout, output.stderr), "post") => {
            "posting supported"
        }
        _ => "posting not supported",
    };
    let session = match runner.run(bin, session_args) {
        Ok(output) if output.status == 0 && looks_logged_in(&output.stdout) => Probe::Working,
        _ => Probe::NotLoggedIn,
    };
    (session, posting.into())
}

fn looks_logged_in(stdout: &str) -> bool {
    let compact = stdout.to_ascii_lowercase().replace(' ', "");
    if compact.contains("\"logged_in\":false")
        || compact.contains("\"authenticated\":false")
        || compact.contains("\"ok\":false")
        || compact.contains("not_authenticated")
        || compact.contains("notlogged")
    {
        return false;
    }
    compact.contains("\"logged_in\":true") || compact.contains("\"authenticated\":true")
}

fn ai_line(config: &XConfig, http: &dyn Http) -> String {
    let (base, model) = config.ai_endpoint();
    let url = format!("{}/models", base.trim_end_matches('/'));
    let reachable = match http.request(HttpRequest {
        method: "GET",
        url,
        body: None,
        bearer: config.api_key(),
        content_type: None,
    }) {
        Ok(_) => true,
        Err(error) => error.starts_with("http "),
    };
    if reachable {
        format!("AI endpoint: working · {model} · {base}")
    } else if config.ai_configured() {
        format!("AI endpoint: not reachable · {model} · {base}")
    } else {
        format!(
            "AI endpoint: not reachable · free default {base} ({model}) · offline templates will be used"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::{MapHttp, ScriptedRunner};

    #[test]
    fn doctor_names_each_free_provider_and_needs_no_credits() {
        let config = XConfig::default();
        let runner = ScriptedRunner::new();
        let report = doctor(&config, &runner, &MapHttp::default());
        let text = report.render();
        assert!(text.contains("No X API credits are needed."));
        assert!(text.contains("Official X API: off"));
        assert!(text.contains("OpenCLI: missing"));
        assert!(text.contains("twitter-cli / Agent Reach: missing"));
        assert!(text.contains("mock: working"));
        assert!(text.contains("intent: working"));
        assert!(text.contains("offline templates"));
        assert!(text.contains("Background art: on"));
        assert!(!config.official_enabled);
        assert!(runner.calls.lock().unwrap().is_empty());
            assert!(text.contains("Post length:"), "{text}");
}

    #[test]
    fn doctor_probes_are_read_only() {
        let config = XConfig::default();
        let runner = ScriptedRunner::new();
        runner.script(
            "opencli",
            &["twitter", "--help"],
            "post <text>\nprofile\nsearch\n",
        );
        runner.script(
            "opencli",
            &["twitter", "whoami"],
            r#"{"logged_in": true, "username": "ada"}"#,
        );
        runner.script("twitter", &["--help"], "  post        Post a new tweet.\n");
        runner.script(
            "twitter",
            &["status", "--json"],
            r#"{"ok": true, "data": {"authenticated": false}}"#,
        );
        let text = doctor(&config, &runner, &MapHttp::default()).render();
        assert!(text.contains("OpenCLI: working"));
        assert!(text.contains("twitter-cli / Agent Reach: not logged in"));
        let calls = runner.calls.lock().unwrap().clone();
        assert!(!calls.is_empty());
        assert!(calls.iter().all(|(_, args)| {
            !args
                .iter()
                .any(|arg| arg == "post" || arg == "reply" || arg == "login")
        }));
    }

    #[test]
    fn a_responding_model_server_counts_as_working() {
        let mut config = XConfig::default();
        config.ai_base_url = "http://127.0.0.1:9/v1".into();
        config.ai_model = "local".into();
        config.ai_explicit = true;
        let http = MapHttp::default();
        http.routes.lock().unwrap().insert(
            "http://127.0.0.1:9/v1/models".into(),
            Err("http 401: unauthorized".into()),
        );
        let text = doctor(&config, &ScriptedRunner::new(), &http).render();
        assert!(text.contains("AI endpoint: working · local"));
    }
}
