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

## Quote image (OpenCLI 2.0.2)

A quote image does not use `twitter post --images` and does not use the local Hermes `post.js` patch. After the native quote card is proven, Termy calls the documented command once:

```text
opencli browser upload --testid fileInput <file> --tab <returned-page-id> --session termyx-quote --window foreground
```

The envelope must say `uploaded: true` and name the file. Termy then reads the composer preview. Avatar and quoted-text emoji do not count. A bridge refusal stops. There is no second upload and Post is not clicked. The refusal keeps the CLI's own code and message (`not_file_input`, `Page.fileChooserOpened`, or `-32000` / `Not allowed`). A missing card, a card that changes after the upload, or a missing preview also leaves Post unclicked.

OpenCLI 2.0.2 checks that the resolved target is `input[type=file]` before it asks the extension to attach. The extension then intercepts a file chooser and clicks that input. Chrome's `-32000 Not allowed` remains the refusal for `DOM.setFileInputFiles` under the extension debugger when that chooser path is not accepted. Neither case is retried from Termy.

This command does not post:

```text
termy-xcli write quote --to 2118971448614334959 --inspect --media marked-quote-probe.png
```

Use one allowlisted jpg, png, gif, or webp that a worker generated for the probe. Do not pass `--confirm yes`.
