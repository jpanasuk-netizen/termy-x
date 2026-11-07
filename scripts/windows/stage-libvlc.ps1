<#
.SYNOPSIS
Stage the official VLC 3.0.24 win64 runtime into <termy.exe dir>\vlc.

.DESCRIPTION
Requires Python 3. Does not modify the source tree and does not install VLC.
Pass either -Zip (hash-checked) or -SourceDir (version string only).

.PARAMETER Zip
Path to vlc-3.0.24-win64.zip from download.videolan.org.

.PARAMETER SourceDir
Extracted official tree, or C:\Program Files\VideoLAN\VLC when that install is 3.0.24.
Files are copied. The install is not changed.

.PARAMETER Dest
Destination folder, normally the vlc directory beside termy.exe.
#>
param(
    [string]$Zip,
    [string]$SourceDir,
    [Parameter(Mandatory = $true)]
    [string]$Dest
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ((-not $Zip -and -not $SourceDir) -or ($Zip -and $SourceDir)) {
    throw "Pass exactly one of -Zip or -SourceDir."
}

$python = $null
foreach ($name in @("python", "python3", "py")) {
    $command = Get-Command $name -ErrorAction SilentlyContinue
    if ($command) {
        $python = $command.Source
        break
    }
}
if (-not $python) {
    throw "Python 3 is required to stage libVLC. Install it, or run scripts/windows/stage-libvlc.py directly."
}

$script = Join-Path $PSScriptRoot "stage-libvlc.py"
$arguments = @($script, "--dest", $Dest)
if ($Zip) {
    $arguments += @("--zip", $Zip)
} else {
    $arguments += @("--source-dir", $SourceDir)
}
& $python @arguments
if ($LASTEXITCODE -ne 0) {
    throw "stage-libvlc.py failed with exit code $LASTEXITCODE"
}
