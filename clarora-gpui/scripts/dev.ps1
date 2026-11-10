# Launch the local GPUI desktop client without requiring Node/npm.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$pdfium = Join-Path $root 'target/debug/pdfium.dll'
if (-not (Test-Path $pdfium)) {
    Write-Host "正在安装固定版本 PDFium：target/debug/pdfium.dll"
    python (Join-Path $root 'scripts/install-pdfium.py') (Join-Path $root 'target/debug')
}
& cargo run --manifest-path (Join-Path $root 'Cargo.toml') --locked -- @args
exit $LASTEXITCODE
