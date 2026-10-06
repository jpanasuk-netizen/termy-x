# Termy X media attach (OpenCLI free path)

## Problem
opencli twitter post --images failed with Page.fileChooserOpened not received within 5s.
set-file-input clicks the hidden X file input; chooser never opens. DataTransfer fallback
never ran because isRecoverableFileInputError ignored that error.

## Fix (local OpenCLI 1.8.8 under Hermes)
- %LOCALAPPDATA%\hermes\node\node_modules\@jackwener\opencli\clis\twitter\utils.js
- %LOCALAPPDATA%\hermes\node\node_modules\@jackwener\opencli\clis\twitter\post.js

1. Treat fileChooserOpened / file-chooser timeouts as recoverable.
2. Attach via CDP DOM.setFileInputFiles first, then setFileInput, then DataTransfer.

Re-check after opencli upgrades: powershell -File scripts/patch-opencli-media.ps1

## Verified 2026-10-06 CT
- Live keep post with image: https://x.com/Jasper_Black/status/2107570911679246724 (has_media true)
- Earlier smoke 2107570178150019217 deleted after verify
- Live text-only 2107566595547787472 left untouched
- x post --dry-run --media plans OpenCLI --images; Compose uses same OpenCliPublisher
- No X API credits used