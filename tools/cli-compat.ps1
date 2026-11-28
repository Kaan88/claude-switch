# Checks that Chat still reads the screens of the Claude Code installed on this machine: records
# them from the real CLI with one of the signed-in accounts and runs Chat's readers over the
# recordings. A run is a few Haiku requests. See tools/screen-recorder/README.md, "After a Claude
# Code update".
#
#   pwsh tools\cli-compat.ps1 -ConfigDir $env:USERPROFILE\.claude-accounts\<name>\cli
#
# First it checks, without an account, that the installed Claude Code still has every /config key
# and label Settings > Claude Code > Advanced writes (-KeysOnly: only that). Then it builds the
# recorder and starts `recorder compat`, which shows the run on a screen of
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
    [switch] $Plain,
    # Only the check of /config's keys and labels (no account, no recording).
    [switch] $KeysOnly
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$crate = Join-Path $repo 'tools\screen-recorder'

# Settings > Claude Code > Advanced writes the /config entries of crates\core\src\cc_config.rs
# into Claude Code's own files. A Claude Code update that renames one of their keys or labels
# would go unnoticed (the page would write a key nothing reads), so every key and label must
# still be in the installed Claude Code's code. Read only; needs no account. Returns the missing.
function Test-ConfigKeys {
    $table = Get-Content -Raw -LiteralPath (Join-Path $repo 'crates\core\src\cc_config.rs')
    $body = $table.Substring($table.IndexOf('pub const ENTRIES'))
    $patterns = @{}
    foreach ($m in [regex]::Matches($body, '(?:switch|entry)\(\s*"([^"]+)",\s*"([^"]+)"')) {
        # A nested key (worktree.baseRef) is checked by its last part.
        $patterns["key $($m.Groups[1].Value)"] = $m.Groups[1].Value.Split('.')[-1] + ':'
        $patterns["label $($m.Groups[2].Value)"] = '"' + $m.Groups[2].Value + '"'
    }
    foreach ($m in [regex]::Matches($body, 'also_remove: &\["([^"]+)"\]')) {
        $patterns["key $($m.Groups[1].Value)"] = $m.Groups[1].Value + ':'
    }
    if ($patterns.Count -lt 40) { throw "Read only $($patterns.Count) keys and labels from cc_config.rs." }
    $claude = Get-Command claude -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $claude -or (Get-Item -LiteralPath $claude.Source).Length -lt 1MB) {
        Write-Warning 'The /config keys were not checked: no native claude.exe on PATH.'
        return @()
    }
    $version = (& $claude.Source --version) -join ' '
    $missing = [System.Collections.Generic.HashSet[string]]::new([string[]]@($patterns.Keys))
    # Chunks with an overlap, as Latin-1 so that every byte is one character.
    $latin1 = [System.Text.Encoding]::Latin1
    $size = 64MB
    $overlap = 1024
    $buffer = [byte[]]::new($size + $overlap)
    $carry = 0
    $stream = [System.IO.File]::OpenRead($claude.Source)
    try {
        while ($missing.Count -gt 0 -and ($read = $stream.Read($buffer, $carry, $size)) -gt 0) {
            $total = $carry + $read
            $text = $latin1.GetString($buffer, 0, $total)
            foreach ($name in @($missing)) {
                if ($text.Contains($patterns[$name], [StringComparison]::Ordinal)) { [void]$missing.Remove($name) }
            }
            $carry = [Math]::Min($overlap, $total)
            [Array]::Copy($buffer, $total - $carry, $buffer, 0, $carry)
        }
    } finally {
        $stream.Dispose()
    }
    if ($missing.Count -eq 0) {
        Write-Host "ok        /config keys and labels: all $($patterns.Count) are in Claude Code $version"
    } else {
        foreach ($name in $missing) { Write-Host "MISSING   /config $name (Claude Code $version)" }
        Write-Host 'See docs/DESIGN.md, "Claude Code''s /config": the entry was renamed or removed.'
    }
    return @($missing)
}

$missingKeys = @(Test-ConfigKeys)
if ($KeysOnly) { exit ([int]($missingKeys.Count -gt 0)) }

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

# 0: every state reads as expected; 1: some do not; 2: the check could not run. A missing
# /config key makes a run that read every state 1.
& $recorder @arguments
$code = $LASTEXITCODE
if ($code -eq 0 -and $missingKeys.Count -gt 0) { $code = 1 }
exit $code
