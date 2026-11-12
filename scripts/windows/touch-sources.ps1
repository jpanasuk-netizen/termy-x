# After unzipping a GitHub archive, file timestamps can be older than
# build artifacts already in the tree. Touch every source file so cargo
# rebuilds them. Safe to run more than once.
param(
    [string]$Root = ""
)

$ErrorActionPreference = "Stop"
if (-not $Root) {
    $Root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
}
Get-ChildItem -Path $Root -Recurse -File -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -notmatch '\\target\\|\\\.git\\' } |
    ForEach-Object { $_.LastWriteTime = Get-Date }
Write-Output "touched source timestamps under $Root"
