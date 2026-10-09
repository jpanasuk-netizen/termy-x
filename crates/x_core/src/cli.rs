use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};

use crate::ai::draft_variants;
use crate::config::{XConfig, example_config};
use crate::doctor::doctor;
use crate::drafts::{find_draft, load_drafts, new_id, timestamp, upsert_drafts};
use crate::io::{
    Clipboard, CommandRunner, Http, Opener, SystemClipboard, SystemOpener, SystemRunner, UreqHttp,
};
use crate::model::{ProviderId, Tone};
use crate::providers::Service;
use crate::publish::{
    ConfirmDecision, GateOutcome, IntentPublisher, confirm_and_publish, first_available,
};
use crate::splash::{render_splash, splash_ansi};

#[derive(Parser, Debug)]
#[command(
    name = "x",
    version,
    about = "Termy X - read and post without X API credits",
    disable_help_subcommand = true
)]
struct Cli {
    /// Print JSON for scripting
    #[arg(long, global = true)]
    json: bool,

    /// auto, opencli, twitter, mock, or official
    #[arg(long, global = true)]
    provider: Option<String>,

    /// Config directory (defaults to the Termy config dir)
    #[arg(long, global = true)]
    config_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Look up a user
    Lookup { handle: String },
    /// Read a post and its thread
    Thread { id_or_url: String },
    /// Publish text after you type yes. `--dry-run` prints the plan and never posts.
    Post {
        text: String,
        /// Print the exact parts, counts, media, and intent URLs. Never posts.
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        reply_to: Option<String>,
        /// Image paths to attach (jpg/png/gif/webp). Repeatable. Max 4.
        #[arg(long = "media", value_name = "PATH")]
        media: Vec<PathBuf>,
    },
    /// Unicode bird
    Splash,
    /// Search posts
    Search {
        query: String,
        /// Latest posts instead of top
        #[arg(long)]
        recent: bool,
        /// Top posts (the default)
        #[arg(long)]
        top: bool,
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    /// Trends, or search ranked by engagement when a free trends endpoint is missing
    Trends {
        #[arg(long)]
        place: Option<String>,
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    /// Top posts for a topic, ranked by engagement
    Popular {
        topic: String,
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    /// Home timeline
    Timeline {
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },
    /// One list of hot or upcoming posts, with an inline reply box. Nothing posts until y.
    Feed {
        /// Subject to search. Omit for the home timeline.
        #[arg(long)]
        topic: Option<String>,
        /// Rank by engagement, then newer posts
        #[arg(long)]
        hot: bool,
        /// Rank by recency, then engagement (the default)
        #[arg(long)]
        upcoming: bool,
        #[arg(long, default_value_t = 10)]
        limit: usize,
        /// Semicolon-separated keys, for example j;r;type:hello;p;y
        #[arg(long)]
        keys: Option<String>,
        /// Print the reply plan and never post
        #[arg(long)]
        dry_run: bool,
    },
    /// Ask the configured free model for draft variants. Never posts.
    Draft {
        idea: String,
        #[arg(long, short = 'n', default_value_t = 3)]
        n: usize,
        #[arg(long, default_value = "punchy")]
        tone: String,
    },
    /// Open an editor and save a draft
    Compose {
        /// Skip the editor and use this text
        #[arg(long)]
        text: Option<String>,
        #[arg(long)]
        reply_to: Option<String>,
    },
    /// Post a saved draft after you type yes
    Publish {
        draft_id: String,
        /// Print the exact text and intent URL without posting. On in unit tests.
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        #[arg(long)]
        reply_to: Option<String>,
    },
    /// Research a topic from Reddit, YouTube, and the web
    Research { topic: String },
    /// Full Termy X command guide (GUI + CLI)
    Help,
    /// Which free providers are installed. No X credits required.
    Doctor,
    /// Print the example config
    Config,
    /// Start the optional official API login. Off unless you enable it.
    Auth,
}

pub struct CliIo {
    pub config: XConfig,
    pub runner: Arc<dyn CommandRunner>,
    pub http: Arc<dyn Http>,
    pub opener: Arc<dyn Opener>,
    pub clipboard: Arc<dyn Clipboard>,
    pub confirm: ConfirmDecision,
    pub editor_text: Option<String>,
}

impl CliIo {
    pub fn live() -> Self {
        let dir = crate::config::default_config_dir();
        Self {
            config: XConfig::load(&dir),
            runner: Arc::new(SystemRunner),
            http: Arc::new(UreqHttp::default()),
            opener: Arc::new(SystemOpener),
            clipboard: Arc::new(SystemClipboard),
            confirm: ConfirmDecision::No,
            editor_text: None,
        }
    }
}

pub fn run_env() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut io = CliIo::live();
    if args.iter().any(|arg| arg == "--dry-run") {
        // The live process still asks on stdin inside execute when confirm is pending.
    }

    execute(&args, &mut io, true)
}

fn print_help_guide() {
    println!("Termy X - GUI + CLI guide");
    println!();
    println!("GUI");
    println!("  Ctrl+Shift+X          Open / close the X panel");
    println!("  Tabs                  Timeline, Search, Trends, Lookup, Compose, Research, Help");
    println!("  Compose               Click the cyan box (caret blinks). Ctrl+V pastes in-box.");
    println!("  Attach image          Section under the compose box. Up to 4 JPG/PNG/GIF/WEBP.");
    println!("                        Thumbnails + filenames. Remove one or Clear all.");
    println!("  Publish -> Post now   Confirm shows exact text + attachments. Post now uses");
    println!("                        OpenCLI (with images) when available; else browser intent.");
    println!("  Tab / Esc / Enter     Cycle controls, cancel confirm, activate focused button");
    println!();
    println!("CLI  (from the termy-x folder: .\\target\\release\\x.exe …)");
    println!("  x doctor              Session / OpenCLI / Premium limit check. Never posts.");
    println!("  x splash              Print the Termy X bird");
    println!("  x search \"query\" [--recent|--top] [--limit N]");
    println!("  x feed [--topic \"subject\"] [--hot|--upcoming] [--limit N]");
    println!("    j/k move  h hot  u upcoming  /subject search");
    println!("    r reply box  type a line  p preview  y post  esc cancel  q quit");
    println!("  x feed --dry-run --keys \"r;type:hello;p;y\"   Preview only, never posts");
    println!("  x timeline [--limit N]");
    println!("  x trends [--place NAME] [--limit N]");
    println!("  x lookup handle");
    println!("  x post \"text\"");
    println!("  x post --dry-run \"text\"          Plan only - never posts");
    println!("  x post --media pic.jpg \"text\"    Up to 4 images (repeat --media)");
    println!("  x post --reply-to ID \"text\"      Reply to a status");
    println!("  PowerShell long text:  x post --dry-run (Get-Content -Raw .\\post.txt)");
    println!();
    println!("Nothing posts until you type yes (CLI) or hit Post now after Confirm (GUI).");
    println!("Premium auto-detects (25,000 chars); otherwise 280.");
}

pub fn execute(args: &[String], io: &mut CliIo, interactive: bool) -> i32 {
    let cli =
        match Cli::try_parse_from(std::iter::once("x".to_string()).chain(args.iter().cloned())) {
            Ok(cli) => cli,
            Err(error) => {
                if error.kind() == clap::error::ErrorKind::DisplayHelp
                    || error.kind() == clap::error::ErrorKind::DisplayVersion
                {
                    if io.config.background_art {
                        let _ = writeln!(io_stdout(), "{}", splash_ansi());
                    }
                    let _ = write!(io_stdout(), "{error}");
                    return 0;
                }
                let _ = writeln!(io_stderr(), "{error}");
                return 2;
            }
        };
    if let Some(dir) = &cli.config_dir {
        io.config = XConfig::load(dir);
    }
    if let Some(provider) = &cli.provider {
        io.config.provider = if provider.eq_ignore_ascii_case("auto") {
            None
        } else {
            ProviderId::parse(provider)
        };
        if io.config.provider == Some(ProviderId::OfficialApi) {
            io.config.official_enabled = true;
        }
    }
    let json = cli.json;
    match cli.command {
        None => {
            print!("{}", splash_ansi());
            println!("Termy X. No X API credits are needed.");
            println!("Try `x help`, `x doctor`, `x splash`, or `x --help`.");
            0
        }
        Some(Cmd::Splash) => {
            print!("{}", splash_ansi());
            0
        }
        Some(Cmd::Help) => {
            print_help_guide();
            0
        }
        Some(Cmd::Doctor) => {
            let report = doctor(&io.config, io.runner.as_ref(), io.http.as_ref());
            if json {
                println!("{}", report.as_json());
            } else {
                println!("{}", report.render());
            }
            0
        }
        Some(Cmd::Config) => {
            println!("{}", example_config());
            0
        }
        Some(Cmd::Auth) => {
            if !io.config.official_enabled {
                eprintln!(
                    "The official X API is off. Set official_api.enabled = true before `x auth`."
                );
                return 1;
            }
            match crate::auth::authorize_url(
                &io.config,
                "termy-x",
                &crate::auth::generate_pkce().challenge,
            ) {
                Ok(url) => {
                    println!(
                        "Open this URL, approve access, then paste the code into `x auth` later."
                    );
                    println!("{url}");
                    println!(
                        "Scopes include tweet.read and tweet.write. Tokens go to the OS keychain or a 0600 file."
                    );
                    0
                }
                Err(error) => {
                    eprintln!("{error}");
                    1
                }
            }
        }
        Some(command) => dispatch(command, json, io, interactive),
    }
}

fn dispatch(command: Cmd, json: bool, io: &mut CliIo, interactive: bool) -> i32 {
    let service = Service::free(&io.config, io.runner.clone(), io.http.clone());
    match command {
        Cmd::Lookup { handle } => emit(
            json,
            service
                .lookup(&handle)
                .map(|served| (served.status.render(), json_value(served))),
        ),
        Cmd::Thread { id_or_url } => emit(
            json,
            service
                .thread(&id_or_url)
                .map(|served| (served.status.render(), json_value(served))),
        ),
        Cmd::Splash => {
            print!("{}", render_splash(true));
            0
        }
        Cmd::Post {
            text,
            dry_run,
            reply_to,
            media,
        } => publish_text(&text, dry_run, reply_to, media, json, io, interactive),
        Cmd::Search {
            query,
            recent,
            top: _,
            limit,
        } => {
            let limit = clamp_limit(limit, io.config.default_limit);
            emit(
                json,
                service
                    .search(&query, recent, limit)
                    .map(|served| (served.status.render(), json_value(served))),
            )
        }
        Cmd::Trends { place, limit } => emit(
            json,
            service
                .trends(
                    place.as_deref(),
                    clamp_limit(limit, io.config.default_limit),
                )
                .map(|served| (served.status.render(), json_value(served))),
        ),
        Cmd::Popular { topic, limit } => emit(
            json,
            service
                .popular(&topic, clamp_limit(limit, io.config.default_limit))
                .map(|served| (served.status.render(), json_value(served))),
        ),
        Cmd::Timeline { limit } => emit(
            json,
            service
                .timeline(clamp_limit(limit, io.config.default_limit))
                .map(|served| (served.status.render(), json_value(served))),
        ),
        Cmd::Feed {
            topic,
            hot,
            upcoming: _,
            limit,
            keys,
            dry_run,
        } => {
            let mode = if hot {
                crate::feed::RankMode::Hot
            } else {
                crate::feed::RankMode::Upcoming
            };
            let limit = clamp_limit(limit, io.config.default_limit);
            let service = Service::free(&io.config, io.runner.clone(), io.http.clone());
            crate::feed::run(
                crate::feed::RunInput {
                    topic,
                    mode,
                    limit,
                    keys,
                    dry_run,
                    interactive,
                },
                |topic, recent, limit| {
                    if let Some(topic) = topic {
                        service
                            .search(topic, recent, limit)
                            .map(|served| (served.value, served.status))
                    } else {
                        let _ = recent;
                        service
                            .timeline(limit)
                            .map(|served| (served.value, served.status))
                    }
                },
                |text, reply_to, dry| {
                    if !dry {
                        io.confirm = crate::publish::ConfirmDecision::Yes;
                    }
                    publish_text(
                        text,
                        dry,
                        Some(reply_to.to_string()),
                        Vec::new(),
                        json,
                        io,
                        false,
                    )
                },
            )
        }
        Cmd::Research { topic } => {
            let hits = service.research(&topic, io.runner.as_ref(), &io.config);
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&hits).unwrap_or_else(|_| "[]".into())
                );
            } else {
                println!("research · {topic} · no X API credits");
                for hit in &hits {
                    println!("[{}] {}", hit.source, hit.title);
                    if !hit.url.is_empty() {
                        println!("  {}", hit.url);
                    }
                    if !hit.excerpt.is_empty() {
                        println!("  {}", hit.excerpt);
                    }
                }
            }
            0
        }
        Cmd::Draft { idea, n, tone } => {
            let Some(tone) = Tone::parse(&tone) else {
                eprintln!("unknown tone `{tone}`. Use punchy, informative, thread, or reply.");
                return 2;
            };
            match draft_variants(&io.config, io.http.as_ref(), &idea, tone, n) {
                Ok(drafts) => {
                    let saved = upsert_drafts(&io.config.drafts_path(), drafts.clone());
                    if let Err(error) = &saved {
                        eprintln!("could not save drafts: {error}");
                        return 1;
                    }
                    if drafts
                        .first()
                        .is_some_and(|draft| draft.source == "template")
                    {
                        let (base, model) = io.config.ai_endpoint();
                        eprintln!(
                            "No reachable model at {base} ({model}). These are offline templates, not a model. Set {} or OPENAI_BASE_URL to a free OpenAI-compatible server (Ollama, LM Studio, Groq, or an OpenRouter :free model). Nothing was posted.",
                            io.config.ai_base_url_env
                        );
                    }
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&drafts).unwrap_or_else(|_| "[]".into())
                        );
                    } else {
                        for draft in &drafts {
                            println!(
                                "draft {} · {} · {}",
                                draft.id,
                                draft.source,
                                draft.tone.as_deref().unwrap_or("-")
                            );
                            println!("{}", draft.text);
                            println!("---");
                        }
                    }
                    0
                }
                Err(error) => {
                    eprintln!("{error}");
                    1
                }
            }
        }
        Cmd::Compose { text, reply_to } => {
            let body = if let Some(text) = text.or_else(|| io.editor_text.clone()) {
                text
            } else if interactive {
                match edit_with_editor("") {
                    Ok(text) => text,
                    Err(error) => {
                        eprintln!("{error}");
                        return 1;
                    }
                }
            } else {
                eprintln!("compose needs --text or an editor");
                return 2;
            };
            let draft = crate::model::Draft {
                id: new_id(),
                text: body.trim().to_string(),
                tone: None,
                source: "compose".into(),
                created_at: timestamp(),
                reply_to,
            };
            if draft.text.is_empty() {
                eprintln!("empty draft discarded");
                return 1;
            }
            if let Err(error) = upsert_drafts(&io.config.drafts_path(), vec![draft.clone()]) {
                eprintln!("{error}");
                return 1;
            }
            println!("saved draft {}", draft.id);
            println!("{}", draft.text);
            0
        }
        Cmd::Publish {
            draft_id,
            dry_run,
            reply_to,
        } => publish_draft(&draft_id, dry_run, reply_to, json, io, interactive),
        Cmd::Help | Cmd::Doctor | Cmd::Config | Cmd::Auth => 0,
    }
}

