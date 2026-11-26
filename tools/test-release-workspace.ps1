# Test with disposable profile paths; leave Rust's toolchain and Cargo cache in place.
[CmdletBinding()]
param([switch] $Offline)

$ErrorActionPreference = 'Stop'
$profileRoot = Join-Path ([IO.Path]::GetTempPath()) ('cs-release-tests-' + [guid]::NewGuid())
$paths = @{
    USERPROFILE = 'home'; HOME = 'home'; APPDATA = 'appdata'; LOCALAPPDATA = 'localappdata'
    TEMP = 'temp'; TMP = 'temp'; CLAUDE_CONFIG_DIR = 'claude-config'
}
$saved = @{}
$fixed = @{ CARGO_BUILD_JOBS = '2' }
if (-not $env:CARGO_HOME) { $fixed.CARGO_HOME = Join-Path $env:USERPROFILE '.cargo' }
if (-not $env:RUSTUP_HOME) { $fixed.RUSTUP_HOME = Join-Path $env:USERPROFILE '.rustup' }
foreach ($key in $paths.Keys) {
    $fixed[$key] = [IO.Directory]::CreateDirectory((Join-Path $profileRoot $paths[$key])).FullName
}

Push-Location -LiteralPath (Split-Path -Parent $PSScriptRoot)
try {
    foreach ($key in $fixed.Keys) {
        $saved[$key] = [Environment]::GetEnvironmentVariable($key)
        [Environment]::SetEnvironmentVariable($key, $fixed[$key])
    }
    $flags = @('--workspace', '--locked')
    if ($Offline) { $flags += '--offline' }
    & cargo test @flags
    if ($LASTEXITCODE -ne 0) { throw "cargo test failed ($LASTEXITCODE)." }
} finally {
    foreach ($key in $saved.Keys) { [Environment]::SetEnvironmentVariable($key, $saved[$key]) }
    Pop-Location
}
