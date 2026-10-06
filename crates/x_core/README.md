# termy_x

Headless Termy X library: provider trait, free-path fallback, mock fixtures, weighted character counting, publish confirmation, and AI draft requests.

## Owner

This crate owns X read/search/publish planning for the `x` CLI and the desktop panel. It must stay free of GPUI so the desktop app can depend on it without pulling UI into the CLI. Official X API access is implemented here and is off unless `official_api.enabled` is set. Default reads are OpenCLI, then twitter-cli / Agent Reach, then the mock provider. Default publishing is OpenCLI, then twitter-cli, then an X web intent.

## Validation

```sh
cargo test -p termy_x
```

## Forbidden Dependencies

- `gpui`
- `gpui-kit`
- `termy` / `crates/desktop_app`
