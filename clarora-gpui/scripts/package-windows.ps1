param(
    [Parameter(Mandatory = $true)][string]$RuntimeDir,
    [string]$Output = "$PSScriptRoot/../../../artifacts/clarora/gpui/windows",
    [string]$Executable = "$PSScriptRoot/../target/release/clarora-gpui.exe",
    [switch]$Installer,
    [string]$InnoSetupCompiler,
    [string]$CertificateThumbprint,
    [string]$TimestampUrl = 'http://timestamp.digicert.com'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
try {
if ($env:OS -ne 'Windows_NT') { throw 'Run this packaging entry point on Windows.' }
$RuntimeDir = (Resolve-Path -LiteralPath $RuntimeDir).Path
$Executable = (Resolve-Path -LiteralPath $Executable).Path
$Output = [IO.Path]::GetFullPath($Output)
if ($Output.StartsWith($RuntimeDir + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) -or $Output -eq $RuntimeDir) {
    throw 'Output must be separate from RuntimeDir.'
}
New-Item -ItemType Directory -Force $Output | Out-Null
$verifier = Join-Path $PSScriptRoot 'verify-windows-package.py'
& python $verifier $RuntimeDir --runtime --report "$Output/runtime-verification.json" --native
if ($LASTEXITCODE -ne 0) { throw 'Complete Windows runtime verification failed.' }
$version = (Select-String -Path "$PSScriptRoot/../Cargo.toml" -Pattern '^version = "([^"]+)"').Matches[0].Groups[1].Value
$stage = Join-Path $Output "Clarora-$version-windows-x64"
if (Test-Path -LiteralPath $stage) {
    if (-not (Test-Path -LiteralPath "$stage/package-manifest.json") -and -not (Test-Path -LiteralPath "$stage/.clarora-package-stage")) { throw 'Refusing to replace a directory without a Clarora package manifest.' }
    Remove-Item -LiteralPath $stage -Recurse -Force
}
New-Item -ItemType Directory $stage | Out-Null
Set-Content -Encoding ascii -LiteralPath "$stage/.clarora-package-stage" -Value 'Clarora Windows package staging directory'
Copy-Item -Path "$RuntimeDir/*" -Destination $stage -Recurse -Force
Copy-Item -LiteralPath $Executable -Destination "$stage/clarora-gpui.exe"
Copy-Item -LiteralPath "$PSScriptRoot/../README.md" -Destination "$stage/README.md"
New-Item -ItemType Directory -Force "$stage/scripts" | Out-Null
Copy-Item -LiteralPath "$PSScriptRoot/PACKAGING.md" -Destination "$stage/scripts/PACKAGING.md"
Copy-Item -LiteralPath "$PSScriptRoot/../../LICENSE" -Destination "$stage/LICENSE"
$signing = 'unsigned'
if ($CertificateThumbprint) {
    $signTool = (Get-Command signtool.exe -ErrorAction Stop).Source
    # Sign our binaries only; preserve the upstream runtime binaries exactly.
    foreach ($binary in @("$stage/clarora-gpui.exe", "$stage/tools/clarora-asr.exe", "$stage/tools/clarora-tts.exe")) {
        & $signTool sign /sha1 $CertificateThumbprint /fd SHA256 /tr $TimestampUrl /td SHA256 $binary
        if ($LASTEXITCODE -ne 0) { throw "Authenticode signing failed: $binary" }
        if ((Get-AuthenticodeSignature -LiteralPath $binary).Status -ne 'Valid') { throw "Authenticode verification failed: $binary" }
    }
    $signing = 'authenticode-verified'
}
# Build the final inventory after signing. runtime-manifest is the sealed-runtime
# marker used by the app; package-manifest records all final package bytes.
& python "$PSScriptRoot/windows-runtime.py" manifest $stage --version $version --signing $signing
if ($LASTEXITCODE -ne 0) { throw 'Cannot write final Windows package inventory.' }
& python $verifier $stage --report "$Output/package-verification.json" --native
if ($LASTEXITCODE -ne 0) { throw 'Final Windows stage verification failed.' }
# Check the packaged executable with no developer PATH and without opening user data.
$previousPath = $env:PATH
try {
    $env:PATH = "$env:SystemRoot/System32"
    $runtimeReport = Join-Path $Output 'runtime-check.json'
    Remove-Item -LiteralPath $runtimeReport -ErrorAction SilentlyContinue
    $process = Start-Process -FilePath "$stage/clarora-gpui.exe" -ArgumentList @('--runtime-check', "`"$runtimeReport`"") -Wait -PassThru
    if ($process.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $runtimeReport)) { throw 'Packaged app runtime check failed.' }
    if (-not (Get-Content -Raw -LiteralPath $runtimeReport | ConvertFrom-Json).passed) { throw 'Packaged app runtime report did not pass.' }
} finally { $env:PATH = $previousPath }
$zip = Join-Path $Output "Clarora-$version-windows-x64-$signing.zip"
& python "$PSScriptRoot/windows-runtime.py" zip $stage $zip
if ($LASTEXITCODE -ne 0) { throw 'Portable ZIP creation failed.' }
& python $verifier $zip --report "$Output/zip-verification.json" --native
if ($LASTEXITCODE -ne 0) { throw 'Final portable ZIP verification failed.' }
$outputs = @($zip)
if ($Installer) {
    if (-not $InnoSetupCompiler) {
        $command = Get-Command ISCC.exe -ErrorAction SilentlyContinue
        if ($command) { $InnoSetupCompiler = $command.Source }
        elseif (Test-Path "${env:ProgramFiles(x86)}/Inno Setup 6/ISCC.exe") { $InnoSetupCompiler = "${env:ProgramFiles(x86)}/Inno Setup 6/ISCC.exe" }
        else { throw 'Installer requested but Inno Setup 6 ISCC.exe is missing.' }
    }
    $setupName = "Clarora-$version-windows-x64-setup-$signing"
    & $InnoSetupCompiler "/DSourceDir=$stage" "/DOutputDir=$Output" "/DAppVersion=$version" "/DOutputName=$setupName" "$PSScriptRoot/installer/clarora-gpui.iss"
    if ($LASTEXITCODE -ne 0) { throw 'Inno Setup compilation failed.' }
    $setup = Join-Path $Output "$setupName.exe"
    if (-not (Test-Path -LiteralPath $setup)) { throw 'Installer compiler did not produce the requested installer.' }
    if ($CertificateThumbprint) {
        & $signTool sign /sha1 $CertificateThumbprint /fd SHA256 /tr $TimestampUrl /td SHA256 $setup
        if ($LASTEXITCODE -ne 0 -or (Get-AuthenticodeSignature -LiteralPath $setup).Status -ne 'Valid') { throw 'Installer Authenticode verification failed.' }
    }
    $outputs += $setup
}
$artifacts = @($outputs | ForEach-Object {
    @{ path = (Split-Path -Leaf $_); sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $_).Hash.ToLowerInvariant(); size = (Get-Item -LiteralPath $_).Length }
})
@{ schema_version = 1; platform = 'windows'; architecture = 'x86_64'; version = $version; signing = $signing; artifacts = $artifacts } |
    ConvertTo-Json -Depth 6 | Set-Content -Encoding utf8 "$Output/release-manifest.json"
$artifacts | ForEach-Object { "$($_.sha256)  $($_.path)" } | Set-Content -Encoding ascii "$Output/SHA256SUMS.txt"
Write-Host "Verified Windows artifacts ($signing): $Output"
} catch {
    $failure = $_.Exception.Message
    try {
        New-Item -ItemType Directory -Force $Output | Out-Null
        @{ schema_version = 1; status = 'failed'; error = $failure } | ConvertTo-Json | Set-Content -Encoding utf8 "$Output/package-failure.json"
    } catch { Write-Warning 'Could not write package-failure.json.' }
    throw $failure
}
