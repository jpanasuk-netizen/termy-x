//! "Ask here": run a coding-agent CLI (Agy, Hermes, Grok, Muse) inside one of
//! Termy's own terminal panes, paste the reply prompt there, and read the
//! answers back out of that pane's scrollback. Pure logic only; the desktop
//! app does the pane I/O. Nothing here posts anything.

use std::path::{Path, PathBuf};

use crate::config::XConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Agent {
    Agy,
    Hermes,
    Grok,
    Muse,
}

impl Agent {
    /// Picker order. Grok first: it is the primary option.
    pub const ALL: [Agent; 4] = [Agent::Grok, Agent::Agy, Agent::Hermes, Agent::Muse];

    pub fn tag(self) -> &'static str {
        match self {
            Agent::Agy => "agy",
            Agent::Hermes => "hermes",
            Agent::Grok => "grok",
            Agent::Muse => "muse",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Agent::Agy => "Agy",
            Agent::Hermes => "Hermes",
            Agent::Grok => "Grok",
            Agent::Muse => "Muse",
        }
    }

    pub fn parse(value: &str) -> Option<Agent> {
        let value = value.trim().to_ascii_lowercase();
        Agent::ALL.into_iter().find(|agent| agent.tag() == value)
    }

    /// Executable names this agent shows up as in a pane's process tree.
    pub fn process_names(self) -> &'static [&'static str] {
        match self {
            Agent::Agy => &["agy", "antigravity"],
            Agent::Hermes => &["hermes"],
            Agent::Grok => &["grok"],
            Agent::Muse => &["muse"],
        }
    }

    /// Hint shown on a greyed-out button.
    pub fn install_hint(self) -> &'static str {
        match self {
            Agent::Agy => {
                "Agy (Antigravity CLI) not found. Install it or set agents.agy in x.toml."
            }
            Agent::Hermes => {
                "Hermes not found on PATH or in WSL. Set agents.hermes in x.toml (e.g. wsl -d Ubuntu -u jpanasuk hermes)."
            }
            Agent::Grok => "Grok CLI not found. Install it or set agents.grok in x.toml.",
            Agent::Muse => "Muse CLI not found. Install it or set agents.muse in x.toml.",
        }
    }
}

/// Does a foreground process name (`grok.exe`, `/usr/bin/muse`) belong to `agent`?
pub fn process_matches(agent: Agent, process: &str) -> bool {
    let base = process
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(process)
        .to_ascii_lowercase();
    let stem = base.strip_suffix(".exe").unwrap_or(&base);
    agent.process_names().contains(&stem)
}

/// What the desktop app knows about one pane of the active tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneInfo {
    pub id: String,
    /// Newest foreground descendant, when the platform can tell.
    pub process: Option<String>,
    pub focused: bool,
    /// Termy started this pane for that agent ("Ask here" new pane).
    pub launched_for: Option<Agent>,
    /// Pane/tab title (agents often set it).
    pub title: String,
    /// Last few lines of the pane, for a banner/prompt fallback.
    pub tail: String,
}

fn has_word(haystack: &str, word: &str) -> bool {
    haystack
        .to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|token| token == word)
}

/// Which agent runs in `pane`: foreground process, then the pane Termy
/// launched for it, then the title, then the recent scrollback.
pub fn pane_agent(pane: &PaneInfo) -> Option<Agent> {
    if let Some(process) = pane.process.as_deref()
        && let Some(agent) = Agent::ALL
            .into_iter()
            .find(|agent| process_matches(*agent, process))
    {
        return Some(agent);
    }
    if pane.launched_for.is_some() {
        return pane.launched_for;
    }
    let by_word = |text: &str| {
        Agent::ALL.into_iter().find(|agent| {
            agent
                .process_names()
                .iter()
                .any(|name| has_word(text, name))
        })
    };
    by_word(&pane.title).or_else(|| by_word(&pane.tail))
}

