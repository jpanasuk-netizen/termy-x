# Termy X desk port (bangerkit-port) - local only, not pushed

Base: main 33dcd5f (= ultimate-final, newest). All X reads/writes via OpenCLI. No paid API.

## Commits
- c14e6d9 x_core inbox/feeds/stats/followers/queue/time + desk panel tabs + `x` CLI subcommands
- 259eecf fix: desk tabs were hidden behind the wide preview dashboard
- e06d973 fix: analytics excluded reposts (RTs were counted with the original author's numbers)

## Features
Inbox (Waiting/Done, AI draft, Reply -> confirm, Queue), Mentions, Feeds (lang/age/mute), Stats (per-day vs prior, best-hour heatmap, top posts), Followers (snapshot on click only, new/gone diff), Queue (Send -> confirm, Remove), Analyze (14-day read of any handle).
Never auto-posts/follows. Every send goes through the confirm screen (GUI) or "type yes" (CLI).

## Verified
- Real OpenCLI reads (CLI, live session @Jasper_Black): inbox (28 items), mentions, analytics 7d vs prior, analyze @elonmusk 14d. Outputs in cli/ (local only, not committed: contains other users' public posts).
- Queue send --dry-run: plan + in_reply_to intent URL, "nothing was posted". Non-dry send answered "no": "not posted".
- GUI (x_panel_preview --mock): every new tab renders; Inbox Reply opens the confirm screen with in_reply_to (v2/02c, 02d).
- cargo fmt, cargo clippy --workspace --all-targets -D warnings: clean.
- cargo test --workspace: all pass except termy_core ffi::tests::terminal_take_frame_update_supports_full_path (terminal FFI, not touched by this work; only clippy-style edits in core).

## Gaps
- GUI click-testing was done in the mock preview, not live termy.exe; blind coordinate clicks in live mode risk hitting Post now, so live GUI was not auto-clicked. Live reads were proven via CLI using the same x_core code.
- Dry-run button click result in GUI not captured (dialog shifted); dry-run proven via CLI.
- Feeds/Stats Load/Followers Snapshot/Analyze buttons: tabs captured, button results not screenshotted.
- BTCC's partial branch was not on GitHub (likely in HermesTools, which was off limits); built fresh.

## Launch
C:\Users\jpana\work\termyx-bk\target\release\termy.exe  (Ctrl+Shift+X opens the X panel)
CLI: C:\Users\jpana\work\termyx-bk\target\release\termy-xcli.exe inbox | mentions | analytics | analyze <handle> | queue
