<#
.SYNOPSIS
    Builds CAMCraft with cargo.
.PARAMETER DebugBuild
    Build a debug binary instead of release.
.PARAMETER Test
    Run the unit tests before building.
.PARAMETER Clean
    Remove build artifacts before building.
#>
[CmdletBinding()]
param(
    [switch]$DebugBuild,
    [switch]$Test,
    [switch]$Clean
)

$ErrorActionPreference = 'Stop'

$project = Split-Path -Parent $PSScriptRoot
$manifest = Join-Path $project 'Cargo.toml'

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw 'cargo not found. Install the Rust toolchain from https://rustup.rs'
}
if (-not (Test-Path $manifest)) {
    throw "Cargo.toml not found at $manifest"
}

function Invoke-Cargo {
    & cargo @args
    if ($LASTEXITCODE -ne 0) { throw "cargo $($args -join ' ') failed with exit code $LASTEXITCODE" }
}

if ($Clean) { Invoke-Cargo clean --manifest-path $manifest }
if ($Test)  { Invoke-Cargo test --manifest-path $manifest }

$profileDir = 'debug'
$buildArgs = @('build', '--manifest-path', $manifest)
if (-not $DebugBuild) {
    $buildArgs += '--release'
    $profileDir = 'release'
}
Invoke-Cargo @buildArgs

$exe = Join-Path $project "target\$profileDir\camcraft.exe"
Write-Host "Built: $exe"
