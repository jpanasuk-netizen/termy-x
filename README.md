<p align="center">
  <img alt="Termy — The terminal, at full speed" src="./assets/termy-readme-hero.png" width="900" />
</p>

<p align="center">
  <a href="https://github.com/lassejlv/termy/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/lassejlv/termy?style=flat-square&color=7aa2f7&labelColor=16161e" /></a>
  <a href="https://github.com/lassejlv/termy/stargazers"><img alt="GitHub stars" src="https://img.shields.io/github/stars/lassejlv/termy?style=flat-square&color=9ece6a&labelColor=16161e" /></a>
  <a href="./LICENSE"><img alt="MIT license" src="https://img.shields.io/badge/license-MIT-c0caf5?style=flat-square&labelColor=16161e" /></a>
</p>

<p align="center">
  <a href="https://termy.sh/download"><strong>Download</strong></a> ·
  <a href="https://termy.sh/docs"><strong>Documentation</strong></a> ·
  <a href="https://github.com/lassejlv/termy/releases"><strong>Releases</strong></a> ·
  <a href="./CONTRIBUTING.md"><strong>Contribute</strong></a>
</p>

Termy is a fast, native terminal for macOS, Linux, and Windows. It combines GPU-accelerated rendering with the terminal workflows you use every day—tabs, splits, search, tasks, layouts, themes, and optional tmux sessions—without turning the interface into a control panel.

- Damage-scoped GPU rendering with dirty-span cell caching
- Tabs, splits, search, tasks, and reusable layouts
- Configurable keybindings, colors, themes, and terminal behavior
- Optional tmux control-mode sessions
- Native platform integration with a reusable headless runtime and FFI

## Install

