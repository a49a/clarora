param(
    [string]$Output = "$PSScriptRoot/../target/package",
    [switch]$Distribution,
    [string]$RuntimeDir,
    [switch]$Installer,
    [string]$CertificateThumbprint,
    [string]$TimestampUrl = 'http://timestamp.digicert.com'
)
$ErrorActionPreference = 'Stop'
$manifest = Join-Path $PSScriptRoot '../Cargo.toml'
& cargo build --manifest-path $manifest --release --locked
if ($LASTEXITCODE -ne 0) { throw 'Rust desktop build failed.' }
if ($Distribution) {
    if (-not $RuntimeDir) { throw '-Distribution requires an explicit -RuntimeDir. Run prepare-windows-runtime.ps1 first.' }
    if (-not $PSBoundParameters.ContainsKey('Output')) { $Output = "$PSScriptRoot/../../../artifacts/clarora/gpui/windows" }
    & "$PSScriptRoot/package-windows.ps1" -Output $Output -RuntimeDir $RuntimeDir `
        -Installer:$Installer -CertificateThumbprint $CertificateThumbprint -TimestampUrl $TimestampUrl
    return
}
New-Item -ItemType Directory -Force $Output | Out-Null
Copy-Item "$PSScriptRoot/../target/release/clarora-gpui.exe" $Output
Copy-Item "$PSScriptRoot/../README.md" "$Output/README.md"
Copy-Item "$PSScriptRoot/../../LICENSE" "$Output/LICENSE"
Write-Host "GPUI developer executable: $Output"
Write-Warning 'This developer output is incomplete and must not be distributed. Use -Distribution -RuntimeDir <verified-runtime> for a portable ZIP and optionally -Installer.'
