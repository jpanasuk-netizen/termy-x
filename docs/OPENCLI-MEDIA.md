# OpenCLI media attach (free path)

Termy X posts images through Hermes OpenCLI (`x post --media` / Compose Attach). No X API credits.

## Bug

`opencli twitter post --images` fails with:

```text
Page.fileChooserOpened not received within 5s
```

X’s composer `input[type=file][data-testid=fileInput]` is hidden. Extension `set-file-input` clicks it and waits for a native file chooser that never opens. Stock `isRecoverableFileInputError` did not treat that timeout as recoverable, so CDP / DataTransfer fallbacks never ran.

## Fix (local OpenCLI under Hermes)

Patched files:

- `%LOCALAPPDATA%\hermes\node\node_modules\@jackwener\opencli\clis\twitter\utils.js`
- `%LOCALAPPDATA%\hermes\node\node_modules\@jackwener\opencli\clis\twitter\post.js`

1. Treat `fileChooserOpened` / file-chooser / `set-file-input` failures as recoverable.
2. Prefer CDP `DOM.setFileInputFiles` first, then `setFileInput`, then DataTransfer.

Markers stamped by the reapply script:

- `TERMY-X-MEDIA-PATCH:recoverable-filechooser`
- `TERMY-X-MEDIA-PATCH:cdp-first-attach`

## When to re-run

**After any OpenCLI / Hermes upgrade** that reinstalls `@jackwener/opencli` (npm update, Hermes self-update, fresh install).

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\reapply-opencli-media-patch.ps1
```

Or:

```powershell
python scripts\reapply-opencli-media-patch.py
```

Or (Git Bash / just):

```bash
just reapply-opencli-media
```

Script is idempotent: skips work if markers already present, prints `OK` / `FAIL`.

Legacy check-only wrapper: `scripts/patch-opencli-media.ps1` (now calls the reapply script).

## Verified (2026-10-06 CT)

- Live keep post with image: https://x.com/Jasper_Black/status/2107570911679246724 (`has_media: true`)
- Free OpenCLI path only — no X API credits used
