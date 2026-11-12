# Trends Desk contract

The desktop window is not an HTTP server. These are the behaviors the old local desk exposed, mapped onto the Rust add-on. The Node server on port 3489 is not part of this contract.

## Freshness

| State | Meaning | Writes a report |
| --- | --- | --- |
| Loading | OpenCLI call is in flight on a background thread. | No |
| Fresh | This run returned one or more posts with an X status URL. | Yes |
| Cached | A report loaded from disk, or still on screen under an empty/error banner. | No |
| Empty | OpenCLI returned no usable posts. | No |
| Error | OpenCLI was missing, `ok: false`, malformed output, timeout, or cancel. | No |
| Fixture | `--fixture` only. Labeled synthetic rows. | No |

A cached card is never relabeled fresh because an older file exists. Empty and error keep the last saved cards on screen and say they are not a new run.

## OpenCLI shapes

Search stdout is parsed from the first JSON value. Accepted shapes:

- `{ "ok": true, "data": [ posts ] }` (OpenCLI 2.0.2)
- `{ "ok": true, "data": { "tweets" \| "results" \| "items": [ posts ] } }`
- `{ "tweets": [ posts ] }`, `{ "results": [ posts ] }`, `{ "items": [ posts ] }`
- a bare array
- a JSON string inside `data` that itself parses as one of the above

`ok: false` is an error. The message is the `error`, `message`, or string `data` field, shortened and redacted. Empty `data: []` is an empty result, not a success fixture. Text that is not JSON is an error.

Search arguments, one vector, no shell:

```text
opencli twitter search <query> --product top --limit 24 -f json
```

The query is a single argument. `focus` is appended when it is non-empty and not already contained in the column description. The result is at most 180 characters, and the focus term is kept by shortening the description first.

Thread inspection:

```text
opencli twitter thread <id> --limit 25 --top-by-engagement 8 -f json
```

Reply dry-run text is only displayed. The desk does not execute `twitter reply`.

Readiness is `opencli twitter whoami -f json` when the binary exists. A missing binary is reported before a search. `whoami` failure does not invent posts.

Timeouts: 120 seconds for search and thread, 25 seconds for whoami. Cancel drops the result. A normal OpenCLI child is stopped by the runner timeout. The Hermes `node` path also stops the child when you press Cancel.

## Posts

A row is kept when it has text and a status URL on an X host (`x.com`, `twitter.com`, `mobile.twitter.com`, `mobile.x.com`, and the `fx`/`vx` twitter hosts). `twitter.com` links are canonicalized to `https://x.com/.../status/<id>`. The original URL string and the original `created_at` string are kept. Posts that are only an image quote, with no text, are dropped. The footer says they are not shown.

## Files

Under the Termy config directory (`%APPDATA%\termy` on Windows, `$XDG_CONFIG_HOME/termy` or `~/.config/termy` elsewhere), in `trends-desk/`:

| File | Contents |
| --- | --- |
| `settings.json` | `focus`, `reply_as`, `daily`, `daily_at`, `daily_done`, `auto`, `every_minutes`. No API key. |
| `topics.json` | `{ id, name, desc }`. Defaults: law, finance, ai. |
| `reports/<id>-<unix>.json` | Fresh run only: query, focus, posts, markdown. |
| `reports/<id>-<unix>.md` | The same markdown. |

Drafts go to the existing `x-drafts.json` with `source` `trends-desk`. Column ids are short lowercase slugs. A path cannot escape `reports/`.

Daily runs, while the window is open, once per America/Chicago calendar day, inside 90 minutes after `daily_at`. The headless CLI does not schedule itself.

## CLI JSON

`termy-xcli trends-desk run --json` prints `kind` of `fresh`, `empty`, or `error`, `wrote_report`, the query, and status URLs. It does not print cookies, tokens, or a key.