fn publish_draft(
    draft_id: &str,
    dry_run: bool,
    reply_to: Option<String>,
    json: bool,
    io: &mut CliIo,
    interactive: bool,
) -> i32 {
    let drafts = load_drafts(&io.config.drafts_path());
    let Some(draft) = find_draft(&drafts, draft_id) else {
        eprintln!("no draft `{draft_id}`");
        return 1;
    };
    let reply = reply_to.or_else(|| draft.reply_to.clone());
    publish_text(
        &draft.text,
        dry_run,
        reply,
        Vec::new(),
        json,
        io,
        interactive,
    )
}

fn publish_text(
    text: &str,
    dry_run: bool,
    reply_to: Option<String>,
    media: Vec<PathBuf>,
    json: bool,
    io: &mut CliIo,
    interactive: bool,
) -> i32 {
    let service = Service::free(&io.config, io.runner.clone(), io.http.clone());
    let mut publishers = service.publishers;
    publishers.push(Box::new(IntentPublisher {
        open_browser: false,
        opener: io.opener.clone(),
    }));
    let Some(publisher) = first_available(publishers.as_slice()) else {
        eprintln!("no publish path");
        return 1;
    };
    if let Err(error) = validate_media_paths(&media) {
        eprintln!("{error}");
        return 1;
    }
    let resolved = crate::premium::resolve_char_limit(&io.config, io.runner.as_ref());
    let plan = crate::publish::build_plan_with_media(
        text,
        reply_to.as_deref(),
        publisher,
        dry_run,
        resolved.limit,
        &media,
    );
    print_plan(&plan);
    if dry_run {
        if json {
            println!("{}", plan_json(&plan, true));
        } else {
            println!(
                "dry run — would use {} — nothing was posted",
                plan.provider.label()
            );
        }
        return 0;
    }
    let decision = if interactive {
        ask_confirm()
    } else {
        io.confirm
    };
    if decision != ConfirmDecision::Yes {
        eprintln!("not posted");
        return 1;
    }
    if publisher.id() == crate::model::ProviderId::WebIntent {
        return deliver_intent(&plan, io, interactive);
    }
    let outcome = confirm_and_publish(text, reply_to.as_deref(), decision, false, publisher);
    match outcome {
        GateOutcome::Rejected { plan } => {
            if let Some(error) = &plan.session {
                eprintln!("{error}");
            }
            eprintln!("not posted");
            1
        }
        GateOutcome::DryRun { plan } => {
            println!("{}", plan_json(&plan, true));
            0
        }
        GateOutcome::Sent { plan, receipt } => {
            if json {
                println!("{}", plan_json(&plan, false));
            } else {
                println!("{receipt}");
            }
            0
        }
    }
}

