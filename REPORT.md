# Termy X — build report

Built on LightBringer (Windows 11, Rust 1.98.1 stable-x86_64-pc-windows-msvc, VS 2022 Build Tools) on 2026-10-06.
The Grok Build CLI (`grok` 1.0.46, Windows native) ran on LightBringer as the coding engine, headless and hidden, in three focused prompt chunks (core/CLI, GUI/art, palette test fix). Grok Bot drove the runs and checked the results.
Starting point: the recovered Cursor-run tree (commits `6206e263` + WIP `447e4db7`), sent over as a git bundle on top of upstream Termy `308bc090`.

Layout: Jeremy changed the spec mid-build. The app is **not** ultrawide-first. It uses a normal desktop layout (preview default 1600x1000) that goes from 3 columns to 2 to 1 as the window narrows.

## What works
- **Free providers by default, in order:** OpenCLI (your logged-in Chrome session) → twitter-cli / Agent Reach (your own cookie) → mock. The Windows runner prefers `opencli.cmd` and also handles `.ps1` shims. Child processes start with `CREATE_NO_WINDOW` and a 25 s timeout.
- **Official X API v2 (OAuth2 PKCE):** optional and **off** by default. A config flag turns it on, and it then goes first.
- **Reads:** user lookup, post/thread, search (latest/top), timeline, trends. "Popular" means search results ranked by engagement. The Research tab covers web, Reddit, YouTube and RSS.
- **Publish confirm gate:** nothing is published until the exact text of every part has been shown.
  - CLI: prints each part with its weighted count, then requires typing `yes`. `--dry-run` never publishes.
  - GUI: a confirm modal shows the exact text, with Copy and Next post buttons.
- **Publish order:** OpenCLI post (only if its help lists `post`) → twitter-cli post (only if supported) → X Web Intent `https://x.com/intent/post?text=…&in_reply_to=…`, which is always available. Threads go out one intent at a time, using Next post and Copy.
- **Weighted counting (twitter-text v3, limit 280):**
  - Latin text counts 1 per character. CJK and other non-Latin characters count 2.
  - Any URL counts 23.
  - An emoji counts 2. A ZWJ sequence, skin-tone emoji or flag counts once, as 2.
  - Text is NFC-normalized first. Thread splitting keeps every part at 280 or less.
- **AI drafting:** any OpenAI-compatible endpoint, set with `TERMY_X_AI_BASE_URL`, `TERMY_X_AI_API_KEY` and `TERMY_X_AI_MODEL` (`OPENAI_BASE_URL` and `OPENAI_API_KEY` also work).
  - The free default is local Ollama at `http://127.0.0.1:11434/v1` with model `llama3.2`. If nothing is reachable, it falls back to offline templates.
  - Free options (Ollama, LM Studio, llama.cpp server, OpenRouter `:free` models, Groq free tier) are documented in `README.md` and `crates/x_core/README.md`.
- **`x doctor`:** reports each free provider, the AI endpoint, art settings, `Official X API: off`, and **"No X API credits are needed."**
- **Unicode CLI splash** (`x`, `x splash`): a small block-character bird with a ♥ +1. Respects `NO_COLOR`.
- **Bird art** (`assets/x/termy-x-bird.svg`): an original faceted swift in neon blue and cyan, with a magenta heart and +1 badge, on a navy grid and starfield. It is not the Twitter bird or the X logo.
  - The SVG is parsed once and rasterized once at 1600x1000 (centered cover crop), then cached in a process-wide `OnceLock`. Resizing does not rasterize again.
  - It shows at full strength on the splash and at `background_opacity` (default 0.18) behind content.
  - `background_art = false` in `~/.config/termy/x.toml`, or the "Art on/off" button, turns it off and skips rasterization.
- **Main Termy app:** the X panel opens with Ctrl+Shift+X (`toggle_x_panel`) or from the command palette, which has its own "X" category. `termy x …` runs the same CLI.

## Bird render timing
- Old: 6880x2880 in about 30 s (from the Cursor run).
- New: **1600x1000 in 41 ms** (release, measured and written to `%TEMP%\termy-x-art-timing.txt`). The debug build took 2.4 s.

## Stubbed or limited
- Official API token exchange is out of band, and the API stays off by default.
- Live publishing was **never run**. Testing used only dry runs, unit tests and the intent URL builder.
- The GUI confirm path ends at the web intent. It does not call OpenCLI or twitter-cli posting.
- Agent Reach is the twitter-cli path; there is no separate binary. RSS research is a title scrape.
- The X panel embedded in the main Termy window is a single 480 px column. The multi-column layout is in `x_panel_preview` and on wide panels.
- On the splash, the headline text sits on top of the bird's upper wing. It is readable, but the art could be moved down slightly.

