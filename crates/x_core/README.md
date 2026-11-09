# termy_x

Headless Termy X library: provider trait, free-path fallback, mock fixtures, twitter-text v3 counting, publish confirmation, and AI draft requests.

Free reads try OpenCLI, then twitter-cli (Agent Reach), then mock fixtures. The official X API is off unless `official_api.enabled` is set. Publishing tries OpenCLI and twitter-cli only when their help text lists a `post` command, then the X web intent. Nothing is sent without an explicit `yes`. `--dry-run` never sends.

`x doctor` reports OpenCLI, twitter-cli / Agent Reach, mock, and intent as `working`, `missing`, or `not logged in`. It always includes the line `No X API credits are needed.`

AI drafts use an OpenAI-compatible endpoint. `TERMY_X_AI_BASE_URL`, `TERMY_X_AI_API_KEY`, and `TERMY_X_AI_MODEL` are preferred. `OPENAI_BASE_URL` and `OPENAI_API_KEY` are also read. The free default is Ollama at `http://127.0.0.1:11434/v1` with `llama3.2`. Other free servers: LM Studio (`http://127.0.0.1:1234/v1`), Groq's free tier (`https://api.groq.com/openai/v1`), OpenRouter models whose ids end in `:free` (`https://openrouter.ai/api/v1`), and llama.cpp. If none of those answer, drafts fall back to offline templates.

On Windows the command runner follows PATHEXT and prefers `opencli.cmd` over the extensionless shell shim npm installs beside it. It still runs a `.ps1` shim when that is the match. Children start with `CREATE_NO_WINDOW` (`0x08000000`) and a 25 second timeout. Doctor and the adapters only invoke read commands unless you type `yes` on a publish.

## Sync across PCs

Account sync is optional. Open the X panel, go to Help, and use **Account sync (optional)**. Type an email and a password, then choose Sign in or Create account. The status line reads off, syncing, synced at a time, or a short error. If the project asks you to confirm the address, the line says to check your email and then Sign in. Forgot password sends a reset email. Sign out forgets the session on this PC only. The password is not saved. Likes, follows, reposts, and receipts stay in the local database and copy to your other PCs after each of them is signed in.

A release build includes the hosted project. Set `TERMY_X_SUPABASE_DEFAULT_ANON_KEY` when building a release so that key is compiled in. The placeholder `REPLACE_WITH_TERMY_X_SUPABASE_DEFAULT_ANON_KEY` is not a key; without a real one, sync stays off. Another project can be saved with `termy-xcli supabase configure --url https://YOUR_PROJECT.supabase.co`.

## Optional Supabase sync

Action state (likes, reposts, follows, and post/reply/quote receipts) stays in the local SQLite file. Sync is off until three things are saved on that machine:

- `supabase_url` in `x.toml` (the project URL only, not a key)
- the anon/publishable key, stored in the OS keychain or a private `0600` file
- a Supabase Auth email/password session

The password is used once for `grant_type=password` and is not stored. The access token is stored in the same secret store. Neither value is written to `x.toml` or the action log.

```sh
termy-xcli supabase configure --url https://YOUR_PROJECT.supabase.co
# with TERMY_X_SUPABASE_ANON_KEY set in the environment
termy-xcli supabase sign-in --email you@example.com
# with TERMY_X_SUPABASE_PASSWORD set in the environment
termy-xcli supabase status
```

`status` prints off, syncing, synced at a time, or error. It does not print the key or the session. A failed push stays in a local outbox and is retried. The app keeps working when Supabase is not configured or unreachable. Sync never posts, likes, or follows on X.

The SQL for the project is `supabase/migrations/20261109124500_x_action_state.sql`. Apply that file in the Supabase SQL editor or as a migration. It creates `x_action_toggles` and `x_action_receipts`, turns on row level security for both, and allows each signed-in user to read and upsert only their own rows.

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
