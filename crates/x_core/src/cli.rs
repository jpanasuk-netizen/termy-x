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
use crate::publish::{ConfirmDecision, GateOutcome, IntentPublisher, first_available};
use crate::splash::{render_splash, splash_ansi};

#[derive(Parser, Debug)]
#[command(
    name = "termy-xcli",
    version,
    about = "Termy X - read and post without X API credits",
    after_help = "The program file is termy-xcli.exe. Do not run a file named x.exe.\nExamples:\n  termy-xcli doctor\n  termy-xcli connect --check\n  termy-xcli write quote --to ID --inspect",
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
    /// Print saved like, follow, and retweet rows. Read-only. No tokens.
    Actions,
    /// Look up a user
    Lookup { handle: String },
    /// Profile totals from OpenCLI plus sums over the loaded tweet window
    Stats {
        /// Handle to read. Omit to use the connected account.
        handle: Option<String>,
    },
    /// Read a post and its thread
    Thread { id_or_url: String },
    /// One panel write. Dry-run unless `--confirm yes`. Never bulk, never a retry.
    Write {
        #[command(subcommand)]
        op: WriteOp,
        /// The only approval that performs the write. Must be the word yes.
        #[arg(long, global = true)]
        confirm: Option<String>,
    },
    /// Publish text after you type yes. `--dry-run` prints the plan and never posts.
    Post {
        text: String,
        /// Print the exact parts, counts, media, and intent URLs. Never posts.
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        reply_to: Option<String>,
        /// Image paths to attach (jpg/png/gif/webp). Repeatable. Max 4.
        #[arg(long = "media", alias = "image", value_name = "PATH")]
        media: Vec<PathBuf>,
    },
    /// Unicode bird
    Splash,
    /// Search posts
    Search {
        query: String,
        /// Latest posts instead of popular
        #[arg(long, alias = "newest")]
        recent: bool,
        /// Popular posts (the default)
        #[arg(long, alias = "popular")]
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
        /// Rank by likes, then engagement (Popular)
        #[arg(long, alias = "popular")]
        hot: bool,
        /// Rank by created_at descending (Newest)
        #[arg(long, alias = "newest")]
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
    /// Connect your own X account (OpenCLI session, or your own X developer token)
    Connect {
        /// Check the connection with a read-only who-are-you call. Never posts.
        #[arg(long)]
        check: bool,
        /// Forget the saved developer key and remembered account on this machine.
        #[arg(long)]
        disconnect: bool,
        /// Paste your own X developer token (read from stdin, never echoed
        /// back or written to a config file; stored in your OS keychain).
        #[arg(long)]
        save_token: bool,
    },
    /// Replies and mentions that still need a response
    Inbox {
        #[command(subcommand)]
        action: Option<InboxAction>,
        #[arg(long, default_value_t = 20, global = true)]
        limit: usize,
        #[arg(long, global = true)]
        handle: Option<String>,
    },
    /// Posts that mention you, newest first
    Mentions {
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        handle: Option<String>,
    },
    /// Keyword/topic feeds with local language, age, and mute filters
    Feeds {
        #[command(subcommand)]
        action: Option<FeedsAction>,
    },
    /// Per-day posts and engagement vs the prior window
    Analytics {
        #[arg(long, default_value_t = 7)]
        days: u32,
        #[arg(long)]
        handle: Option<String>,
        #[arg(long, default_value_t = 80)]
        limit: usize,
    },
    /// Snapshot followers (button/CLI only) and diff new vs gone
    Followers {
        #[command(subcommand)]
        action: Option<FollowersAction>,
        #[arg(long, global = true)]
        handle: Option<String>,
        #[arg(long, default_value_t = 200, global = true)]
        limit: usize,
    },
    /// Queue drafted replies; Send opens the confirm path
    Queue {
        #[command(subcommand)]
        action: Option<QueueAction>,
    },
    /// 14-day read of any handle (posts/day, engagement, best hours)
    Analyze {
        handle: String,
        #[arg(long, default_value_t = 14)]
        days: u32,
        #[arg(long, default_value_t = 80)]
        limit: usize,
    },
}

#[derive(Subcommand, Debug)]
enum WriteOp {
    /// Standalone tweet. No reply id.
    Post { text: String },
    /// Reply to one status.
    Reply {
        #[arg(long)]
        to: String,
        text: String,
    },
    /// Quote one status from its repost menu, the same path as the Q-tweet button.
    Quote {
        #[arg(long)]
        to: String,
        /// Omit with --inspect. Required before a real quote.
        text: Option<String>,
        /// Open the quote composer and report the card. Does not click Post.
        #[arg(long)]
        inspect: bool,
        /// Not supported for a quote. The command stops before the browser opens.
        #[arg(long)]
        media: Option<PathBuf>,
    },
    /// Repost one status.
    Repost {
        #[arg(long)]
        to: String,
    },
    /// Like one status. Same path as the panel Like button.
    Like {
        #[arg(long)]
        to: String,
    },
    /// Remove a like. Same path as the panel Unlike button.
    Unlike {
        #[arg(long)]
        to: String,
    },
    /// Follow one handle. Same path as the panel Follow button.
    Follow {
        #[arg(long)]
        to: String,
    },
    /// Unfollow one handle. Same path as the panel Unfollow button.
    Unfollow {
        #[arg(long)]
        to: String,
    },
}

#[derive(Subcommand, Debug)]
enum InboxAction {
    /// Mark a tweet as done
    Done { id: String },
    /// Mark a tweet as waiting
    Waiting { id: String },
    /// Draft a reply with the AI path. Never posts.
    Draft { id: String },
}

#[derive(Subcommand, Debug)]
enum FeedsAction {
    Add {
        query: String,
        #[arg(long)]
        lang: Option<String>,
        #[arg(long = "max-age")]
        max_age: Option<u32>,
        #[arg(long = "mute-word")]
        mute_word: Vec<String>,
        #[arg(long = "mute-account")]
        mute_account: Vec<String>,
    },
    Remove {
        id: String,
    },
    Run {
        id: String,
        #[arg(long, default_value_t = 15)]
        limit: usize,
    },
}

