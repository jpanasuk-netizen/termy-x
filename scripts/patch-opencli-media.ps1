# Re-apply OpenCLI twitter media attach fix (fileChooserOpened 5s).
# Patches local Hermes OpenCLI so x post --media / Compose Attach work free.
$ErrorActionPreference = "Stop"
$ocRoot = Join-Path $env:LOCALAPPDATA "hermes\node\node_modules\@jackwener\opencli"
$utils = Join-Path $ocRoot "clis\twitter\utils.js"
$post = Join-Path $ocRoot "clis\twitter\post.js"
if (-not (Test-Path $utils) -or -not (Test-Path $post)) { throw "OpenCLI twitter adapters missing under $ocRoot" }
Copy-Item $utils "$utils.bak-chooser" -Force
Copy-Item $post "$post.bak-chooser" -Force
$u = Get-Content $utils -Raw
if ($u -notmatch "filechooseropened") {
  Write-Host "utils.js needs manual re-patch for isRecoverableFileInputError (fileChooserOpened)"
} else { Write-Host "utils.js OK (fileChooser recovery present)" }
$p = Get-Content $post -Raw
if ($p -notmatch "attachImagesViaCdp") {
  Write-Host "post.js needs CDP-first attachImages helpers re-applied"
} else { Write-Host "post.js OK (CDP-first attach present)" }