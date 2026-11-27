# Checks that Chat still reads the screens of the Claude Code installed on this machine: records
# them from the real CLI with one of the signed-in accounts and runs Chat's readers over the
# recordings. A run is a few Haiku requests. See tools/screen-recorder/README.md, "After a Claude
# Code update".
#
#   pwsh tools\cli-compat.ps1 -ConfigDir $env:USERPROFILE\.claude-accounts\<name>\cli
#
# This only builds the recorder and starts `recorder compat`, which shows the run on a screen of
# its own (keys: Up/Down, Enter, r, q), removes its scratch folder and the scratch sessions it left
# in the account's folder, and prints where its report is.
[CmdletBinding()]
param(
    # The account to record with: its CLAUDE_CONFIG_DIR (%USERPROFILE%\.claude or
    # %USERPROFILE%\.claude-accounts\<name>\cli). Without it: the signed-in account whose
    # `projects` folder was written least recently.
    [string] $ConfigDir,
    [string] $Model = 'haiku',
    # Only these scripts (names from tools\screen-recorder\scripts, without .txt).
    [string[]] $Only,
    # Where the recordings and the report go; by default a new folder under
    # %LOCALAPPDATA%\claude-switch-cli-compat\runs (the newest five are kept).
    [string] $Out,
    # Do nothing when this Claude Code version was already checked in full.
    [switch] $OnlyIfNewVersion,
    # Plain lines instead of the screen.
    [switch] $Plain
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$crate = Join-Path $repo 'tools\screen-recorder'

& cargo build --locked --manifest-path (Join-Path $crate 'Cargo.toml')
if ($LASTEXITCODE -ne 0) { throw "The recorder did not build ($LASTEXITCODE)." }
$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $crate 'target' }
$recorder = Join-Path $target 'debug\recorder.exe'
if (-not (Test-Path -LiteralPath $recorder -PathType Leaf)) { throw "No recorder at $recorder." }

$arguments = @('compat', '--repo', $repo, '--model', $Model)
if ($ConfigDir) { $arguments += @('--config-dir', (Resolve-Path -LiteralPath $ConfigDir).Path) }
if ($Only) { $arguments += @('--only', ($Only -join ',')) }
if ($Out) { $arguments += @('--out', $Out) }
if ($OnlyIfNewVersion) { $arguments += '--only-if-new-version' }
if ($Plain) { $arguments += '--plain' }

# 0: every state reads as expected; 1: some do not; 2: the check could not run.
& $recorder @arguments
exit $LASTEXITCODE
