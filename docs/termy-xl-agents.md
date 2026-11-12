---
name: termy-desk
description: Draft X replies inside Termy XL with Grok, Hermes, Agy, or Muse. Hermes on Windows uses the local gateway. Nothing is posted except the compose card's Send button.
---

# Termy XL agents

Termy XL and Termy X stay separate. This skill drafts replies in Termy XL. It does not post, and it does not change Termy X, its shortcut, or `%APPDATA%\termy`.

Termy XL stores its own settings in `%APPDATA%\Termy XL\x.toml`.

## Open a pane

In the compose card, **Open Grok**, **Open Hermes**, **Open Agy**, or **Open Muse** starts that agent's pane. The pane shows the command, a transcript, and a line you can type. **Use as draft** copies the typed line, or the last reply in the transcript, into the compose box. That copy is not a post.

If the program is not installed, the pane shows one line and stays open:

`Hermes is missing: hermes was not found.`

The same shape is used for Grok, Agy, and Muse. Termy XL does not crash when a binary is missing.

## Commands

Defaults when `%APPDATA%\Termy XL\x.toml` does not set one:

| Agent | Command |
| --- | --- |
| Grok | `grok` |
| Hermes | `hermes` |
| Agy | `agy` |
| Muse | `muse` |

Hermes is the Windows-native CLI. The known install path is `%LOCALAPPDATA%\hermes\bin\hermes.exe`. Termy XL uses that file when it is present. Do not launch Hermes through `wsl.exe`.

Override a command under `[agents]` in the Termy XL file only:

```toml
[agents]
grok = "grok"
hermes = "hermes"
agy = "agy"
muse = "muse"
```

A saved command wins over the default. If that program is not on PATH and is not the known Hermes exe, Termy XL shows the missing line above.

## Draft

**Draft with Grok**, **Draft with Hermes**, **Draft with Agy**, or **Draft with Muse** sends the original post and the compose card's tone. The ask is for 3 different replies (agree and add, question, contrarian or witty), separated by a line that contains only `---`.

Termy XL keeps three replies that are not the same sentence. If the agent repeats a line, the duplicate is dropped and a different fallback line is used. If the agent errors, including HTTP 402, the card still gets three different fallback replies built from the post, and the error is one line on the card.

Those replies fill the compose card. They are not posted.

## Hermes gateway

Termy XL runs on Windows and talks to the Hermes 0.23.0 gateway on this machine. There is no WSL port to bridge.

- Base URL: `http://127.0.0.1:8642/v1`
- `POST /v1/chat/completions`
- `POST /v1/responses`
- `GET /v1/models`
- `GET /v1/capabilities`
- `GET /health` returns 200 and does not use auth. Termy XL's Hermes status pill calls `/health`.

`/v1` requires `Authorization: Bearer <API_SERVER_KEY>`.

Key order:

1. Use the Hermes key saved in Termy XL settings, if there is one.
2. Otherwise read `API_SERVER_KEY` from `%LOCALAPPDATA%\hermes\.env` at request time.

Do not log the key. Do not copy it into `x.toml` unless the user presses **Save Hermes**. Do not commit it.

Default model is `hermes-agent`. `/v1/models` lists only that name. It is the profile's primary model (currently vyce `glm-5.3-flash`).

A bare model name `glm-5.3-flash` is ignored by the gateway. To ask for that model directly, send:

```json
{"provider":"vyce","model":"glm-5.3-flash"}
```

The gateway honors that form. Termy XL sends `hermes-agent` unless the Hermes model setting is `glm-5.3-flash`, `vyce:glm-5.3-flash`, or `vyce/glm-5.3-flash`, in which case it sends the provider form above.

Settings in Termy XL, written only when the user saves:

```toml
[agents]
hermes_base_url = "http://127.0.0.1:8642/v1"
hermes_model = "hermes-agent"
```

`hermes_api_key` is added only by **Save Hermes** when the field is not empty. Clearing the field and saving removes it from the Termy XL file. The `.env` value is still read at request time.

Grok, Agy, and Muse draft through their CLI unless a future command override says otherwise. Hermes drafts through the gateway above, so a missing `hermes.exe` does not block **Draft with Hermes** when the gateway and key are available. **Open Hermes** still needs the CLI.

## Hermes takes provider

The header offers **Free takes**, **Treg takes**, and **Hermes takes**. The choice is saved as `desk.provider` in `%APPDATA%\Termy XL\x.toml` only. Termy X's config is not written.

Hermes takes call the OpenAI-compatible gateway. The default base URL is `http://127.0.0.1:8642/v1`. Change `agents.hermes_base_url` when the gateway is published somewhere else, including when Hermes Agent runs in WSL and the reachable address is not that default. The URL is a setting. It is not the only address the code can call.

On select, Termy XL sends `GET {base}/models`. The status line is one of:

- `Hermes · hermes-agent @ http://127.0.0.1:8642/v1`
- `Hermes · needs a key (401)`
- `Hermes · needs credits (402)`
- `Hermes · unreachable (connection refused)`

A 401 or 402 stops further Hermes calls for that session. The card still gets three different fallback replies from the post. Nothing is posted.

## termy-xcli desk

Agents call `termy-xcli` (never `x.exe`). `desk` is the short name of `trends-desk`. Every command below prints JSON and sets `"posted": false`. They read and write `%APPDATA%\Termy XL` (override with `--data-dir` or `TERMY_XL_DATA_DIR`). They do not post. Posting stays on the compose card's **Send** button.

```text
termy-xcli desk search --subject Law --json
termy-xcli desk takes --tweet 77 --provider hermes --n 3 --json
termy-xcli desk takes --tweet https://x.com/ada/status/77 --provider free --n 3 --json
termy-xcli desk drafts list --json
termy-xcli desk drafts save --text "A queued reply" --tweet 77 --json
termy-xcli desk queue-reply --tweet 77 --text "A queued reply" --json
```

`--provider` is `free`, `treg`, or `hermes`. `takes` uses a tweet already stored by a desk search. `queue-reply` and `drafts save` replace the one draft for that tweet. The draft shows up in Termy XL. It is not sent.

Search:

```json
{"ok":true,"posted":false,"subject":"Law","topic":"law","kind":"fresh","message":"1 source-linked post(s)","posts":[{"id":"77","author":"ada","text":"Courts are testing models","url":"https://x.com/ada/status/77","likes":4}]}
```

Takes:

```json
{"ok":true,"posted":false,"provider":"hermes","tweet":"77","url":"https://x.com/ada/status/77","note":"Hermes draft (http://127.0.0.1:8642/v1). Not posted.","variants":["Agree, and one thing to add.","What changes if this is only half true?","The cost is the part left out."]}
```

Queue:

```json
{"ok":true,"posted":false,"id":"ab12cd34","text":"A queued reply","reply_to":"77"}
```

Draft list:

```json
{"ok":true,"posted":false,"drafts":[{"id":"ab12cd34","text":"A queued reply","reply_to":"77","source":"termy-xl-queue","created_at":"1730000000"}]}
```

## What posts

Only the compose card's **Send** button posts, and only for that card. Esc, Close, **Use as draft**, opening a pane, **Draft with …**, and every `termy-xcli desk` command leave the text unsent.
