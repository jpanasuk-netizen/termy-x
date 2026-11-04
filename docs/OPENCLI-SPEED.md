# OpenCLI write-speed patch (local Hermes install)

Removes fixed sleeps in OpenCLI's twitter write commands that run
*after* the write is already verified (plus one pre-click pause after
composer-text verification):

- `like.js`, `unlike.js`, `retweet.js`, `unretweet.js`: post-success `page.wait(2)` (-2s per engage)
- `post.js`: pre-submit `page.wait(1)` after verified text insert (-1s per post/reply-via-post)

No selector, poll, confirmation, or timeout logic changes; success
still requires the same observed UI state / toast as upstream.

Re-apply after any OpenCLI reinstall/upgrade (from the repo root, on
the PC):

```
python scripts\reapply-opencli-speed-patch.py
```

Idempotent; first run writes `<file>.bak-termy-speed` backups.
Verified 2026-10-19 on OpenCLI 1.8.8: whitespace-post probe (reaches
the submit stage but cannot post) 10.4s before -> 5.7/7.3/8.9s after;
the guaranteed component is the removed sleeps (1s/2s), the rest is
page-load variance.
