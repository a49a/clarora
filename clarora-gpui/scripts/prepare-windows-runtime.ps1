param(
    [string]$Output = "$PSScriptRoot/../target/windows-runtime",
    [string]$WorkDir = "$PSScriptRoot/../target/windows-dependencies",
    [string]$MsvcRedistDir = "$env:VCToolsRedistDir/x64/Microsoft.VC143.CRT",
    [string]$MsvcLicenseDir = "$env:VSINSTALLDIR/Licenses"
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
try {
if ($env:OS -ne 'Windows_NT') { throw 'Windows runtime preparation requires Windows x64 and the MSVC C++ build tools.' }
if (-not $env:VCToolsRedistDir -and $MsvcRedistDir -eq '/x64/Microsoft.VC143.CRT') { throw 'Initialize the x64 MSVC developer environment or pass -MsvcRedistDir and -MsvcLicenseDir.' }
$WorkDir = [IO.Path]::GetFullPath($WorkDir)
$Output = [IO.Path]::GetFullPath($Output)
& cargo fetch --manifest-path "$PSScriptRoot/../Cargo.toml" --locked --target x86_64-pc-windows-msvc
if ($LASTEXITCODE -ne 0) { throw 'Cannot fetch locked Windows Rust dependencies for the license inventory.' }
& python "$PSScriptRoot/windows-runtime.py" fetch $WorkDir
if ($LASTEXITCODE -ne 0) { throw 'Pinned Windows runtime preparation failed.' }
$inputs = Get-Content -Raw "$WorkDir/build-inputs.json" | ConvertFrom-Json
$whisperInstall = Join-Path $WorkDir 'whisper-install'
& cmake -S $inputs.whisper_source -B "$WorkDir/whisper-build" -A x64 `
    "-DCMAKE_INSTALL_PREFIX=$whisperInstall" '-DBUILD_SHARED_LIBS=ON' '-DGGML_BACKEND_DL=ON' `
    '-DGGML_NATIVE=OFF' '-DGGML_OPENMP=OFF' '-DGGML_AVX=OFF' '-DGGML_AVX2=OFF' '-DGGML_FMA=OFF' '-DGGML_F16C=OFF' `
    '-DWHISPER_BUILD_TESTS=OFF' '-DWHISPER_BUILD_EXAMPLES=OFF'
if ($LASTEXITCODE -ne 0) { throw 'Whisper CMake configure failed.' }
& cmake --build "$WorkDir/whisper-build" --config Release --parallel 4
if ($LASTEXITCODE -ne 0) { throw 'Whisper native build failed.' }
& cmake --install "$WorkDir/whisper-build" --config Release
if ($LASTEXITCODE -ne 0) { throw 'Whisper install failed.' }
& cmake -S "$PSScriptRoot/../native" -B "$WorkDir/native-build" -A x64 `
    "-DCMAKE_PREFIX_PATH=$whisperInstall" "-DSHERPA_INCLUDE=$($inputs.sherpa_include)" "-DSHERPA_LIBRARY=$($inputs.sherpa_library)" '-DCMAKE_CXX_FLAGS=/utf-8'
if ($LASTEXITCODE -ne 0) { throw 'Clarora native ASR/TTS configure failed.' }
& cmake --build "$WorkDir/native-build" --config Release --parallel 4
if ($LASTEXITCODE -ne 0) { throw 'Clarora native ASR/TTS build failed.' }
& python "$PSScriptRoot/windows-runtime.py" stage $WorkDir $Output `
    --msvc-redist $MsvcRedistDir --msvc-licenses $MsvcLicenseDir
if ($LASTEXITCODE -ne 0) { throw 'Windows runtime staging failed.' }
& python "$PSScriptRoot/verify-windows-package.py" $Output --runtime --native --report "$WorkDir/runtime-verification.json"
if ($LASTEXITCODE -ne 0) { throw 'Windows runtime verification failed.' }
Write-Host "Complete Windows runtime: $Output"
} catch {
    $failure = $_.Exception.Message
    try {
        New-Item -ItemType Directory -Force $WorkDir | Out-Null
        @{ schema_version = 1; status = 'failed'; error = $failure } | ConvertTo-Json | Set-Content -Encoding utf8 "$WorkDir/runtime-build-failure.json"
    } catch { Write-Warning 'Could not write runtime-build-failure.json.' }
    throw $failure
}
