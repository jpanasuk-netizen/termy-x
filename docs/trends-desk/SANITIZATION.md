# Sanitization

The add-on was ported from the sanitized snapshot `trends-desk-addon-reference-2026-11-11`. That zip is not committed. This tree does not add the excluded live files.

## Not in this repository

- `.xai-key`, `.env`, cookies, Chrome profiles, OpenCLI session stores
- private `reports/`, `replies.json`, `usage.jsonl`, live `settings.json`, logs
- compiled starters, desktop screenshots of the old browser desk
- account home paths and the live post-as handle

Settings examples in code use `@YourHandle`. The paid model id from the old settings file is not a desk setting and is not read.

## What was kept

- OpenCLI 2.0.2 `{ "ok": true, "data": [posts] }` parsing, plus bare arrays and `tweets` / `results` / `items`
- `ok: false`, malformed output, and an empty list as honest outcomes
- Law, Finance, and AI columns, custom columns, focus, run one, run all
- source URLs and original timestamps
- local report history, with new / rising / fading against the previous URLs
- reply inspection and draft text
- the rule that a failed run does not write a fixture success and does not call a paid API

## What was not carried over

- Automatic posting, BlueJay handoff, and the Muse / Connecture send button
- The paid xAI search path and any key file
- A Node UI. The window is Rust and GPUI. `termy-xcli trends-desk` is the non-GPU path.
- The old copy-on-launch script that overwrote `trends-ui.mjs`. There is one copy of the desk, in this repo.

Synthetic rows used by `--fixture` and the unit tests contain the text `SYNTHETIC FIXTURE` and are not written as reports.

A second scan of the new desk sources should show no user-home paths, no live handles, and no credential literals. Loading a key from the environment is not implemented on this path.

The build source for a Windows machine is the GitHub archive of this commit (`https://github.com/jpanasuk-netizen/termy-x/archive/<FULL_SHA>.zip`). That archive is the repository tree at that SHA, including `Cargo.lock`. It does not contain a browser session, cookies, or a local checkout.
