<#
.SYNOPSIS
    Builds CAMCraft with cargo.
.PARAMETER DebugBuild
    Build a debug binary instead of release.
.PARAMETER Test
    Run the unit tests before building.
.PARAMETER Installer
    Build a release MSI installer. WiX 3.14 binaries are downloaded to tools\wix3 on first run.
    Output: dist\CAMCraft-<version>-x64.msi
.PARAMETER Clean
    Remove build artifacts before building.
#>
[CmdletBinding()]
param(
    [switch]$DebugBuild,
    [switch]$Test,
    [switch]$Clean,
    [switch]$Installer
)

$ErrorActionPreference = 'Stop'

$project = Split-Path -Parent $PSScriptRoot
$manifest = Join-Path $project 'Cargo.toml'

# Admin or fresh shells may not have the rustup folder on PATH.
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if ((Test-Path $cargoBin) -and ($env:PATH -notlike "*$cargoBin*")) { $env:PATH = "$cargoBin;$env:PATH" }

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

if ($Installer) {
    if ($DebugBuild) { throw '-Installer builds release only, do not combine with -DebugBuild' }
    $version = (Select-String -Path $manifest -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches.Groups[1].Value

    $wix = Join-Path $project 'tools\wix3'
    if (-not (Test-Path "$wix\candle.exe")) {
        Write-Host 'Downloading WiX 3.14 binaries ...'
        New-Item -ItemType Directory -Force $wix | Out-Null
        $zip = Join-Path $wix 'wix314-binaries.zip'
        Invoke-WebRequest 'https://github.com/wixtoolset/wix3/releases/download/wix3141rtm/wix314-binaries.zip' -OutFile $zip
        Expand-Archive $zip -DestinationPath $wix -Force
    }

    $dist = Join-Path $project 'dist'
    New-Item -ItemType Directory -Force $dist | Out-Null
    $obj = Join-Path $dist 'main.wixobj'
    $msi = Join-Path $dist "CAMCraft-$version-x64.msi"
    & "$wix\candle.exe" -nologo -arch x64 "-dVersion=$version" "-dExePath=$exe" `
        "-dIconPath=$project\assets\icons\camcraft.ico" -out $obj (Join-Path $project 'wix\main.wxs')
    if ($LASTEXITCODE) { throw 'candle failed' }
    & "$wix\light.exe" -nologo -ext WixUIExtension -sice:ICE38 -sice:ICE43 -sice:ICE57 -sice:ICE64 -o $msi $obj
    if ($LASTEXITCODE) { throw 'light failed' }
    Write-Host "Built: $msi"
}