fn print_plan(plan: &crate::publish::PublishPlan) {
    println!("{}", plan.provider.label());
    if let Some(session) = &plan.session {
        println!("{session}");
    }
    println!(
        "{} weighted · {} part(s) · limit {}",
        plan.count.weighted,
        plan.parts.len(),
        plan.count.limit
    );

    if !plan.media.is_empty() {
        println!("media · {} file(s):", plan.media.len());
        for path in &plan.media {
            println!("  - {}", path.display());
        }
        if plan.provider == crate::model::ProviderId::OpenCli {
            println!("OpenCLI will upload via --images");
        } else {
            println!("this provider cannot upload files; browser attach required if you continue");
        }
    }
    for (index, part) in plan.parts.iter().enumerate() {
        let count = crate::text::weighted_len_limited(part, plan.count.limit);
        println!(
            "--- part {}/{} · {} weighted ---",
            index + 1,
            plan.parts.len(),
            count.weighted
        );
        println!("{part}");
        if let Some(url) = plan.intent_urls.get(index) {
            println!("{url}");
        }
    }
}

fn deliver_intent(plan: &crate::publish::PublishPlan, io: &CliIo, interactive: bool) -> i32 {
    for (index, part) in plan.parts.iter().enumerate() {
        match io.clipboard.copy(part) {
            Ok(()) => println!("copied part {} to the clipboard", index + 1),
            Err(error) => eprintln!(
                "could not copy part {} to the clipboard: {error}",
                index + 1
            ),
        }
        let Some(url) = plan.intent_urls.get(index) else {
            continue;
        };
        if let Err(error) = io.opener.open(url) {
            eprintln!("could not open the intent URL: {error}");
            return 1;
        }
        if interactive && index + 1 < plan.parts.len() {
            print!("next: ");
            let _ = io::stdout().flush();
            let mut line = String::new();
            if io::stdin().read_line(&mut line).is_err() {
                eprintln!("stopped before the remaining parts");
                return 1;
            }
            if matches!(line.trim(), "q" | "no" | "n") {
                eprintln!("stopped before the remaining parts");
                return 1;
            }
        }
    }
    println!(
        "opened {} web intent(s). Each part is on the clipboard when its window opens. Click Post yourself.",
        plan.parts.len()
    );
    0
}