## How to run (Windows, from the repo root)
```
cargo build --release
target\release\x.exe doctor
target\release\x.exe splash
target\release\x.exe search "gpui terminal"
target\release\x.exe post --dry-run "hello"      # shows the exact text + intent URL; nothing is posted
target\release\termy.exe                          # main app; Ctrl+Shift+X toggles the X panel
target\release\x_panel_preview.exe --splash       # splash, full-strength bird (default 1600x1000)
target\release\x_panel_preview.exe --mock         # panel with fixture results over the dimmed bird, no network
target\release\x_panel_preview.exe --mock --width 1280 --height 800
```
The release `x_panel_preview` uses `windows_subsystem = "windows"`, so no console window opens.

## Test results (`cargo test --workspace --no-fail-fast`, Windows)
- termy_x (x_core): **42 passed, 0 failed.** Covers fallback order (reads and publish), intent URL encoding including `in_reply_to`, weighted counting with 280/281 boundaries, emoji sequences, CJK, URLs, thread splitting, the confirm gate, doctor, splash and config.
- termy (desktop bin): **865 passed, 0 failed.** Two palette tests broke when the "X" category was added; they're fixed in `2f654c0e`.
- termy (desktop lib): **195 passed, 1 failed.** The failure is `terminal_ui::tmux::client::tests::new_reports_unsupported_platform_on_non_unix`. It already fails in upstream Termy on Windows; the file is identical to upstream `308bc090`.
- termy_core 704/704, termy_cli 29/29, xtask 33/33, integration suites (mux, plugins, display_terminal, ffi_export_guard, kitty_*, remote_*, resize_clear) all pass.
- **Total: 1908 passed, 1 failed (upstream, Windows-only).**
- The release build (`cargo build --release`) finishes cleanly.

## Screenshots (real Windows captures from LightBringer, 1600x1000 client area)
- `screenshots/splash-1600x1000.png`: splash with the full-strength bird
- `screenshots/panel-mock-1600x1000.png`: 3-column panel with mock results over the dimmed bird
- `screenshots/x-doctor.txt`, `screenshots/x-splash.txt`, `screenshots/x-post-dry-run.txt`: CLI output

## Commits (local, on `main`)
- `2f654c0e` command_palette: X category tint + test update
- `4ab07a84` x_panel_preview: splash/mock flags
- `b0703506` x_panel: responsive layout + confirm modal
- `5f4638bc` x_panel: cached bird raster
- `c2b8dcbf` x_cli: doctor, splash, dry-run
- `7d20f7bc` x_core: provider order + windows runner
- `3c05ffce` x_core: twitter-text v3 counting + tests
- `ec664dd7` x_core: compile the token-file test on Windows
- `447e4db7` WIP: bird raster cache (art.rs), AI/config tweaks (from the recovered Cursor run)
- `6206e263` Recover Termy X work from stopped Cursor run (bc-354a166c)
- `ef07447f` Merge Termy upstream/main into Termy X
- `498ab8af` Initialize project
- plus the commit that adds this REPORT.md and the screenshots

## Push status
**Not pushed.** One non-interactive `git push origin main` was attempted (GIT_TERMINAL_PROMPT=0, GCM_INTERACTIVE=never, 60 s timeout). It failed with `fatal: could not read Username for 'https://origin.cursor.com': terminal prompts disabled` (exit 128). Git Credential Manager has no stored credential for origin.cursor.com, so the commits stay local. No token was created or pasted.
Remotes: `origin` = https://origin.cursor.com/jeremy-panasuk/termy-x.git, `upstream` = https://github.com/lassejlv/termy.git.
To publish, run `git push origin main` once from an interactive terminal in the repo and complete the Google sign-in when Git Credential Manager prompts for it.

## Compose caret / first-letter fix (2026-10-06, commit 07d9251d)

**Symptoms:** Empty compose showed the caret on the right; the first typed letter (often a capital) disappeared.

**Root cause:**
1. Empty-state render placed the placeholder *before* the caret, so the blink sat at the end of the hint line instead of index 0 on the left.
2. `on_key` bailed on `mods.modified()`, which includes Shift — so Shift+letter (sentence capitals) never reached `insert_text`. Prefer `key_char` now; only ctrl/alt/platform/function block plain text.

**Fix:** Caret-first empty layout; Shift-safe insert via `key_char`; `stop_propagation` on compose editing keys. Release `termy.exe` rebuilt. Not pushed.