/// Agent already running in the active tab: the focused pane wins.
pub fn detect_running(panes: &[PaneInfo]) -> Option<Agent> {
    panes
        .iter()
        .find(|pane| pane.focused)
        .and_then(pane_agent)
        .or_else(|| panes.iter().find_map(pane_agent))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanePick {
    /// The agent already runs here.
    Agent(String),
    /// No agent pane: use the focused pane (the user's own shell or agent).
    Focused(String),
    /// Open a new pane running the agent command.
    NewPane,
}

/// Pane order: one running the agent, else the focused pane, else a new pane.
pub fn pick_pane(agent: Agent, panes: &[PaneInfo]) -> PanePick {
    let runs_agent = |pane: &&PaneInfo| pane_agent(pane) == Some(agent);
    // Prefer the focused pane when it is itself the agent.
    if let Some(pane) = panes.iter().filter(runs_agent).find(|pane| pane.focused) {
        return PanePick::Agent(pane.id.clone());
    }
    if let Some(pane) = panes.iter().find(runs_agent) {
        return PanePick::Agent(pane.id.clone());
    }
    if let Some(pane) = panes.iter().find(|pane| pane.focused) {
        return PanePick::Focused(pane.id.clone());
    }
    PanePick::NewPane
}

/// Marker line closing every pasted prompt. Replies are read after the
/// last copy of it, so the prompt's own numbered rules are never parsed.
pub fn prompt_marker(nonce: u64) -> String {
    format!("[termy-x ask {nonce:x}: answer below this line]")
}

/// The prompt as pasted into the pane: the handoff prompt plus the marker.
pub fn marked_prompt(prompt: &str, nonce: u64) -> String {
    format!("{}\n\n{}", prompt.trim_end(), prompt_marker(nonce))
}

/// Text after the last occurrence of `marker` in the pane's scrollback, or
/// `None` when the marker scrolled away or was never echoed.
pub fn text_after_marker(scrollback: &str, marker: &str) -> Option<String> {
    let index = scrollback.rfind(marker)?;
    let rest = &scrollback[index + marker.len()..];
    let rest = rest.trim_matches(|c: char| c == '\n' || c == '\r');
    Some(
        rest.lines()
            // TUIs draw their transcript inside box borders.
            .map(|line| {
                line.trim()
                    .trim_matches(|c: char| matches!(c, '\u{2502}' | '\u{2503}' | '|'))
                    .trim()
            })
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// Reply options from scrollback: [`text_after_marker`] then
/// [`crate::handoff::parse_options`].
pub fn grab_options(scrollback: &str, marker: &str) -> Vec<String> {
    text_after_marker(scrollback, marker)
        .map(|text| crate::handoff::parse_options(&text))
        .unwrap_or_default()
}

/// One agent's resolved command, or why it is greyed out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCommand {
    pub agent: Agent,
    /// Shell command line to start the agent in a new pane.
    pub command: Option<String>,
    /// Where it came from: "x.toml", "PATH", "WSL", or a path.
    pub source: String,
}

impl AgentCommand {
    pub fn installed(&self) -> bool {
        self.command.is_some()
    }
}

fn quote_if_needed(path: &Path) -> String {
    let text = path.display().to_string();
    if text.contains(' ') {
        format!("\"{text}\"")
    } else {
        text
    }
}

fn on_path(names: &[&str]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd", ".bat", ""]
    } else {
        &[""]
    };
    std::env::split_paths(&path).find_map(|dir| {
        names.iter().find_map(|name| {
            exts.iter()
                .map(|ext| dir.join(format!("{name}{ext}")))
                .find(|candidate| candidate.is_file())
        })
    })
}

fn known_locations(agent: Agent) -> Vec<PathBuf> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let mut out = Vec::new();
    match agent {
        Agent::Grok => {
            if let Some(home) = &home {
                out.push(home.join(".grok").join("bin").join("grok.exe"));
                out.push(home.join(".grok").join("bin").join("grok"));
            }
        }
        Agent::Agy => {
            if let Some(local) = &local {
                out.push(local.join("agy").join("bin").join("agy.exe"));
            }
        }
        Agent::Hermes => {
            if let Some(local) = &local {
                out.push(local.join("hermes").join("bin").join("hermes.exe"));
            }
        }
        Agent::Muse => {
            if let Some(local) = &local {
                out.push(
                    local
                        .join("Programs")
                        .join("Muse Code")
                        .join("bin")
                        .join("muse.exe"),
                );
            }
        }
    }
    out
}

fn wsl_prefix(distro: &str, user: &str) -> Vec<String> {
    let mut args = vec!["-d".to_string(), distro.to_string()];
    if !user.trim().is_empty() {
        args.push("-u".into());
        args.push(user.trim().to_string());
    }
    args
}

