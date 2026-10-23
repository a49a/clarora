param(
    [Parameter(Mandatory = $true)][string]$Installer,
    [Parameter(Mandatory = $true)][string]$Evidence
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:OS -ne 'Windows_NT') { throw 'Installer lifecycle validation requires Windows.' }
$Installer = (Resolve-Path -LiteralPath $Installer).Path
$Evidence = [IO.Path]::GetFullPath($Evidence)
New-Item -ItemType Directory -Force $Evidence | Out-Null
$registered = '{17F3D01D-07B1-45AC-8A65-4304FDCE9158}_is1'
foreach ($hive in @('HKCU:', 'HKLM:')) {
    if (Test-Path -LiteralPath "$hive\Software\Microsoft\Windows\CurrentVersion\Uninstall\$registered") {
        @{ schema_version = 1; status = 'failed'; error = 'Existing Clarora installation; isolated installer check refused.' } |
            ConvertTo-Json | Set-Content -Encoding utf8 "$Evidence/installer-lifecycle.json"
        throw 'Installer lifecycle verification requires an isolated user without an existing Clarora installation.'
    }
}
$installRoot = Join-Path $env:TEMP ("Clarora installer check " + [guid]::NewGuid().ToString('N'))
$dataRoot = Join-Path $env:LOCALAPPDATA 'Clarora-GPUI'
$dataExisted = Test-Path -LiteralPath $dataRoot
New-Item -ItemType Directory -Force $dataRoot | Out-Null
$sentinel = Join-Path $dataRoot ("installer-preservation-" + [guid]::NewGuid().ToString('N') + '.txt')
$token = [guid]::NewGuid().ToString('N')
Set-Content -NoNewline -Encoding ascii -LiteralPath $sentinel -Value $token
$report = @{ schema_version = 1; status = 'failed'; installer = Split-Path -Leaf $Installer; installer_sha256 = (Get-FileHash -LiteralPath $Installer -Algorithm SHA256).Hash.ToLowerInvariant(); install_path = $installRoot; rounds = @() }
try {
    foreach ($round in 1..2) {
        # First install, then an in-place upgrade using the same stable AppId.
        $process = Start-Process -FilePath $Installer -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/DIR=`"$installRoot`"", "/LOG=`"$Evidence/install-$round.log`"") -Wait -PassThru
        if ($process.ExitCode -ne 0) { throw "Installer round $round failed: $($process.ExitCode)" }
        & python "$PSScriptRoot/verify-windows-package.py" $installRoot --installed --native --report "$Evidence/installed-$round.json"
        if ($LASTEXITCODE -ne 0) { throw "Installed package round $round verification failed." }
        $previousPath = $env:PATH
        try {
            $env:PATH = "$env:SystemRoot/System32"
            $runtimeReport = Join-Path $Evidence "app-runtime-$round.json"
            $runtime = Start-Process -FilePath "$installRoot/clarora-gpui.exe" -ArgumentList @('--runtime-check', "`"$runtimeReport`"") -Wait -PassThru
            if ($runtime.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $runtimeReport)) { throw "Installed application runtime round $round failed." }
            if (-not (Get-Content -Raw -LiteralPath $runtimeReport | ConvertFrom-Json).passed) { throw "Installed application runtime report $round did not pass." }
        } finally { $env:PATH = $previousPath }
        if ((Get-Content -Raw -LiteralPath $sentinel) -ne $token) { throw "Application data changed during install round $round." }
        $report.rounds += @{ round = $round; install_exit = $process.ExitCode; runtime_verified = $true; user_data_preserved = $true }
    }
    $uninstaller = Join-Path $installRoot 'unins000.exe'
    if (-not (Test-Path -LiteralPath $uninstaller)) { throw 'Installed uninstaller is missing.' }
    $process = Start-Process -FilePath $uninstaller -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/LOG=`"$Evidence/uninstall.log`"") -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "Uninstaller failed: $($process.ExitCode)" }
    if (Test-Path -LiteralPath "$installRoot/clarora-gpui.exe") { throw 'Uninstall left the application executable behind.' }
    if ((Get-Content -Raw -LiteralPath $sentinel) -ne $token) { throw 'Uninstall removed or changed application data.' }
    $report.uninstall_exit = $process.ExitCode
    $report.user_data_preserved = $true
    $report.status = 'passed'
} catch {
    $report.error = $_.Exception.Message
    throw
} finally {
    if ($report.status -ne 'passed' -and (Test-Path -LiteralPath "$installRoot/unins000.exe")) {
        try {
            $cleanup = Start-Process -FilePath "$installRoot/unins000.exe" -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/LOG=`"$Evidence/failure-cleanup.log`"") -Wait -PassThru
            $report.failure_cleanup_exit = $cleanup.ExitCode
        } catch { $report.failure_cleanup_error = $_.Exception.Message }
    }
    $report | ConvertTo-Json -Depth 8 | Set-Content -Encoding utf8 "$Evidence/installer-lifecycle.json"
    Remove-Item -LiteralPath $sentinel -ErrorAction SilentlyContinue
    if (-not $dataExisted -and -not (Get-ChildItem -LiteralPath $dataRoot -ErrorAction SilentlyContinue)) { Remove-Item -LiteralPath $dataRoot -ErrorAction SilentlyContinue }
}