Download the latest build from **[termy.sh/download](https://termy.sh/download)** or browse every artifact on **[GitHub Releases](https://github.com/lassejlv/termy/releases)**.

macOS DMGs from v0.2.75 onward are signed and notarized. Open the DMG and drag
Termy to `/Applications`. See [macOS troubleshooting](https://termy.sh/docs/getting-started/troubleshooting)
if the app still does not open.

### Build from source

Termy is a Rust workspace. Build and launch the desktop app with:

```bash
cargo run --release -p termy
```

See the [installation guide](https://termy.sh/docs/getting-started/installation) for platform-specific steps.

## What you can shape

Termy keeps its behavior in plain configuration rather than burying it in hidden application state.

| Surface | What you control |
| --- | --- |
| Appearance | Themes, colors, fonts, chrome contrast, and tab presentation |
| Input | Keybindings, terminal behavior, mouse reporting, and shortcuts |
| Workspace | Tabs, split panes, tasks, reusable layouts, and working directories |
| Sessions | Local shells and optional tmux-backed sessions |

Start with [Customize Termy](https://termy.sh/docs/customize) or use the complete [configuration reference](https://termy.sh/docs/reference/configuration-reference).

## Architecture

Termy is more than a window around a PTY. Its terminal emulation uses Termy's own streaming parser, screen storage, and native PTY runtime. The repository also contains a GPUI desktop application, CLI, native FFI, website, and release tooling.

```text
desktop / embedding hosts
        │
        ├── terminal UI and platform integration
        │
        ├── reusable command, search, theme, and release crates
        │
        └── terminal runtime, PTY, parser, and rendering snapshots
```

Read [Project Layout](./docs/architecture/project-layout.md) for ownership boundaries and [Release Packaging](./docs/architecture/release-packaging.md) for artifact flow.

## Sponsors

Termy is supported by companies and people who care about fast, native developer tools.

<p align="center">
  <a href="https://neon.tech">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="./assets/legends/neon-logo-dark-color.svg" />
      <source media="(prefers-color-scheme: light)" srcset="./assets/legends/neon-logo-light-color.svg" />
      <img alt="Neon" src="./assets/legends/neon-logo-light-color.svg" width="157" />
    </picture>
  </a>
  &nbsp;&nbsp;&nbsp;&nbsp;
  <a href="https://github.com/mezotv">
    <img alt="Dominik Koch" src="https://github.com/mezotv.png" width="64" />
  </a>
</p>

## Termy X

Termy X is a fork layer on [Termy](https://github.com/lassejlv/termy) (MIT, © Lasse Jørgensen). The terminal itself is unchanged. X reads, drafts, and publishing live in `crates/x_core` so upstream merges stay small. The desktop app gains a side panel; the CLI gains `termy x` and a standalone `x` binary.

No X API credits are required. The official API is off unless you set `official_api.enabled = true`.

### Commands

```bash
x doctor
x timeline
x search "gpui" --recent --limit 10
x lookup @termy
x post 1001
x trends --place worldwide
x popular gpui
x draft "ship the panel" --n 3 --tone punchy
x compose --text "hello from the terminal"
x publish <draft-id>          # asks [y/N], prints the exact text first
x publish <draft-id> --dry-run
x research "gpui terminals"
termy x doctor                # same commands through the Termy CLI
```

`--json` prints machine-readable output. `--provider mock` forces fixtures (what the tests use). Tones for `x draft` are `punchy`, `informative`, `thread`, and `reply`.

`x publish` never posts until you answer `y`. It prints the exact text, the weighted character count (URLs count as 23, limit 280), and the web-intent URLs. `--dry-run` stops after that.

### Free providers

Reads try, in order:

1. [OpenCLI](https://github.com/jackwener/opencli) `twitter` adapter (`profile`, `thread`, `search`, `timeline`, `trending`). Uses your logged-in Chrome session and the Browser Bridge extension. Termy X does not automate login or store passwords.
2. [twitter-cli](https://github.com/public-clis/twitter-cli), the X path used by [Agent Reach](https://github.com/Panniantong/Agent-Reach). Auth is your own cookie (`TWITTER_AUTH_TOKEN` and `TWITTER_CT0`, or the browser cookie jar). Trends are derived from search results ranked by engagement when a trends endpoint is missing.
3. Built-in mock fixtures, so the app and tests run with nothing installed.

Publishing tries, in order, and still requires the confirm step:

1. `opencli twitter post` / `opencli twitter reply`
2. `twitter post` / `twitter post --reply-to`
3. An X web intent: `https://x.com/intent/post?text=...` (and `in_reply_to` on the first post of a thread). Termy X opens it in the browser; you click Post. The panel's "Next post" button copies the next intent URL.

`x doctor` reports which of these are installed and states that no X API credits are needed. Cookie and browser-session providers are labeled "uses your own logged-in session".

### Optional official API

Create an app at the X developer portal, set `official_api.enabled = true`, and export the client id:

```bash
export TERMY_X_CLIENT_ID=...
x auth
```

`x auth` prints an OAuth 2.0 PKCE authorize URL (scopes include `tweet.read` and `tweet.write`) and can exchange a code into the OS keychain (service `termy-x`) or a `0600` file. The localhost callback listener is not started for you; paste the redirect yourself only if you finish the exchange out of band. Cost figures in the status line are labeled estimates (post read $0.005, user read $0.010, post write $0.010) and are not a bill.

### AI drafts

Drafts stay drafts. Nothing is auto-posted.

```bash
export TERMY_X_AI_BASE_URL=http://127.0.0.1:11434/v1   # Ollama
export TERMY_X_AI_MODEL=llama3.2
# export TERMY_X_AI_API_KEY=...   # Groq or OpenRouter, when the server requires one
x draft "ship the panel" --tone informative
```

Other free OpenAI-compatible servers: Groq (`https://api.groq.com/openai/v1`) and OpenRouter free models (`https://openrouter.ai/api/v1`). If those variables are unset, `x draft` uses local templates and labels them `template`, not a model. Prompt text lives in `crates/x_core/prompts/default.toml`.

### Config

`x config` prints an example. The file is `~/.config/termy/x.toml`.

| Key | Default | Meaning |
| --- | --- | --- |
| `background_art` | `true` | Geometric swift behind the panel, and the CLI splash |
| `provider` | `auto` | `auto`, `opencli`, `twitter`, `mock`, or `official` |
| `default_limit` | `10` | Page size, clamped to 20 on the CLI |
| `official_api.enabled` | `false` | Turn on the paid API |
| `ai.base_url_env` | `TERMY_X_AI_BASE_URL` | Env var that holds the chat endpoint |
| `shell.min_interval_ms` | `1500` | Pause between shell-outs |

### Desktop panel

Open it with `Ctrl+Shift+X` (`Cmd+Shift+X` on macOS) or the command palette entries under X: Timeline, Search, Trends, Lookup, Compose, Research. Compose shows a live preview, the weighted counter, thread splits, and "Suggest variants". Publish opens a confirm dialog with the exact text.

The background is an original low-poly swift (electric blue and cyan, magenta heart with a `+1`) in `assets/x/termy-x-bird.svg`, drawn for a 3440×1440 frame and scaled with object-fit cover. Behind lists it sits under a dark veil so the art reads at about 20% strength. The empty splash shows it at full strength. "Art on/off" writes `background_art` into `x.toml`. The CLI prints a small ANSI bird when that flag is on.

Preview the panel on fixture data:

```bash
cargo run -p termy --bin x_panel_preview -- timeline
TERMY_X_PREVIEW_WIDTH=3440 TERMY_X_PREVIEW_HEIGHT=900 cargo run -p termy --bin x_panel_preview -- splash
```

## Roadmap and contributing

- [Engineering quality roadmap](./docs/engineering/roadmap.md)
- [Contributor setup and validation](./CONTRIBUTING.md)

Contributions are welcome. Keep changes scoped, run the nearest validation command, and preserve the boundaries documented in the architecture guide.

<p align="center">
  <sub>MIT licensed · Built in Rust · <a href="https://termy.sh">termy.sh</a></sub>
</p>
