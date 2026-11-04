# Compatibility wrapper — calls the durable reapply script.
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
& (Join-Path $here "reapply-opencli-media-patch.ps1") @args
exit $LASTEXITCODE
