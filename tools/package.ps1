[CmdletBinding()]
param(
    [ValidatePattern('\A[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.+-]+)?\z')]
    [string] $AppVersion = '0.1.0',
    [string] $Iscc,
    [switch] $Offline
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$triple = 'x86_64-pc-windows-msvc'
$dist = Join-Path $root 'dist'

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
    $targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'target' }
    $targetDir = [IO.Path]::GetFullPath($targetDir, $root)
    $argsForCargo = @('--release', '--locked', '-p', 'claude-switch', '--target', $triple, '--target-dir', $targetDir)
    if ($Offline) { $argsForCargo += '--offline' }
    & cargo build @argsForCargo
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)." }

    $builtExe = Join-Path $targetDir "$triple/release/claude-switch.exe"
    $fileVersion = (Get-Item -LiteralPath $builtExe).VersionInfo.FileVersion
    if ($fileVersion -cne $AppVersion) { throw "Expected executable version $AppVersion; got '$fileVersion'." }

    $notices = Join-Path $dist 'THIRD-PARTY-NOTICES'
    & "$PSScriptRoot/write-third-party-notices.ps1" -Out $notices -Offline:$Offline
    $exe = Join-Path $dist 'claude-switch.exe'
    Copy-Item -LiteralPath $builtExe -Destination $exe -Force
    & $Iscc "/DAppVersion=$AppVersion" "/DExe=$exe" "/DNotices=$notices" (Join-Path $root 'installer\claude-switch.iss')
    if ($LASTEXITCODE -ne 0) { throw "ISCC failed ($LASTEXITCODE)." }
}
finally {
    $env:CARGO_BUILD_JOBS = $oldJobs
    Pop-Location
}