#[derive(Subcommand, Debug)]
enum FollowersAction {
    /// Take a snapshot now (the only way a snapshot is written)
    Snapshot,
    /// Diff latest vs previous without taking a new snapshot
    Diff,
}

#[derive(Subcommand, Debug)]
enum QueueAction {
    Add {
        #[arg(long = "reply-to")]
        reply_to: String,
        #[arg(long)]
        text: String,
        #[arg(long)]
        author: Option<String>,
    },
    Remove {
        id: String,
    },
    /// Open the confirm path for this item. `--dry-run` never posts.
    Send {
        id: String,
        #[arg(long)]
        dry_run: bool,
    },
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
    println!(
        "  Tabs                  Timeline, Search, Mentions, Inbox, Trends, Lookup, Compose, Research,"
    );
    println!("                        Feeds, Stats, Followers, Queue, Analyze, Help");
    println!(
        "  s                     Popular / Newest sort (every list, including nested replies and desk lists)"
    );
    println!("  Compose               Click the cyan box (caret blinks). Ctrl+V pastes in-box.");
    println!("  Attach image          Section under the compose box. Up to 4 JPG/PNG/GIF/WEBP.");
    println!("                        Thumbnails + filenames. Remove one or Clear all.");
    println!("  Publish -> Post now   Confirm shows exact text + attachments. Post now uses");
    println!("                        OpenCLI (with images) when available; else browser intent.");
    println!("  Tab / Esc / Enter     Cycle controls, cancel confirm, activate focused button");
    println!();
    println!("CLI binary: termy-xcli.exe. Do not run x.exe.");
    #[cfg(windows)]
    println!("CLI  (from the termy-x folder: .\\target-vlc\\release\\termy-xcli.exe …)");
    #[cfg(not(windows))]
    println!("CLI  (from the termy-x folder: ./target/release/termy-xcli …)");
    println!(
        "  termy-xcli doctor              Session / OpenCLI / Premium limit check. Never posts."
    );
    println!("  termy-xcli splash              Print the Termy X bird");
    println!("  termy-xcli search \"query\" [--recent|--newest|--top|--popular] [--limit N]");
    println!(
        "  termy-xcli feed [--topic \"subject\"] [--hot|--popular|--upcoming|--newest] [--limit N]"
    );
    println!("    j/k move  s popular/newest  h popular  u newest  /subject search");
    println!("    r reply box  type a line  p preview  y post  esc cancel  q quit");
    println!("  termy-xcli feed --dry-run --keys \"r;type:hello;p;y\"   Preview only, never posts");
    println!("  termy-xcli timeline [--limit N]");
    println!("  termy-xcli trends [--place NAME] [--limit N]");
    println!("  termy-xcli lookup handle");
    println!(
        "  termy-xcli stats [handle]          Profile totals + tweet-window sums (n/a if missing)"
    );
    println!(
        "  termy-xcli write post \"text\"                  Dry-run. Does not start the GUI or post."
    );
    println!("  termy-xcli write reply --to ID \"text\"         Dry-run a reply");
    println!(
        "  termy-xcli write quote --to ID --inspect        Look at the quote card. Does not click Post"
    );
    println!("  termy-xcli write quote --to ID \"text\"         Dry-run a Q-tweet");
    println!("  termy-xcli write repost --to ID                Dry-run a repost");
    println!("  termy-xcli write like --to ID                  Dry-run a like");
    println!("  termy-xcli write unlike --to ID                Dry-run an unlike");
    println!("  termy-xcli write follow --to HANDLE            Dry-run a follow");
    println!("  termy-xcli write unfollow --to HANDLE          Dry-run an unfollow");
    println!("  termy-xcli write post \"text\" --confirm yes    One real post. No retry.");
    println!("  termy-xcli post \"text\"");
    println!("  termy-xcli post --dry-run \"text\"          Plan only - never posts");
    println!("  termy-xcli post --media pic.jpg \"text\"    Up to 4 images (repeat --media)");
    println!("  termy-xcli post --reply-to ID \"text\"      Reply to a status");
    println!("  termy-xcli inbox [--handle me]            Replies/mentions still waiting");
    println!("  termy-xcli inbox done ID | waiting ID | draft ID");
    println!("  termy-xcli mentions                       Posts that mention you, newest first");
    println!("  termy-xcli feeds | feeds add QUERY [--lang en] [--max-age 24]");
    println!("  termy-xcli feeds run ID                   Search + apply local filters");
    println!("  termy-xcli analytics [--days 7]           Per-day stats vs the prior window");
    println!("  termy-xcli followers snapshot | diff      Snapshot on command only; then diff");
    println!("  termy-xcli queue | queue add --reply-to ID --text \"...\"");
    println!("  termy-xcli queue send ID --dry-run        Confirm path; dry-run never posts");
    println!("  termy-xcli analyze HANDLE [--days 14]     14-day read of any account");
    #[cfg(windows)]
    println!("  PowerShell long text:  termy-xcli post --dry-run (Get-Content -Raw .\\post.txt)");
    #[cfg(not(windows))]
    println!("  Shell long text:       termy-xcli post --dry-run \"$(cat ./post.txt)\"");
    println!();
    println!("Nothing posts until you type yes (CLI) or hit Post now after Confirm (GUI).");
    println!("Premium auto-detects (25,000 chars); otherwise 280.");
}

