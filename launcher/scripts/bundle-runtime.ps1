# Build-time step: snapshots the engine sources the install wizard compiles into src-tauri\resources
# (runtime-src.tar.gz + runtime-src.json; the manifest names the snapshot, shown by the wizard and the About page).
#
#   powershell -ExecutionPolicy Bypass -File scripts\bundle-runtime.ps1 [-Repo <engine repository>] [-Ref <ref>] [-KitTools <dir>]
#
# Runs `git archive` of the engine sources (read only: nothing is checked out or changed there), adds the helper
# scripts of launcher\kit-tools under kit-tools\, and refuses to write the archive if any file fails the "nothing
# derived from the game" scan (examples\bundle_runtime.rs). `npm run tauri build` runs it first (tauri.conf.json
# beforeBuildCommand).
#
# The manifest names the snapshot by the git tree id of the engine sources: the same sources have the same id in
# every repository, whatever its history. The archive is remade only when that id, or a helper script, differs from
# what the manifest records; a fresh clone therefore builds without touching the committed archive.
#
# Where the engine sources are:
# - by default, the engine\ folder of the repository that holds this launcher (ref HEAD);
# - with -Repo or KALAMAR_RUNTIME_REPO: the engine in a repository of its own (ref main unless -Ref says otherwise).
# Without either (no git checkout at all, for example a source archive), the committed archive is kept.
param(
    [string]$Repo = $(if ($env:KALAMAR_RUNTIME_REPO) { $env:KALAMAR_RUNTIME_REPO } else { $env:SPLAT3_RUNTIME_REPO }),
    [string]$Ref = '',
    [string]$KitTools = $(if ($env:KALAMAR_KIT_TOOLS) { $env:KALAMAR_KIT_TOOLS } else { $env:SPLAT3_KIT_TOOLS })
)
$ErrorActionPreference = 'Stop'
$Launcher = Split-Path $PSScriptRoot -Parent
$Parent = Split-Path $Launcher -Parent
$SubDir = ''
if (-not $Repo -and (Test-Path (Join-Path $Parent 'engine\recomp\gen.py')) -and (Test-Path (Join-Path $Parent '.git'))) {
    $Repo = $Parent
    $SubDir = 'engine'
}
if (-not $Ref) { $Ref = if ($SubDir) { 'HEAD' } else { 'main' } }
if (-not $KitTools) {
    $KitTools = Join-Path $Launcher 'kit-tools'
}
if (-not $Repo -or -not (Test-Path (Join-Path $Repo '.git'))) {
    Write-Host "bundle-runtime: no engine repository found (name one with -Repo or KALAMAR_RUNTIME_REPO); keeping the committed src-tauri\resources\runtime-src.*"
    exit 0
}
$commit = (& git -C $Repo rev-parse --verify "$Ref^{commit}").Trim()
if ($LASTEXITCODE -ne 0 -or -not $commit) { throw "bundle-runtime: cannot resolve $Ref in $Repo" }
# The snapshot's identifier: the tree of the engine sources (of the engine\ folder in a combined repository).
$treeSpec = if ($SubDir) { "${commit}:$SubDir" } else { "$commit^{tree}" }
$tree = (& git -C $Repo rev-parse --verify $treeSpec).Trim()
if ($LASTEXITCODE -ne 0 -or -not $tree) { throw "bundle-runtime: cannot resolve the engine tree $treeSpec in $Repo" }
$manifest = Join-Path $Launcher 'src-tauri\resources\runtime-src.json'
if (Test-Path $manifest) {
    $man = Get-Content $manifest -Raw | ConvertFrom-Json
    # The helper scripts are part of the archive too: a change there remakes it for the same engine sources.
    $kitSame = $true
    foreach ($entry in $man.kit_tools) {
        $rel = $entry[0] -replace '^kit-tools/', ''
        $file = Join-Path $KitTools $rel
        if (-not (Test-Path $file) -or (Get-FileHash $file -Algorithm SHA256).Hash.ToLower() -ne $entry[1]) { $kitSame = $false }
    }
    if ($man.commit -eq $tree -and $kitSame) {
        Write-Host "bundle-runtime: already at engine snapshot $tree"
        exit 0
    }
}
Push-Location (Join-Path $Launcher 'src-tauri')
try {
    $extra = @()
    if ($SubDir) { $extra = @('--subdir', $SubDir) }
    & cargo run --quiet --example bundle_runtime -- --repo $Repo --commit $commit --kit-tools $KitTools @extra
    if ($LASTEXITCODE -ne 0) { throw "bundle-runtime: refused or failed (exit $LASTEXITCODE)" }
} finally { Pop-Location }