fn ask_confirm() -> ConfirmDecision {
    print!("Type yes to post this exact text: ");
    let _ = io::stdout().flush();
    let mut line = String::new();
    if io::stdin().read_line(&mut line).is_err() {
        return ConfirmDecision::No;
    }
    if line.trim() == "yes" {
        ConfirmDecision::Yes
    } else {
        ConfirmDecision::No
    }
}

fn plan_json(plan: &crate::publish::PublishPlan, dry_run: bool) -> String {
    serde_json::json!({
        "ok": true,
        "dry_run": dry_run,
        "provider": plan.provider.as_str(),
        "text": plan.text,
        "weighted": plan.count.weighted,
        "parts": plan.parts,
        "intent_urls": plan.intent_urls,
        "media": plan.media.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
    })
    .to_string()
}

fn emit<T: serde::Serialize>(json: bool, result: Result<(String, T), String>) -> i32 {
    match result {
        Ok((status, value)) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "ok": true,
                        "status": status,
                        "credits": "none",
                        "data": value,
                    })
                );
            } else {
                println!("{status}");
                println!(
                    "{}",
                    serde_json::to_string_pretty(&value).unwrap_or_default()
                );
            }
            0
        }
        Err(error) => {
            if json {
                println!("{}", serde_json::json!({"ok": false, "error": error}));
            } else {
                eprintln!("{error}");
            }
            1
        }
    }
}

