# Project Layout

The workspace has three Termy packages: desktop, CLI, and core. Add modules within the
appropriate package instead of creating a crate for each helper.

Termy X adds two more crates so the terminal stays easy to merge from upstream:
`crates/x_core` (`termy_x`, headless) and `crates/x_cli` (the `x` binary). The
desktop panel lives in `crates/desktop_app/src/x_panel` and only calls `termy_x`.

## Desktop: `crates/desktop_app` (`termy`)

- `src/terminal_view/`: windows, tabs, panes, terminal interaction and persistence.
- `src/settings_view/`, `src/onboarding/`, and `src/ui/`: application workflows and presentation.
- `src/terminal_ui/`: GPUI terminal grid, pixel snapping, input adapters, and tmux pane display.
- `src/design_system/`: stateless GPUI controls, tokens, metrics, and icons.
- `src/native_sdk/`: platform clipboard, permissions, and file-manager integration.
- `src/auto_update/`: update checks and installation coordinated with GPUI.

The library target exposes presentation helpers to the desktop binary, examples,
and integration tests. It is part of the same package.

## Core: `crates/core` (`termy_core`)

Core is headless and shared by desktop and CLI. Its existing terminal API remains
at the library root. Supporting modules are grouped by responsibility:

- `multiplexer/`: persistent session host, authenticated IPC, clients, and layout operations.
- `session_model/`: saved windows, workspaces, tabs, panes, and split geometry.
- `terminal_engine/`: terminal engine and native PTY/ConPTY runtime.
- `tmux_control_core/`: tmux control-mode protocol and transport.
- `config_core/`, `command_core/`, `theme_core/`, `themes/`, and `search_engine/`: shared configuration, commands, themes, and search.
- `plugin_runtime/` and `ssh_core/`: plugin execution and SSH host management.
- `cli_install_core/` and `release_core/`: installation and release helpers.
- `ffi/`: C ABI, with its public header at `include/termy.h`.

`termy-session-host` remains a standalone binary target inside this package.
The shared library artifact is `libtermy_core` (`termy_core.dll` on Windows);
C exports and header contracts are unchanged.

## CLI: `crates/cli` (`termy_cli`)

- `src/commands/`: terminal launch, plugin and config commands, and JSON multiplexer controls.
- `src/xtask/`: repository automation, generated documentation, dependency checks, and performance tooling.

The default binary is `termy-cli`. Run repository tooling with
`cargo run -p termy_cli --bin xtask -- <command>`. `termy x` delegates to the
same command set as the standalone `x` binary.

## Termy X: `crates/x_core` (`termy_x`) and `crates/x_cli`

- `crates/x_core`: provider trait, free-path fallback, mock fixtures, weighted length, publish confirmation, drafts, and AI requests. No GPUI.
- `crates/x_cli`: the `x` executable. Behavior stays in `termy_x`.
- `crates/desktop_app/src/x_panel`: the side panel. Background art is `assets/x/termy-x-bird.svg`.
- `crates/desktop_app/src/bin/x_panel_preview.rs`: opens the panel on fixture data for review.

## Repository support

`docs/` contains contributor documentation; `website/` contains public docs.
`assets/` holds application assets; `assets/schemas/theme.schema.json` defines the theme JSON schema. `scripts/` owns packaging and platform checks.
Benchmark examples live in `crates/cli/examples/`.

## Boundaries and validation

Core and CLI must remain free of GPUI and desktop dependencies. The terminal
engine and command catalog remain independent modules; design-system components
must not reach into application state. `scripts/check-boundaries.sh` checks these
rules along with packaging, generated docs, and dependency policy.

```sh
cargo check --workspace --all-targets
cargo test --workspace
just check-boundaries
```

See [Release Packaging](release-packaging.md) and
[Testing](../engineering/testing.md) for platform-specific validation.
