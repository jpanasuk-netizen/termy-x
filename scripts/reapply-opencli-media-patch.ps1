# Re-apply OpenCLI twitter media-attach patch (Hermes local install).
# Thin Windows wrapper around the Python reapply script (idempotent).
#
# Usage:
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\reapply-opencli-media-patch.ps1
#   powershell -File scripts\reapply-opencli-media-patch.ps1 -OpenCliRoot "D:\path\to\@jackwener\opencli"
#
# Markers (idempotent):
#   TERMY-X-MEDIA-PATCH:recoverable-filechooser  (utils.js)
#   TERMY-X-MEDIA-PATCH:cdp-first-attach         (post.js)

[CmdletBinding()]
param(
    [string]$OpenCliRoot = ""
)

$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$pyScript = Join-Path $here "reapply-opencli-media-patch.py"

if (-not (Test-Path $pyScript)) {
    Write-Host "FAIL: missing $pyScript" -ForegroundColor Red
    exit 1
}

$python = $null
foreach ($cand in @("python", "python3", "py")) {
    $cmd = Get-Command $cand -ErrorAction SilentlyContinue
    if ($cmd) { $python = $cmd.Source; break }
}
if (-not $python) {
    # Hermes node often ships nearby; last resort: Windows py launcher
    Write-Host "FAIL: python not found on PATH (need python to reapply patch)" -ForegroundColor Red
    exit 1
}

$argList = @($pyScript)
if ($OpenCliRoot) {
    $argList += @("--opencli-root", $OpenCliRoot)
}

Write-Host "INFO: using $python"
& $python @argList
exit $LASTEXITCODE
