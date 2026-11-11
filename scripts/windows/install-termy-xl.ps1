# Install the standalone Termy XL window beside an existing Termy install.
# Termy XL is trends_desk.exe. It does not replace termy.exe, termy-7154b8a5.exe,
# or any shortcut that launches those programs. Do not run x.exe.
# Settings, history, and drafts go to %APPDATA%\Termy XL unless TERMY_XL_DATA_DIR is set.
param(
    [string]$SourceDir = "",
    [string]$InstallDir = ""
)

$ErrorActionPreference = "Stop"

$visibleName = "Termy XL"
$binaryName = "trends_desk.exe"
$cliName = "termy-xcli.exe"

if (-not $InstallDir) {
    $InstallDir = Join-Path $env:LOCALAPPDATA $visibleName
}

function Normalize([string]$Path) {
    return [System.IO.Path]::GetFullPath($Path).TrimEnd('\')
}

$installFull = Normalize $InstallDir
$blocked = @(
    (Join-Path $env:LOCALAPPDATA "Termy"),
    (Join-Path ${env:ProgramFiles} "Termy")
)
if (${env:ProgramFiles(x86)}) {
    $blocked += (Join-Path ${env:ProgramFiles(x86)} "Termy")
}
foreach ($dir in $blocked) {
    if ((Test-Path $dir) -and ((Normalize $dir) -eq $installFull)) {
        throw "Refusing to install Termy XL into the Termy directory: $dir"
    }
}

if (-not $SourceDir) {
    $repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
    $candidates = @(
        (Join-Path $repoRoot "target\release"),
        (Join-Path $repoRoot "target\x86_64-pc-windows-msvc\release")
    )
    foreach ($candidate in $candidates) {
        if (Test-Path (Join-Path $candidate $binaryName)) {
            $SourceDir = $candidate
            break
        }
    }
}
if (-not $SourceDir) {
    throw "Pass -SourceDir pointing at the folder that contains $binaryName"
}

$sourceExe = Join-Path $SourceDir $binaryName
if (-not (Test-Path $sourceExe)) {
    throw "Missing $sourceExe. Build it with: cargo build --release -p termy --bin trends_desk"
}
if ([System.IO.Path]::GetFileName($sourceExe) -eq "x.exe") {
    throw "Refusing to install a file named x.exe"
}

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
Copy-Item -Force $sourceExe (Join-Path $InstallDir $binaryName)
$sourceCli = Join-Path $SourceDir $cliName
if (Test-Path $sourceCli) {
    Copy-Item -Force $sourceCli (Join-Path $InstallDir $cliName)
}

$target = Join-Path $InstallDir $binaryName
$shell = New-Object -ComObject WScript.Shell
$shortcutPaths = @(
    (Join-Path ([Environment]::GetFolderPath("Desktop")) "$visibleName.lnk"),
    (Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\$visibleName.lnk")
)
foreach ($shortcutPath in $shortcutPaths) {
    $leaf = [System.IO.Path]::GetFileName($shortcutPath)
    if ($leaf -eq "Termy.lnk" -or $leaf -eq "Termy X.lnk") {
        throw "Refusing to rewrite $shortcutPath"
    }
    $link = $shell.CreateShortcut($shortcutPath)
    $link.TargetPath = $target
    $link.WorkingDirectory = $InstallDir
    $link.Description = $visibleName
    $link.WindowStyle = 1
    $link.Save()
    Write-Output "shortcut $shortcutPath -> $target"
}

$desktop = [Environment]::GetFolderPath("Desktop")
Get-ChildItem -Path $desktop -Filter *.lnk -ErrorAction SilentlyContinue | ForEach-Object {
    $existing = $shell.CreateShortcut($_.FullName)
    if ($existing.TargetPath -like "*termy-7154b8a5.exe*") {
        Write-Output "left untouched: $($_.FullName)"
    }
}

Write-Output "installed $target"
Write-Output "Termy shortcuts and the Termy install folder were not changed."
Write-Output "Termy XL data is %APPDATA%\Termy XL (override with TERMY_XL_DATA_DIR). It is not %APPDATA%\termy."