pub fn execute(args: &[String], io: &mut CliIo, interactive: bool) -> i32 {
    let cli = match Cli::try_parse_from(
        std::iter::once("termy-xcli".to_string()).chain(args.iter().cloned()),
    ) {
        Ok(cli) => cli,
        Err(error) => {
            if error.kind() == clap::error::ErrorKind::DisplayHelp
                || error.kind() == clap::error::ErrorKind::DisplayVersion
            {
                // Keep --help/--version clean and script-friendly: no splash.
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
        if !provider.eq_ignore_ascii_case("auto") && ProviderId::parse(provider).is_none() {
            eprintln!(
                "unknown provider `{provider}`. Use auto, opencli, twitter, mock, or official. Example: x --provider mock timeline"
            );
            return 2;
        }
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
        Some(Cmd::Connect {
            check,
            disconnect,
            save_token,
        }) => connect(check, disconnect, save_token, io),
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
        Cmd::Actions => {
            let _ = json;
            let path = io.config.action_state_path();
            match crate::action_state::export_sanitized(&path) {
                Ok(text) => {
                    println!("{text}");
                    0
                }
                Err(error) => {
                    eprintln!("{error}");
                    1
                }
            }
        }
        Cmd::Lookup { handle } if handle.trim().is_empty() => {
            eprintln!("give a username. Example: x lookup yourname");
            2
        }
        Cmd::Lookup { handle } => emit(
            json,
            service
                .lookup(&handle)
                .map(|served| (served.status.render(), json_value(served))),
        ),
        Cmd::Stats { handle } => cmd_stats(handle, json, &service, &io.config),
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
        Cmd::Write { op, confirm } => gui_write(op, confirm, json, io),
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
        Cmd::Inbox {
            action,
            limit,
            handle,
        } => cmd_inbox(action, limit, handle, json, io),
        Cmd::Mentions { limit, handle } => cmd_mentions(limit, handle, json, io),
        Cmd::Feeds { action } => cmd_feeds(action, json, io),
        Cmd::Analytics {
            days,
            handle,
            limit,
        } => cmd_analytics(days, handle, limit, json, io),
        Cmd::Followers {
            action,
            handle,
            limit,
        } => cmd_followers(action, handle, limit, json, io),
        Cmd::Queue { action } => cmd_queue(action, json, io, interactive),
        Cmd::Analyze {
            handle,
            days,
            limit,
        } => cmd_analyze(handle, days, limit, json, io),
        Cmd::Help | Cmd::Doctor | Cmd::Config | Cmd::Auth | Cmd::Connect { .. } => 0,
    }
}

fn resolve_handle(io: &CliIo, handle: Option<String>) -> Result<String, String> {
    if let Some(handle) = handle {
        let handle = crate::model::strip_handle(&handle).to_string();
        if !handle.is_empty() {
            return Ok(handle);
        }
    }
    crate::connect::load_connected_handle(&io.config)
        .map(|h| crate::model::strip_handle(&h).to_string())
        .filter(|h| !h.is_empty())
        .or_else(|| {
            if io.config.provider == Some(crate::model::ProviderId::Mock) {
                Some("termy".into())
            } else {
                None
            }
        })
        .ok_or_else(|| {
            "no connected handle. Run `x connect --check`, or pass --handle.".to_string()
        })
}

fn cmd_inbox(
    action: Option<InboxAction>,
    limit: usize,
    handle: Option<String>,
    json: bool,
    io: &mut CliIo,
) -> i32 {
    let me = match resolve_handle(io, handle) {
        Ok(me) => me,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    let mut store = crate::inbox::load_inbox(&io.config.inbox_path());
    match action {
        Some(InboxAction::Done { id }) => {
            crate::inbox::mark(&mut store, &id, crate::inbox::InboxState::Done);
            if let Err(error) = crate::inbox::save_inbox(&io.config.inbox_path(), &store) {
                eprintln!("{error}");
                return 1;
            }
            println!("inbox {id} · done");
            0
        }
        Some(InboxAction::Waiting { id }) => {
            crate::inbox::mark(&mut store, &id, crate::inbox::InboxState::Waiting);
            if let Err(error) = crate::inbox::save_inbox(&io.config.inbox_path(), &store) {
                eprintln!("{error}");
                return 1;
            }
            println!("inbox {id} · waiting");
            0
        }
        Some(InboxAction::Draft { id }) => {
            let service = Service::free(&io.config, io.runner.clone(), io.http.clone());
            match service.inbox_posts(&me, clamp_limit(limit, 20)) {
                Ok(served) => {
                    let Some(post) = served
                        .value
                        .iter()
                        .find(|p| p.id == id || p.id.starts_with(&id))
                    else {
                        eprintln!("no inbox item `{id}`");
                        return 1;
                    };
                    let idea = crate::inbox::draft_idea(post);
                    match crate::ai::draft_variants(
                        &io.config,
                        io.http.as_ref(),
                        &idea,
                        crate::model::Tone::Reply,
                        3,
                    ) {
                        Ok(mut drafts) => {
                            for draft in &mut drafts {
                                draft.reply_to = Some(post.id.clone());
                            }
                            if let Err(error) = crate::drafts::upsert_drafts(
                                &io.config.drafts_path(),
                                drafts.clone(),
                            ) {
                                eprintln!("{error}");
                                return 1;
                            }
                            if json {
                                println!(
                                    "{}",
                                    serde_json::to_string_pretty(&drafts)
                                        .unwrap_or_else(|_| "[]".into())
                                );
                            } else {
                                println!("drafts for reply to {} · nothing posted", post.id);
                                for draft in &drafts {
                                    println!("{} · {}", draft.id, draft.text);
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
                Err(error) => {
                    eprintln!("{error}");
                    1
                }
            }
        }
        None => {
            let service = Service::free(&io.config, io.runner.clone(), io.http.clone());
            match service.inbox_posts(&me, clamp_limit(limit, 20)) {
                Ok(served) => {
                    let items = crate::inbox::apply_states(&served.value, &store);
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({
                                "ok": true,
                                "handle": me,
                                "items": items.iter().map(|item| serde_json::json!({
                                    "state": item.state.as_str(),
                                    "id": item.post.id,
                                    "author": item.post.author_handle,
                                    "text": item.post.text,
                                })).collect::<Vec<_>>(),
                            })
                        );
                    } else {
                        println!("inbox · @{me} · {} item(s)", items.len());
                        if items.is_empty() {
                            println!("nothing waiting — empty result, not an error");
                        }
                        for item in &items {
                            println!(
                                "[{}] {} @{} {}",
                                item.state.as_str(),
                                item.post.id,
                                item.post.author_handle,
                                item.post.text.chars().take(80).collect::<String>()
                            );
                        }
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
    }
}

fn cmd_mentions(limit: usize, handle: Option<String>, json: bool, io: &CliIo) -> i32 {
    let me = match resolve_handle(io, handle) {
        Ok(me) => me,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    let service = Service::free(&io.config, io.runner.clone(), io.http.clone());
    emit(
        json,
        service
            .mentions_posts(&me, clamp_limit(limit, 20))
            .map(|served| (served.status.render(), json_value(served))),
    )
}

fn cmd_feeds(action: Option<FeedsAction>, json: bool, io: &CliIo) -> i32 {
    let mut store = crate::feeds::load_feeds(&io.config.feeds_path());
    match action {
        None => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&store).unwrap_or_else(|_| "{}".into())
                );
            } else if store.feeds.is_empty() {
                println!("no feeds yet. Add one: x feeds add \"topic\" --lang en --max-age 24");
            } else {
                for feed in &store.feeds {
                    println!(
                        "{} · {} · lang={} · age={}h · mute {} word(s) {} account(s)",
                        feed.id,
                        feed.query,
                        feed.lang.as_deref().unwrap_or("-"),
                        feed.max_age_hours
                            .map_or_else(|| "-".into(), |h| h.to_string()),
                        feed.muted_words.len(),
                        feed.muted_accounts.len()
                    );
                }
            }
            0
        }
        Some(FeedsAction::Add {
            query,
            lang,
            max_age,
            mute_word,
            mute_account,
        }) => {
            if query.trim().is_empty() {
                eprintln!("feeds add needs a query");
                return 2;
            }
            let feed = crate::feeds::add_feed(
                &mut store,
                crate::feeds::EngageFeed {
                    id: String::new(),
                    query,
                    lang,
                    max_age_hours: max_age,
                    muted_words: mute_word,
                    muted_accounts: mute_account,
                },
            );
            if let Err(error) = crate::feeds::save_feeds(&io.config.feeds_path(), &store) {
                eprintln!("{error}");
                return 1;
            }
            println!(
                "saved feed {} · {}",
                feed.id,
                crate::feeds::search_query(&feed)
            );
            0
        }
        Some(FeedsAction::Remove { id }) => {
            if !crate::feeds::remove_feed(&mut store, &id) {
                eprintln!("no feed `{id}`");
                return 1;
            }
            if let Err(error) = crate::feeds::save_feeds(&io.config.feeds_path(), &store) {
                eprintln!("{error}");
                return 1;
            }
            println!("removed feed {id}");
            0
        }
        Some(FeedsAction::Run { id, limit }) => {
            let Some(feed) = crate::feeds::find_feed(&store, &id).cloned() else {
                eprintln!("no feed `{id}`");
                return 1;
            };
            let service = Service::free(&io.config, io.runner.clone(), io.http.clone());
            let query = crate::feeds::search_query(&feed);
            match service.search(&query, true, clamp_limit(limit, 15)) {
                Ok(mut served) => {
                    served.value =
                        crate::feeds::filter_posts(&served.value, &feed, crate::time::unix_now());
                    crate::time::sort_newest(&mut served.value, |post| post.created_at.as_deref());
                    emit(json, Ok((served.status.render(), json_value(served))))
                }
                Err(error) => {
                    eprintln!("{error}");
                    1
                }
            }
        }
    }
}

fn cmd_analytics(days: u32, handle: Option<String>, limit: usize, json: bool, io: &CliIo) -> i32 {
    let me = match resolve_handle(io, handle) {
        Ok(me) => me,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    let service = Service::free(&io.config, io.runner.clone(), io.http.clone());
    match service.tweets(&me, limit.clamp(1, 200)) {
        Ok(served) => {
            let now = crate::time::unix_now();
            let compare = crate::analytics_stats::compare_periods(&served.value, now, days);
            let heat = crate::analytics_stats::heatmap(&served.value);
            let top = crate::analytics_stats::top_posts(&served.value, 5);
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "ok": true,
                        "handle": me,
                        "compare": compare,
                        "heatmap_best": heat.best,
                        "top": top.iter().map(|p| serde_json::json!({
                            "id": p.id, "text": p.text, "engagement": p.engagement()
                        })).collect::<Vec<_>>(),
                    })
                );
            } else {
                println!(
                    "analytics · @{me} · last {}d vs prior {}d",
                    compare.days, compare.days
                );
                println!(
                    "posts {} ({:+.1}%) · likes {} ({:+.1}%) · replies {} ({:+.1}%) · reposts {} ({:+.1}%) · views {} ({:+.1}%)",
                    compare.current.posts,
                    compare.delta_pct.posts,
                    compare.current.likes,
                    compare.delta_pct.likes,
                    compare.current.replies,
                    compare.delta_pct.replies,
                    compare.current.reposts,
                    compare.delta_pct.reposts,
                    compare.current.views,
                    compare.delta_pct.views
                );
                if let Some(best) = &heat.best {
                    println!(
                        "best hour · {} · engagement {}",
                        best.label(),
                        best.engagement
                    );
                }
                for post in &top {
                    println!(
                        "top {} · {} · {}",
                        post.id,
                        post.engagement(),
                        post.text.chars().take(72).collect::<String>()
                    );
                }
                if served.value.is_empty() {
                    println!("no tweets in this window — empty result, not an error");
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

fn cmd_followers(
    action: Option<FollowersAction>,
    handle: Option<String>,
    limit: usize,
    json: bool,
    io: &CliIo,
) -> i32 {
    let me = match resolve_handle(io, handle) {
        Ok(me) => me,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    let mut log = crate::followers::load_log(&io.config.followers_path());
    if matches!(action, Some(FollowersAction::Snapshot)) {
        let service = Service::free(&io.config, io.runner.clone(), io.http.clone());
        match service.followers(&me, limit.clamp(1, 200)) {
            Ok(served) => {
                crate::followers::append_snapshot(
                    &mut log,
                    crate::followers::make_snapshot(&me, served.value, None),
                );
                if let Err(error) = crate::followers::save_log(&io.config.followers_path(), &log) {
                    eprintln!("{error}");
                    return 1;
                }
                println!(
                    "snapshot saved · @{me} · {} follower(s)",
                    log.snapshots.last().map_or(0, |s| s.followers.len())
                );
            }
            Err(error) => {
                eprintln!("{error}");
                return 1;
            }
        }
    }
    if let Some(diff) = crate::followers::diff_latest(&log) {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&diff).unwrap_or_else(|_| "{}".into())
            );
        } else {
            println!(
                "followers · @{me} · latest {} · previous {}",
                diff.latest_count, diff.previous_count
            );
            if diff.previous_at.is_none() {
                println!(
                    "only one snapshot — new list is the current set; take another later to diff"
                );
            }
            println!("new ({})", diff.new.len());
            for row in &diff.new {
                println!("  + @{} {}", row.handle, row.name);
            }
            println!("unfollowed ({})", diff.gone.len());
            for row in &diff.gone {
                println!("  - @{} {}", row.handle, row.name);
            }
        }
        0
    } else {
        println!("no snapshots yet. Run `x followers snapshot` (never automatic).");
        0
    }
}

fn cmd_queue(action: Option<QueueAction>, json: bool, io: &mut CliIo, interactive: bool) -> i32 {
    let mut items = crate::queue::load_queue(&io.config.queue_path());
    match action {
        None => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".into())
                );
            } else if items.is_empty() {
                println!("reply queue is empty");
            } else {
                for item in &items {
                    println!(
                        "{} · reply to {} @{} · {}",
                        item.id, item.reply_to, item.author, item.text
                    );
                }
            }
            0
        }
        Some(QueueAction::Add {
            reply_to,
            text,
            author,
        }) => {
            let item =
                crate::queue::make_item(&text, &reply_to, author.as_deref().unwrap_or(""), None);
            if let Err(error) = crate::queue::enqueue(&mut items, item.clone()) {
                eprintln!("{error}");
                return 1;
            }
            if let Err(error) = crate::queue::save_queue(&io.config.queue_path(), &items) {
                eprintln!("{error}");
                return 1;
            }
            println!("queued {} · reply to {}", item.id, item.reply_to);
            0
        }
        Some(QueueAction::Remove { id }) => {
            if crate::queue::remove_item(&mut items, &id).is_none() {
                eprintln!("no queue item `{id}`");
                return 1;
            }
            if let Err(error) = crate::queue::save_queue(&io.config.queue_path(), &items) {
                eprintln!("{error}");
                return 1;
            }
            println!("removed {id}");
            0
        }
        Some(QueueAction::Send { id, dry_run }) => {
            let Some(item) = crate::queue::find_item(&items, &id).cloned() else {
                eprintln!("no queue item `{id}`");
                return 1;
            };
            let code = publish_text(
                &item.text,
                dry_run,
                Some(item.reply_to.clone()),
                Vec::new(),
                json,
                io,
                interactive,
            );
            if code == 0 && !dry_run {
                crate::queue::remove_item(&mut items, &item.id);
                let _ = crate::queue::save_queue(&io.config.queue_path(), &items);
            }
            code
        }
    }
}

fn cmd_analyze(handle: String, days: u32, limit: usize, json: bool, io: &CliIo) -> i32 {
    let handle = crate::model::strip_handle(&handle).to_string();
    if handle.is_empty() {
        eprintln!("analyze needs a handle. Example: x analyze ada");
        return 2;
    }
    let service = Service::free(&io.config, io.runner.clone(), io.http.clone());
    let profile = service.lookup(&handle).ok().map(|s| s.value);
    match service.tweets(&handle, limit.clamp(1, 200)) {
        Ok(served) => {
            let posts = if served.value.is_empty() {
                profile
                    .as_ref()
                    .map(|p| p.posts.clone())
                    .unwrap_or_default()
            } else {
                served.value
            };
            let report = crate::analytics_stats::account_report(
                &handle,
                profile.as_ref(),
                &posts,
                crate::time::unix_now(),
                days,
            );
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".into())
                );
            } else {
                println!(
                    "analyze · @{} · {}d · {} posts ({:.2}/day) · avg engagement {:.1} · reply ratio {:.0}%",
                    report.handle,
                    report.days,
                    report.posts,
                    report.posts_per_day,
                    report.avg_engagement,
                    report.reply_ratio * 100.0
                );
                if let Some(best) = report.best_hours.first() {
                    println!("best hours · {}", best.label());
                }
                for post in &report.top_posts {
                    println!(
                        "top {} · {} · {}",
                        post.id,
                        post.engagement(),
                        post.text.chars().take(72).collect::<String>()
                    );
                }
                if posts.is_empty() {
                    println!("no tweets returned — empty result, not an error");
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

pub fn parse_whoami(text: &str) -> Result<String, String> {
    crate::connect::parse_whoami(text)
}

fn connect(check: bool, disconnect: bool, save_token: bool, io: &CliIo) -> i32 {
    if disconnect {
        return match crate::connect::disconnect(&io.config) {
            Ok(()) => {
                println!(
                    "Disconnected on this machine: forgot the saved developer key and remembered account."
                );
                println!(
                    "To switch accounts: log in to X in Chrome as the other account, then run `x connect --check`."
                );
                println!(
                    "(Termy X cannot log Chrome out for you, and never asks for your password.)"
                );
                0
            }
            Err(error) => {
                eprintln!("could not disconnect: {error}");
                1
            }
        };
    }
    if save_token {
        println!("Paste YOUR OWN X developer token, then press Enter.");
        println!(
            "It is stored in your OS keychain (or a private 0600 file) — never in x.toml, never printed."
        );
        print!("Token: ");
        let _ = io::stdout().flush();
        let mut line = String::new();
        if io::stdin().read_line(&mut line).is_err() {
            eprintln!("could not read the token. Nothing was saved.");
            return 1;
        }
        return match crate::connect::store_bearer_token(&io.config, &line) {
            Ok(()) => {
                println!("Saved. Now run `x connect --check` to prove it works.");
                0
            }
            Err(error) => {
                eprintln!("{error}");
                1
            }
        };
    }
    if !check {
        println!("Connect YOUR X account — pick one way:");
        println!();
        println!("Easiest (no developer key, recommended):");
        println!("  1. Open Chrome.");
        println!("  2. Go to x.com and log in as yourself.");
        println!("  3. Run: x connect --check");
        println!("  You should see: Connected as @yourname");
        println!();
        println!("Your own X developer token (optional, only if you have one):");
        println!("  1. Create an app at developer.x.com (your account, your keys).");
        println!("  2. Run: x connect --save-token   and paste the token there");
        println!("     (or set TERMY_X_BEARER_TOKEN in your shell only — never in a file or chat)");
        println!("  3. Run: x connect --check");
        println!();
        println!("Switch or forget this machine: x connect --disconnect");
        println!("Termy X never stores your password and never prints your token.");
        return 0;
    }
    match crate::connect::detect_connection(&io.config, io.runner.as_ref(), io.http.as_ref()) {
        crate::connect::Connection::Connected { handle, via } => {
            println!("Connected as @{handle} via {via}.");
            0
        }
        crate::connect::Connection::NotConnected { reason, next } => {
            eprintln!("Not connected — {reason}.");
            eprintln!("Next steps:");
            for line in next.lines() {
                eprintln!("  {line}");
            }
            1
        }
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

fn inspect_quote(target: &str, media: Option<PathBuf>, io: &CliIo) -> i32 {
    let id = crate::model::post_id_from_input(target);
    if id.is_empty() || !id.chars().all(|ch| ch.is_ascii_digit()) {
        eprintln!("The target must be a status id or a status URL. Post was not clicked.");
        return 2;
    }
    if !io.runner.exists(&io.config.opencli_bin) {
        eprintln!("OpenCLI is not installed. Post was not clicked.");
        return 1;
    }
    let media_paths = media.into_iter().collect::<Vec<_>>();
    match crate::providers::inspect_quote_attachment(
        &io.config.opencli_bin,
        io.runner.as_ref(),
        &id,
        &media_paths,
    ) {
        Ok(line) => {
            println!("quote inspect — Post was not clicked");
            println!("{line}");
            0
        }
        Err(error) => {
            println!("quote inspect — Post was not clicked");
            println!("{error}");
            eprintln!("quote inspect not ready — Post was not clicked");
            1
        }
    }
}

fn gui_write(op: WriteOp, confirm: Option<String>, json: bool, io: &mut CliIo) -> i32 {
    use crate::publish::{GuiWrite, GuiWriteRequest, run_gui_write};
    let write = match op {
        WriteOp::Post { text } => GuiWrite::Post { text },
        WriteOp::Reply { to, text } => GuiWrite::Reply {
            target_id: to,
            text,
        },
        WriteOp::Quote {
            to,
            text,
            inspect,
            media,
        } => {
            if media.is_some() {
                eprintln!("{}", crate::publish::quote_image_unsupported());
                return 1;
            }
            if inspect {
                return inspect_quote(&to, None, io);
            }
            GuiWrite::Quote {
                target_id: to,
                text: text.unwrap_or_default(),
                media: media.into_iter().collect(),
            }
        }
        WriteOp::Repost { to } => GuiWrite::Repost { target_id: to },
        WriteOp::Like { to } => GuiWrite::Like { target_id: to },
        WriteOp::Unlike { to } => GuiWrite::Unlike { target_id: to },
        WriteOp::Follow { to } => GuiWrite::Follow { handle: to },
        WriteOp::Unfollow { to } => GuiWrite::Unfollow { handle: to },
    };
    let approved = confirm.as_deref() == Some("yes");
    let service = Service::free(&io.config, io.runner.clone(), io.http.clone());
    let publisher = if approved {
        first_available(service.publishers.as_slice())
    } else {
        service
            .publishers
            .iter()
            .find(|publisher| match publisher.id() {
                crate::model::ProviderId::OpenCli => io.runner.exists(&io.config.opencli_bin),
                crate::model::ProviderId::TwitterCli => io.runner.exists(&io.config.twitter_bin),
                crate::model::ProviderId::OfficialApi => io.config.official_enabled,
                crate::model::ProviderId::Mock | crate::model::ProviderId::WebIntent => false,
            })
            .map(|publisher| publisher.as_ref())
    };
    let Some(publisher) = publisher else {
        eprintln!(
            "No in-app posting path found. Nothing was sent. This command does not open the Termy window or a browser."
        );
        return 1;
    };
    let account = crate::connect::load_connected_handle(&io.config).unwrap_or_default();
    if !approved {
        let mut db = crate::action_state::ActionDb::default();
        let outcome = run_gui_write(
            publisher,
            &mut db,
            &GuiWriteRequest {
                write,
                confirm,
                account,
                char_limit: io.config.char_limit_or_fallback(),
            },
            0,
            0,
        );
        return print_gui_write(&outcome, json, false);
    }
    let path = io.config.action_state_path();
    let mut db = match crate::action_state::ActionDb::load(&path) {
        Ok(db) => db,
        Err(error) => {
            eprintln!("{error} Nothing was sent.");
            return 1;
        }
    };
    let account = crate::connect::load_connected_handle(&io.config).unwrap_or_default();
    let outcome = run_gui_write(
        publisher,
        &mut db,
        &GuiWriteRequest {
            write,
            confirm,
            account,
            char_limit: io.config.char_limit_or_fallback(),
        },
        crate::action_state::now_ms(),
        crate::action_state::now_ms(),
    );
    let saved = if outcome.saved_a_row() {
        match db.save(&path) {
            Ok(()) => true,
            Err(error) => {
                eprintln!("The write finished, but the action database was not saved: {error}");
                false
            }
        }
    } else {
        false
    };
    print_gui_write(&outcome, json, saved)
}

fn print_gui_write(outcome: &crate::publish::GuiWriteOutcome, json: bool, saved: bool) -> i32 {
    match outcome {
        crate::publish::GuiWriteOutcome::DryRun { provider, detail } => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "ok": true,
                        "dry_run": true,
                        "sent": false,
                        "provider": provider.as_str(),
                        "detail": detail,
                    })
                );
            } else {
                println!("dry-run — nothing was sent");
                println!("{detail}");
                println!("Add --confirm yes to perform this one write. It will not retry.");
            }
            0
        }
        crate::publish::GuiWriteOutcome::Sent { provider, receipt } => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "ok": true,
                        "dry_run": false,
                        "sent": true,
                        "provider": provider.as_str(),
                        "receipt": receipt,
                        "saved": saved,
                    })
                );
            } else {
                println!("sent — one write, no retry");
                println!("{receipt}");
                println!("provider {}", provider.label());
                if saved {
                    println!("saved the action row");
                }
            }
            0
        }
        crate::publish::GuiWriteOutcome::Rejected { message } => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "ok": false,
                        "dry_run": false,
                        "sent": false,
                        "error": message,
                    })
                );
            } else {
                eprintln!("not sent");
                eprintln!("{message}");
            }
            1
        }
    }
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
    if text.trim().is_empty() {
        eprintln!("nothing to post: the text is empty. Example: x post --dry-run \"hello world\"");
        return 2;
    }
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
    // MUST use the media-aware path: confirm_and_publish drops attachments.
    let outcome = crate::publish::confirm_and_publish_with_media(
        text,
        reply_to.as_deref(),
        decision,
        false,
        publisher,
        resolved.limit,
        &media,
    );
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
    if !plan.media.is_empty() {
        println!(
            "images cannot go through a web intent — attach them in the browser compose box: {}",
            plan.media
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
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

fn cmd_stats(handle: Option<String>, json: bool, service: &Service, config: &XConfig) -> i32 {
    let handle = handle
        .filter(|h| !h.trim().is_empty())
        .or_else(|| crate::connect::load_connected_handle(config));
    let Some(handle) = handle.filter(|h| !h.trim().is_empty()) else {
        eprintln!("give a username. Example: x stats yourname");
        return 2;
    };
    match service.lookup(&handle) {
        Ok(served) => {
            let stats = crate::stats::account_stats_from_profile(&served.value);
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "ok": true,
                        "status": served.status.render(),
                        "credits": "none",
                        "data": stats,
                    })
                );
            } else {
                println!("{}", served.status.render());
                println!("{}", crate::stats::render_account_stats(&stats));
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
        .map_err(|error| format!("could not open an editor ({editor}: {error}). Use x compose --text \"your draft\" instead, or set EDITOR to an editor you have."))?;
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
    crate::publish::validate_media_paths(media)
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
    fn dummy_mistakes_are_friendly_errors_not_crashes() {
        let dir = tempfile::tempdir().unwrap();
        let mut io = test_io(dir.path());
        // Bad provider must not silently fall back to mock.
        assert_eq!(
            execute(
                &["--provider".into(), "bogus".into(), "timeline".into()],
                &mut io,
                false
            ),
            2
        );
        // Empty post must not create an empty browser intent.
        assert_eq!(
            execute(
                &["post".into(), "--dry-run".into(), "   ".into()],
                &mut io,
                false
            ),
            2
        );
        assert_eq!(
            execute(
                &["post".into(), "--dry-run".into(), "".into()],
                &mut io,
                false
            ),
            2
        );
        // Empty lookup.
        assert_eq!(execute(&["lookup".into(), "".into()], &mut io, false), 2);
        assert_eq!(execute(&["stats".into()], &mut io, false), 2);
        // Connect instructions are printable with no account set up.
        assert_eq!(execute(&["connect".into()], &mut io, false), 0);
    }

    #[test]
    fn write_dry_run_does_not_spawn_or_post() {
        let dir = tempfile::tempdir().unwrap();
        let runner = Arc::new(ScriptedRunner::new());
        runner.install("opencli");
        let mut io = test_io(dir.path());
        io.config.provider = Some(ProviderId::OpenCli);
        io.runner = runner.clone();
        assert_eq!(
            execute(
                &[
                    "write".into(),
                    "post".into(),
                    "hello from the harness".into()
                ],
                &mut io,
                false
            ),
            0
        );
        assert!(
            runner.calls.lock().expect("calls").is_empty(),
            "dry-run must not start OpenCLI"
        );
        assert_eq!(
            execute(
                &[
                    "write".into(),
                    "repost".into(),
                    "--to".into(),
                    "2118971448614334959".into(),
                    "--confirm".into(),
                    "Yes".into()
                ],
                &mut io,
                false
            ),
            0
        );
        assert!(runner.calls.lock().expect("calls").is_empty());
        assert_eq!(
            execute(&["write".into(), "--help".into()], &mut io, false),
            0
        );
        assert_eq!(execute(&["--help".into()], &mut io, false), 0);
        assert!(runner.calls.lock().expect("calls").is_empty());
        assert_eq!(
            execute(
                &["write".into(), "like".into(), "--help".into()],
                &mut io,
                false
            ),
            0
        );
        assert_eq!(
            execute(
                &[
                    "write".into(),
                    "follow".into(),
                    "--to".into(),
                    "moosaswrld".into()
                ],
                &mut io,
                false
            ),
            0
        );
        assert!(runner.calls.lock().expect("calls").is_empty());
    }

    #[test]
    fn write_like_and_follow_use_the_panel_path_and_a_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let runner = Arc::new(ScriptedRunner::new());
        runner.script(
            "opencli-like",
            &["twitter", "--help"],
            "post <text>\nlike\nunlike\nfollow\nunfollow\nretweet\n",
        );
        runner.script(
            "opencli-like",
            &["twitter", "like"],
            r#"[{"status":"success","message":"clicked","unconfirmed":true}]"#,
        );
        runner.script(
            "opencli-like",
            &["twitter", "timeline"],
            r#"[{"id":"2119746741364981888","favorited":true}]"#,
        );
        runner.script(
            "opencli-like",
            &["twitter", "follow"],
            r#"[{"status":"success","message":"clicked","unconfirmed":true}]"#,
        );
        runner.script(
            "opencli-like",
            &["twitter", "user"],
            r#"{"screen_name":"moosaswrld","following":false}"#,
        );
        let mut io = test_io(dir.path());
        io.config.opencli_bin = "opencli-like".into();
        io.config.provider = Some(ProviderId::OpenCli);
        io.runner = runner.clone();
        crate::connect::save_connected_handle(&io.config, "jasper_black").unwrap();
        assert_eq!(
            execute(
                &[
                    "write".into(),
                    "like".into(),
                    "--to".into(),
                    "2119746741364981888".into(),
                    "--confirm".into(),
                    "yes".into()
                ],
                &mut io,
                false
            ),
            0
        );
        let calls = runner.calls.lock().unwrap().clone();
        assert!(
            calls
                .iter()
                .any(|(_, args)| args.get(1).map(String::as_str) == Some("like"))
        );
        assert!(
            calls
                .iter()
                .any(|(_, args)| args.get(1).map(String::as_str) == Some("timeline"))
        );
        assert_ne!(
            execute(
                &[
                    "write".into(),
                    "follow".into(),
                    "--to".into(),
                    "moosaswrld".into(),
                    "--confirm".into(),
                    "yes".into()
                ],
                &mut io,
                false
            ),
            0
        );
        assert!(
            !runner
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|(_, args)| args.iter().any(|arg| arg == "x.exe"))
        );
    }

    #[test]
    fn whoami_parses_v1_and_v2_opencli_shapes() {
        assert_eq!(
            parse_whoami(r#"[{"logged_in":true,"username":"ada"}]"#).unwrap(),
            "ada"
        );
        assert_eq!(
            parse_whoami(r#"{"ok":true,"data":[{"logged_in":true,"username":"@ada"}]}"#).unwrap(),
            "ada"
        );
        assert!(parse_whoami(r#"{"logged_in":false}"#).is_err());
        assert!(parse_whoami(r#"{"ok":false,"error":{"message":"nope"}}"#).is_err());
        assert!(parse_whoami("not json").is_err());
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
            execute(
                &["stats".into(), "termy".into(), "--json".into()],
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

    #[test]
    fn desk_commands_run_on_mock_without_posting() {
        let dir = tempfile::tempdir().unwrap();
        let mut io = test_io(dir.path());
        assert_eq!(
            execute(
                &["inbox".into(), "--handle".into(), "termy".into()],
                &mut io,
                false
            ),
            0
        );
        assert_eq!(
            execute(
                &["inbox".into(), "done".into(), "1001".into()],
                &mut io,
                false
            ),
            0
        );
        assert_eq!(
            execute(
                &["mentions".into(), "--handle".into(), "termy".into()],
                &mut io,
                false
            ),
            0
        );
        assert_eq!(
            execute(
                &[
                    "feeds".into(),
                    "add".into(),
                    "rust".into(),
                    "--lang".into(),
                    "en".into(),
                    "--max-age".into(),
                    "24".into(),
                    "--mute-word".into(),
                    "spam".into()
                ],
                &mut io,
                false
            ),
            0
        );
        let feeds = crate::feeds::load_feeds(&io.config.feeds_path());
        assert_eq!(feeds.feeds.len(), 1);
        assert_eq!(
            execute(
                &["feeds".into(), "run".into(), feeds.feeds[0].id.clone()],
                &mut io,
                false
            ),
            0
        );
        assert_eq!(execute(&["feeds".into()], &mut io, false), 0);
        assert_eq!(
            execute(
                &[
                    "analytics".into(),
                    "--handle".into(),
                    "termy".into(),
                    "--days".into(),
                    "7".into()
                ],
                &mut io,
                false
            ),
            0
        );
        assert_eq!(
            execute(
                &[
                    "followers".into(),
                    "snapshot".into(),
                    "--handle".into(),
                    "termy".into()
                ],
                &mut io,
                false
            ),
            0
        );
        assert_eq!(
            execute(&["followers".into(), "diff".into()], &mut io, false),
            0
        );
        assert_eq!(
            execute(
                &[
                    "queue".into(),
                    "add".into(),
                    "--reply-to".into(),
                    "1001".into(),
                    "--text".into(),
                    "thanks".into(),
                    "--author".into(),
                    "ada".into()
                ],
                &mut io,
                false
            ),
            0
        );
        let queued = crate::queue::load_queue(&io.config.queue_path());
        assert_eq!(queued.len(), 1);
        assert_eq!(
            execute(
                &[
                    "queue".into(),
                    "send".into(),
                    queued[0].id.clone(),
                    "--dry-run".into()
                ],
                &mut io,
                false
            ),
            0
        );
        assert_eq!(
            crate::queue::load_queue(&io.config.queue_path()).len(),
            1,
            "dry-run must leave the item queued"
        );
        assert_eq!(
            execute(
                &["queue".into(), "remove".into(), queued[0].id.clone()],
                &mut io,
                false
            ),
            0
        );
        assert_eq!(
            execute(
                &[
                    "analyze".into(),
                    "termy".into(),
                    "--days".into(),
                    "14".into()
                ],
                &mut io,
                false
            ),
            0
        );
        assert_eq!(execute(&["analyze".into(), "".into()], &mut io, false), 2);
    }
}
