param([string]$Output = "$PSScriptRoot/../target/package")
$ErrorActionPreference = 'Stop'
$manifest = Join-Path $PSScriptRoot '../Cargo.toml'
& cargo build --manifest-path $manifest --release --locked
if ($LASTEXITCODE -ne 0) { throw 'Rust desktop build failed.' }
New-Item -ItemType Directory -Force $Output | Out-Null
Copy-Item "$PSScriptRoot/../target/release/clarora-gpui.exe" $Output
Copy-Item "$PSScriptRoot/../README.md" "$Output/README.md"
Copy-Item "$PSScriptRoot/../../LICENSE" "$Output/LICENSE"
Write-Host "GPUI desktop package: $Output"
Write-Host 'Place mpv-2.dll (and its dependencies) and pdfium.dll beside the executable.'
