# Run only on the disposable Windows release runner. Never uninstall an existing copy.
[CmdletBinding()]
param([Parameter(Mandatory)][string] $AppVersion)

$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or -not $env:RUNNER_TEMP) {
    throw 'Run only on a GitHub-hosted release runner.'
}
$uninstallKey = '{4F2C369B-84AF-4A46-AE8B-D22103025F78}_is1'
foreach ($hive in 'HKCU:', 'HKLM:') {
    foreach ($base in 'Software', 'Software\WOW6432Node') {
        if (Test-Path "$hive\$base\Microsoft\Windows\CurrentVersion\Uninstall\$uninstallKey") {
            throw 'An existing Claude Switch install was found; refusing to run the smoke test.'
        }
    }
}

$dist = Join-Path (Split-Path -Parent $PSScriptRoot) 'dist'
$setup = Join-Path $dist 'claude-switch-setup.exe'
$hub = Get-Content -LiteralPath (Join-Path $dist 'hub.txt')
$setupEntry = @($hub -cmatch '^windows-x86_64\.setup:')
$hashEntry = @($hub -cmatch '^windows-x86_64\.setup\.sha256:')
$setupHash = (Get-FileHash -LiteralPath $setup -Algorithm SHA256).Hash.ToLowerInvariant()
if ($setupEntry.Count -ne 1 -or $setupEntry[0] -cne 'windows-x86_64.setup: claude-switch-setup.exe' -or
    $hashEntry.Count -ne 1 -or $hashEntry[0] -cne "windows-x86_64.setup.sha256: $setupHash") {
    throw 'hub.txt does not identify the exact staged installer and its SHA-256.'
}
Write-Host "Hub setup SHA-256 matches: $setupHash"

$scratch = Join-Path $env:RUNNER_TEMP ('cs-setup-smoke-' + [guid]::NewGuid())
[void][IO.Directory]::CreateDirectory($scratch)
$installDir = Join-Path $scratch 'Claude Switch'
$silent = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART')
$installArgs = $silent + @('/SP-', "/DIR=`"$installDir`"")
$installed = Start-Process -FilePath $setup -ArgumentList $installArgs -Wait -PassThru
if ($installed.ExitCode -ne 0) { throw "Silent setup failed ($($installed.ExitCode))." }
try {
    $exe = Join-Path $installDir 'claude-switch.exe'
    $version = (Get-Item -LiteralPath $exe).VersionInfo.FileVersion
    if ($version -cne $AppVersion) { throw "Installed version '$version' does not match the tag '$AppVersion'." }
    foreach ($file in 'claude-switch.exe', 'THIRD-PARTY-NOTICES') {
        $actual = (Get-FileHash -LiteralPath (Join-Path $installDir $file)).Hash
        $expected = (Get-FileHash -LiteralPath (Join-Path $dist $file)).Hash
        if ($actual -ne $expected) { throw "Installed $file differs from the release asset." }
    }
    Write-Host "Installed version $version; executable and notices match the staged assets."

    # There is no --version flag. With --demo, the headless diagnostic explicitly refuses
    # before reading any config or credentials or making a network request.
    $stdout = Join-Path $scratch 'probe.stdout'
    $stderr = Join-Path $scratch 'probe.stderr'
    $profile = [IO.Directory]::CreateDirectory((Join-Path $scratch 'probe-profile')).FullName
    $probeEnv = @{ USERPROFILE = $profile; HOME = $profile; APPDATA = $profile; LOCALAPPDATA = $profile }
    $probe = Start-Process -FilePath $exe -ArgumentList '--demo', '--usage-shape' `
        -Environment $probeEnv -RedirectStandardOutput $stdout -RedirectStandardError $stderr -Wait -PassThru
    $reply = [string](Get-Content -LiteralPath $stderr -Raw)
    if ($probe.ExitCode -ne 1 -or $reply.Trim() -cne 'claude-switch: unavailable in demo mode') {
        throw "Installed executable did not answer the headless demo probe (exit $($probe.ExitCode))."
    }
    Write-Host 'Installed executable refused the usage diagnostic in demo mode, as expected (exit 1).'
} finally {
    $uninstall = Join-Path $installDir 'unins000.exe'
    $removed = Start-Process -FilePath $uninstall -ArgumentList $silent -Wait -PassThru
    if ($removed.ExitCode -ne 0) { throw "Silent uninstall failed ($($removed.ExitCode))." }
    if (Test-Path -LiteralPath $installDir) { throw "Uninstall left the program folder: $installDir" }
    Write-Host 'Silent uninstall removed the program folder.'
}