fn json_value<T: serde::Serialize>(served: crate::model::Served<T>) -> serde_json::Value {
    serde_json::json!({
        "provider": served.status.provider.as_str(),
        "session": served.status.session,
        "credits": served.status.credits,
        "note": served.status.note,
        "value": served.value,
    })
}

fn clamp_limit(limit: usize, default_limit: usize) -> usize {
    let limit = if limit == 0 { default_limit } else { limit };
    limit.clamp(1, 20)
}

fn edit_with_editor(initial: &str) -> Result<String, String> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "nano".into());
    let path = std::env::temp_dir().join(format!("termy-x-{}.txt", new_id()));
    std::fs::write(&path, initial).map_err(|error| error.to_string())?;
    let status = std::process::Command::new(&editor)
        .arg(&path)
        .status()
        .map_err(|error| format!("failed to launch {editor}: {error}"))?;
    if !status.success() {
        return Err(format!("{editor} exited {status}"));
    }
    std::fs::read_to_string(&path).map_err(|error| error.to_string())
}

fn io_stdout() -> io::Stdout {
    io::stdout()
}

fn io_stderr() -> io::Stderr {
    io::stderr()
}

fn validate_media_paths(media: &[PathBuf]) -> Result<(), String> {
    if media.len() > 4 {
        return Err("at most 4 images per post".into());
    }
    for path in media {
        if !path.is_file() {
            return Err(format!("media not found: {}", path.display()));
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "gif" | "webp") {
            return Err(format!(
                "unsupported media {}: use jpg/png/gif/webp",
                path.display()
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::{RecordingOpener, ScriptedRunner};
    use crate::publish::ConfirmDecision;
    fn test_io(dir: &std::path::Path) -> CliIo {
        CliIo {
            config: XConfig {
                config_dir: dir.to_path_buf(),
                min_interval_ms: 0,
                provider: Some(ProviderId::Mock),
                ..XConfig::default()
            },
            runner: Arc::new(ScriptedRunner::new()),
            http: Arc::new(crate::io::MapHttp::default()),
            opener: Arc::new(RecordingOpener::default()),
            clipboard: Arc::new(crate::io::RecordingClipboard::default()),
            confirm: ConfirmDecision::Yes,
            editor_text: None,
        }
    }

    #[test]
    fn mock_commands_run_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let mut io = test_io(dir.path());
        assert_eq!(execute(&["doctor".into()], &mut io, false), 0);
        assert_eq!(
            execute(&["timeline".into(), "--json".into()], &mut io, false),
            0
        );
        assert_eq!(
            execute(
                &["lookup".into(), "@termy".into(), "--json".into()],
                &mut io,
                false
            ),
            0
        );
        assert_eq!(
            execute(&["thread".into(), "1001".into()], &mut io, false),
            0
        );
        assert_eq!(execute(&["splash".into()], &mut io, false), 0);
        assert_eq!(
            execute(
                &["post".into(), "--dry-run".into(), "hello world".into()],
                &mut io,
                false
            ),
            0
        );
        assert_eq!(
            execute(
                &[
                    "search".into(),
                    "panel".into(),
                    "--recent".into(),
                    "--limit".into(),
                    "2".into(),
                    "--json".into()
                ],
                &mut io,
                false
            ),
            0
        );
        assert_eq!(
            execute(
                &["trends".into(), "--place".into(), "nyc".into()],
                &mut io,
                false
            ),
            0
        );
        assert_eq!(
            execute(&["popular".into(), "gpui".into()], &mut io, false),
            0
        );
        assert_eq!(
            execute(
                &[
                    "feed".into(),
                    "--topic".into(),
                    "generator sizing".into(),
                    "--dry-run".into(),
                    "--keys".into(),
                    "j;r;type:hello from the terminal;p;y".into(),
                ],
                &mut io,
                false
            ),
            0
        );
        assert_eq!(
            execute(
                &[
                    "feed".into(),
                    "--dry-run".into(),
                    "--keys".into(),
                    "r;type:not yet;y".into(),
                ],
                &mut io,
                false
            ),
            0
        );
        assert_eq!(
            execute(
                &[
                    "draft".into(),
                    "ship the panel".into(),
                    "--n".into(),
                    "2".into(),
                    "--tone".into(),
                    "punchy".into()
                ],
                &mut io,
                false
            ),
            0
        );
        let drafts = load_drafts(&io.config.drafts_path());
        assert_eq!(drafts.len(), 2);
        io.confirm = ConfirmDecision::No;
        assert_eq!(
            execute(
                &["publish".into(), drafts[0].id.clone(), "--dry-run".into()],
                &mut io,
                false
            ),
            0
        );
        io.confirm = ConfirmDecision::Yes;
        assert_eq!(
            execute(&["publish".into(), drafts[0].id.clone()], &mut io, false),
            0
        );
        assert_eq!(
            execute(
                &[
                    "compose".into(),
                    "--text".into(),
                    "hello from compose".into()
                ],
                &mut io,
                false
            ),
            0
        );
        assert!(
            load_drafts(&io.config.drafts_path())
                .iter()
                .any(|draft| draft.text == "hello from compose")
        );
    }
}
