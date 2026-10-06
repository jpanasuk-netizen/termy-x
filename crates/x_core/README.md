# termy_x

Headless Termy X library: provider trait, free-path fallback, mock fixtures, twitter-text v3 counting, publish confirmation, and AI draft requests.

Free reads try OpenCLI, then twitter-cli (Agent Reach), then mock fixtures. The official X API is off unless `official_api.enabled` is set. Publishing tries OpenCLI and twitter-cli only when their help text lists a `post` command, then the X web intent. Nothing is sent without an explicit `yes`. `--dry-run` never sends.

`x doctor` reports OpenCLI, twitter-cli / Agent Reach, mock, and intent as `working`, `missing`, or `not logged in`. It always includes the line `No X API credits are needed.`

AI drafts use an OpenAI-compatible endpoint. `TERMY_X_AI_BASE_URL`, `TERMY_X_AI_API_KEY`, and `TERMY_X_AI_MODEL` are preferred. `OPENAI_BASE_URL` and `OPENAI_API_KEY` are also read. The free default is Ollama at `http://127.0.0.1:11434/v1` with `llama3.2`. Other free servers: LM Studio (`http://127.0.0.1:1234/v1`), Groq's free tier (`https://api.groq.com/openai/v1`), OpenRouter models whose ids end in `:free` (`https://openrouter.ai/api/v1`), and llama.cpp. If none of those answer, drafts fall back to offline templates.

On Windows the command runner follows PATHEXT and prefers `opencli.cmd` over the extensionless shell shim npm installs beside it. It still runs a `.ps1` shim when that is the match. Children start with `CREATE_NO_WINDOW` (`0x08000000`) and a 25 second timeout. Doctor and the adapters only invoke read commands unless you type `yes` on a publish.

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