/// `wsl -d Ubuntu -u <user> -- bash -lc 'command -v <name>'`, Windows only.
fn in_wsl(name: &str, distro: &str, user: &str) -> bool {
    if !cfg!(windows) {
        return false;
    }
    let mut command = std::process::Command::new("wsl");
    command.args(wsl_prefix(distro, user));
    command.args(["--", "bash", "-lc", &format!("command -v {name}")]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
        .output()
        .is_ok_and(|output| output.status.success() && !output.stdout.is_empty())
}

/// Resolve one agent: x.toml override, then PATH, then known install
/// paths, then WSL (Hermes/Muse). Can block briefly on WSL: call it
/// off the UI thread.
pub fn detect(config: &XConfig, agent: Agent) -> AgentCommand {
    if let Some(command) = config.agent_command(agent) {
        return AgentCommand {
            agent,
            command: Some(command.to_string()),
            source: "x.toml".into(),
        };
    }
    if let Some(path) = on_path(agent.process_names()) {
        return AgentCommand {
            agent,
            command: Some(agent.tag().to_string()),
            source: format!("PATH ({})", path.display()),
        };
    }
    if let Some(path) = known_locations(agent)
        .into_iter()
        .find(|path| path.is_file())
    {
        return AgentCommand {
            agent,
            command: Some(quote_if_needed(&path)),
            source: path.display().to_string(),
        };
    }
    if matches!(agent, Agent::Hermes | Agent::Muse) {
        let (distro, user) = (&config.wsl_distro, &config.wsl_user);
        if in_wsl(agent.tag(), distro, user) {
            return AgentCommand {
                agent,
                command: Some(format!(
                    "wsl {} -- {}",
                    wsl_prefix(distro, user).join(" "),
                    agent.tag()
                )),
                source: format!("WSL ({distro})"),
            };
        }
    }
    AgentCommand {
        agent,
        command: None,
        source: "not found".into(),
    }
}

pub fn detect_all(config: &XConfig) -> Vec<AgentCommand> {
    Agent::ALL
        .into_iter()
        .map(|agent| detect(config, agent))
        .collect()
}

fn last_path(config: &XConfig) -> PathBuf {
    config.config_dir.join("x-ask-agent.txt")
}

/// Last picked agent (Grok when never picked).
pub fn load_last(config: &XConfig) -> Agent {
    std::fs::read_to_string(last_path(config))
        .ok()
        .and_then(|text| Agent::parse(&text))
        .unwrap_or(Agent::Grok)
}

pub fn save_last(config: &XConfig, agent: Agent) -> Result<(), String> {
    std::fs::create_dir_all(&config.config_dir).map_err(|error| error.to_string())?;
    std::fs::write(last_path(config), agent.tag()).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(id: &str, process: Option<&str>, focused: bool) -> PaneInfo {
        PaneInfo {
            id: id.into(),
            process: process.map(str::to_string),
            focused,
            launched_for: None,
            title: String::new(),
            tail: String::new(),
        }
    }

    #[test]
    fn detects_running_agent_by_process_title_or_tail() {
        let panes = [
            pane("1", Some("pwsh.exe"), false),
            pane("2", Some("muse.exe"), false),
        ];
        assert_eq!(detect_running(&panes), Some(Agent::Muse));
        let mut titled = pane("1", None, true);
        titled.title = "Hermes Agent - termyx".into();
        assert_eq!(detect_running(&[titled]), Some(Agent::Hermes));
        let mut tail = pane("1", Some("pwsh.exe"), true);
        tail.tail = "Welcome to grok\n> ".into();
        assert_eq!(detect_running(&[tail]), Some(Agent::Grok));
        let plain = pane("1", Some("pwsh.exe"), true);
        assert_eq!(detect_running(&[plain]), None);
        // "grokbot" is not grok.
        let mut near = pane("1", None, true);
        near.title = "grokbot notes".into();
        assert_eq!(detect_running(&[near]), None);
    }

    #[test]
    fn focused_pane_agent_wins_detection() {
        let panes = [
            pane("1", Some("grok.exe"), false),
            pane("2", Some("agy.exe"), true),
        ];
        assert_eq!(detect_running(&panes), Some(Agent::Agy));
    }

    #[test]
    fn process_names_match_with_paths_and_exe() {
        assert!(process_matches(Agent::Grok, "grok.exe"));
        assert!(process_matches(
            Agent::Grok,
            r"C:\Users\x\.grok\bin\GROK.EXE"
        ));
        assert!(process_matches(Agent::Muse, "/usr/bin/muse"));
        assert!(process_matches(Agent::Agy, "agy.exe"));
        assert!(!process_matches(Agent::Grok, "grokbot.exe"));
        assert!(!process_matches(Agent::Hermes, "pwsh.exe"));
    }

    #[test]
    fn picks_pane_running_the_agent_first() {
        let panes = [
            pane("1", Some("pwsh.exe"), true),
            pane("2", Some("grok.exe"), false),
        ];
        assert_eq!(pick_pane(Agent::Grok, &panes), PanePick::Agent("2".into()));
    }

    #[test]
    fn prefers_focused_agent_pane_among_several() {
        let panes = [
            pane("1", Some("grok.exe"), false),
            pane("2", Some("grok.exe"), true),
        ];
        assert_eq!(pick_pane(Agent::Grok, &panes), PanePick::Agent("2".into()));
    }

    #[test]
    fn launched_pane_counts_without_process_info() {
        let mut launched = pane("3", None, false);
        launched.launched_for = Some(Agent::Hermes);
        let panes = [pane("1", None, true), launched];
        assert_eq!(
            pick_pane(Agent::Hermes, &panes),
            PanePick::Agent("3".into())
        );
        // A pane launched for another agent does not count.
        assert_eq!(
            pick_pane(Agent::Muse, &panes),
            PanePick::Focused("1".into())
        );
    }

    #[test]
    fn falls_back_to_focused_then_new_pane() {
        let panes = [pane("1", Some("pwsh.exe"), false), pane("2", None, true)];
        assert_eq!(pick_pane(Agent::Agy, &panes), PanePick::Focused("2".into()));
        let unfocused = [pane("1", Some("pwsh.exe"), false)];
        assert_eq!(pick_pane(Agent::Agy, &unfocused), PanePick::NewPane);
        assert_eq!(pick_pane(Agent::Agy, &[]), PanePick::NewPane);
    }

    #[test]
    fn scrollback_after_last_marker_only() {
        let marker = prompt_marker(0xabc);
        let prompt = marked_prompt("Write 3 replies:\n1. be short\n2. no hashtags", 0xabc);
        let scrollback = format!(
            "old stuff\n1. stale\n> {prompt}\n\n1. First reply   \n2. Second reply\n3. Third reply\n"
        );
        let after = text_after_marker(&scrollback, &marker).unwrap();
        assert!(after.starts_with("1. First reply"));
        assert!(!after.contains("no hashtags"));
        let options = grab_options(&scrollback, &marker);
        assert_eq!(options, vec!["First reply", "Second reply", "Third reply"]);
    }

    #[test]
    fn scrollback_uses_the_newest_ask() {
        let marker = prompt_marker(7);
        let text = format!("{marker}\n1. old\n2. old2\n{marker}\n1. new\n2. new2\n");
        assert_eq!(grab_options(&text, &marker), vec!["new", "new2"]);
    }

    #[test]
    fn tui_box_borders_are_stripped() {
        let marker = prompt_marker(9);
        let text = format!(
            "\u{2502} {marker} \u{2502}\n\u{2502} 1. Alpha   \u{2502}\n\u{2502} 2. Beta \u{2502}\n"
        );
        assert_eq!(grab_options(&text, &marker), vec!["Alpha", "Beta"]);
    }

    #[test]
    fn missing_marker_grabs_nothing() {
        assert_eq!(text_after_marker("1. a\n2. b", &prompt_marker(1)), None);
        assert!(grab_options("1. a\n2. b", &prompt_marker(1)).is_empty());
    }

    #[test]
    fn config_override_wins_and_last_choice_round_trips() {
        let dir = std::env::temp_dir().join(format!("termy-x-agents-{}", std::process::id()));
        let mut config = XConfig {
            config_dir: dir.clone(),
            ..XConfig::default()
        };
        config.apply_text("[agents]\nhermes = \"wsl -d Ubuntu -u jpanasuk -- hermes\"\n");
        let hermes = detect(&config, Agent::Hermes);
        assert_eq!(hermes.source, "x.toml");
        assert_eq!(
            hermes.command.as_deref(),
            Some("wsl -d Ubuntu -u jpanasuk -- hermes")
        );
        assert_eq!(load_last(&config), Agent::Grok);
        save_last(&config, Agent::Muse).unwrap();
        assert_eq!(load_last(&config), Agent::Muse);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn grok_is_first_in_the_picker() {
        assert_eq!(Agent::ALL[0], Agent::Grok);
        assert_eq!(Agent::parse(" Hermes\n"), Some(Agent::Hermes));
    }
}
