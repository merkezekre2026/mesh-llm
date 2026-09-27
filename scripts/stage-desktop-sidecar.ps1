# Windows counterpart of stage-desktop-sidecar.sh: stage a built mesh-llm host
# and its native runtime(s) as the Tauri sidecar and resources of the desktop
# app (desktop/src-tauri/binaries/). The host is copied unchanged.
param(
    [Parameter(Mandatory = $true)][string]$Binary,
    [string]$Runtimes = "",
    [string]$Target = ""
)

$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path -Parent $PSScriptRoot
$StageDir = Join-Path $RepoRoot "desktop/src-tauri/binaries"

if (-not (Test-Path -LiteralPath $Binary -PathType Leaf)) {
    throw "mesh-llm binary not found: $Binary (build it with 'just build' or 'just release-build')"
}
if ($Target -eq "") {
    $hostLine = rustc -vV | Where-Object { $_ -like "host: *" }
    $Target = $hostLine.Substring(6).Trim()
}

New-Item -ItemType Directory -Force -Path $StageDir | Out-Null
$sidecar = Join-Path $StageDir "mesh-llm-sidecar-$Target.exe"
Copy-Item -LiteralPath $Binary -Destination $sidecar -Force
Write-Output "staged sidecar: $sidecar"

$runtimeStage = Join-Path $StageDir "native-runtimes"
if (Test-Path -LiteralPath $runtimeStage) {
    Remove-Item -LiteralPath $runtimeStage -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $runtimeStage | Out-Null
# Tauri requires the resource directory to exist even when it is empty.
New-Item -ItemType File -Force -Path (Join-Path $runtimeStage ".keep") | Out-Null

if ($Runtimes -eq "") {
    Write-Output "no -Runtimes given: the app will use mesh-llm's own runtime discovery"
    exit 0
}
if (-not (Test-Path -LiteralPath $Runtimes -PathType Container)) {
    throw "native runtime directory not found: $Runtimes"
}

$dirs = Get-ChildItem -LiteralPath $Runtimes -Directory
if ($dirs.Count -eq 0) {
    throw "no runtime directories found in $Runtimes"
}
foreach ($dir in $dirs) {
    Copy-Item -LiteralPath $dir.FullName -Destination $runtimeStage -Recurse -Force
    Write-Output "staged runtime: $($dir.Name)"
}
