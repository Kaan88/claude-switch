[CmdletBinding()]
param(
    [ValidatePattern('\A[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.+-]+)?\z')]
    [string] $AppVersion = '0.1.0',
    [string] $Iscc
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$target = Join-Path $root 'target'

if (-not $Iscc) {
    $candidates = @(
        foreach ($base in ${env:ProgramFiles(x86)}, $env:ProgramFiles, "$env:LOCALAPPDATA\Programs") {
            if ($base) { Join-Path $base 'Inno Setup 7\ISCC.exe' }
        }
    )
    $Iscc = $candidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
    if (-not $Iscc) {
        $Iscc = (Get-Command ISCC.exe -CommandType Application -ErrorAction SilentlyContinue |
            Select-Object -First 1).Source
    }
    if (-not $Iscc) {
        throw 'ISCC.exe not found. Install Inno Setup 7 or pass -Iscc with the full path.'
    }
}
$Iscc = (Resolve-Path -LiteralPath $Iscc).Path

$oldJobs = $env:CARGO_BUILD_JOBS
Push-Location -LiteralPath $root
try {
    $env:CARGO_BUILD_JOBS = '2'
    & cargo build --release --locked -p claude-switch --target-dir $target
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)." }

    $exe = Join-Path $target 'release\claude-switch.exe'
    if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw "Missing release executable: $exe" }

    & $Iscc "/DAppVersion=$AppVersion" "/DExe=$exe" (Join-Path $root 'installer\claude-switch.iss')
    if ($LASTEXITCODE -ne 0) { throw "ISCC failed ($LASTEXITCODE)." }
}
finally {
    $env:CARGO_BUILD_JOBS = $oldJobs
    Pop-Location
}
