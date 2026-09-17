# 构建 Windows 端侧转写包装 DLL（clarora_asr.dll），并把运行时依赖 DLL
# 复制到 UWP 工程目录（windows/Clarora/），使其进入应用包。
# 依赖：
#   * vcpkg（whisper.cpp 没有官方 Windows 预编译库；GitHub windows runner 自带）
#   * sherpa-onnx 发行包（脚本自动下载）
# 产物缺失时应用仍可构建运行，只是端侧转写在设置中会提示不可用。
$ErrorActionPreference = 'Stop'

function Get-DependencyArchive {
    param([string]$Name, [string]$Url, [string]$Destination)
    Write-Host "Downloading $Name from $Url"
    # Show progress; fail stalled transfers instead of silently waiting for the
    # entire job timeout. Only transient curl failures are retried.
    & curl.exe -fL --connect-timeout 30 --max-time 600 `
        --speed-limit 1024 --speed-time 60 --retry 2 --retry-max-time 900 `
        -o $Destination $Url
    if ($LASTEXITCODE -ne 0) { throw "$Name download failed (curl exit $LASTEXITCODE)." }
    Write-Host "Downloaded $Name ($((Get-Item $Destination).Length) bytes)."
}

Push-Location (Join-Path $PSScriptRoot '..')
try {
    $asrDir = Join-Path (Get-Location) 'windows/clarora-asr'
    $depsDir = Join-Path $asrDir 'deps'
    New-Item -ItemType Directory -Force $depsDir | Out-Null

    # ── whisper.cpp（vcpkg）──
    $vcpkgRoot = $env:VCPKG_ROOT
    if (-not $vcpkgRoot) {
        $command = Get-Command vcpkg -ErrorAction SilentlyContinue
        if ($command) { $vcpkgRoot = Split-Path $command.Source }
    }
    if (-not $vcpkgRoot -and (Test-Path (Join-Path $env:LOCALAPPDATA 'microsoft/vcpkg'))) {
        $vcpkgRoot = Join-Path $env:LOCALAPPDATA 'microsoft/vcpkg'
    }
    if (-not $vcpkgRoot) { throw 'vcpkg is required for whisper.cpp (set VCPKG_ROOT; see https://learn.microsoft.com/vcpkg/get-started).' }
    Write-Host 'Installing whisper.cpp with vcpkg...'
    & (Join-Path $vcpkgRoot 'vcpkg.exe') install whisper-cpp:x64-windows
    if ($LASTEXITCODE -ne 0) { throw 'vcpkg install whisper-cpp:x64-windows failed.' }
    $vcpkgInstalled = Join-Path $vcpkgRoot 'installed/x64-windows'
    Write-Host 'Preparing whisper.cpp include and library links...'
    New-Item -ItemType Directory -Force (Join-Path $depsDir 'whisper.cpp') | Out-Null
    New-Item -ItemType SymbolicLink -Path (Join-Path $depsDir 'whisper.cpp/include') -Target (Join-Path $vcpkgInstalled 'include') -Force | Out-Null
    New-Item -ItemType SymbolicLink -Path (Join-Path $depsDir 'whisper.cpp/lib') -Target (Join-Path $vcpkgInstalled 'lib') -Force | Out-Null

    # ── sherpa-onnx（发行包，MD Release，无 TTS）──
    $sherpaVersion = 'v1.13.8'
    $sherpaName = "sherpa-onnx-$sherpaVersion-win-x64-shared-MD-Release-no-tts-lib.tar.bz2"
    $sherpaDir = Join-Path $depsDir 'sherpa-onnx'
    if (-not (Test-Path (Join-Path $sherpaDir 'lib'))) {
        New-Item -ItemType Directory -Force $sherpaDir | Out-Null
        $archive = Join-Path $env:TEMP $sherpaName
        Get-DependencyArchive -Name 'sherpa-onnx' -Destination $archive `
            -Url "https://github.com/k2-fsa/sherpa-onnx/releases/download/$sherpaVersion/$sherpaName"
        Write-Host 'Extracting sherpa-onnx...'
        & tar -xjf $archive -C $sherpaDir
        if ($LASTEXITCODE -ne 0) { throw 'sherpa-onnx extraction failed.' }
        Get-ChildItem $sherpaDir -Directory | ForEach-Object {
            Get-ChildItem $_.FullName | ForEach-Object { Move-Item -Force $_.FullName $sherpaDir }
        }
    }
    $sherpaInclude = (Get-ChildItem $sherpaDir -Recurse -Directory -Filter 'include' | Select-Object -First 1).FullName
    $sherpaLib = (Get-ChildItem $sherpaDir -Recurse -Directory -Filter 'lib' | Select-Object -First 1).FullName
    if (-not $sherpaInclude -or -not $sherpaLib) { throw 'sherpa-onnx package layout unexpected (include/lib not found).' }

    # ── pdfium（PDF 渲染，bblanchon 发行包）──
    $pdfiumTag = 'chromium/8057'
    $pdfiumArchive = Join-Path $env:TEMP 'pdfium-win-x64.tgz'
    $pdfiumDir = Join-Path $depsDir 'pdfium'
    if (-not (Test-Path (Join-Path $pdfiumDir 'bin/pdfium.dll'))) {
        New-Item -ItemType Directory -Force $pdfiumDir | Out-Null
        Get-DependencyArchive -Name 'pdfium' -Destination $pdfiumArchive `
            -Url "https://github.com/bblanchon/pdfium-binaries/releases/download/$pdfiumTag/pdfium-win-x64.tgz"
        Write-Host 'Extracting pdfium...'
        & tar -xzf $pdfiumArchive -C $pdfiumDir
        if ($LASTEXITCODE -ne 0) { throw 'pdfium extraction failed.' }
    }

    # ── CMake 构建（MSVC x64）──
    $buildDir = Join-Path $asrDir 'build'
    Write-Host 'Configuring clarora_asr with CMake...'
    & cmake -S $asrDir -B $buildDir -A x64 `
        "-DCMAKE_TOOLCHAIN_FILE=$(Join-Path $vcpkgRoot 'scripts/buildsystems/vcpkg.cmake')" `
        "-DSHERPA_INCLUDE_DIR=$sherpaInclude" `
        "-DSHERPA_LIB_DIR=$sherpaLib"
    if ($LASTEXITCODE -ne 0) { throw 'CMake configure failed.' }
    Write-Host 'Building clarora_asr (Release)...'
    & cmake --build $buildDir --config Release
    if ($LASTEXITCODE -ne 0) { throw 'clarora_asr.dll build failed.' }

    # ── 依赖 DLL 汇总到 UWP 工程目录（csproj 以 Content 打包）──
    $packageDir = Join-Path (Get-Location) 'windows/Clarora'
    Write-Host 'Copying ASR and PDF runtime DLLs into the application...'
    New-Item -ItemType Directory -Force $packageDir | Out-Null
    Copy-Item -Force (Join-Path $buildDir 'Release/clarora_asr.dll') $packageDir
    foreach ($source in @(
        (Join-Path $vcpkgInstalled 'bin'),
        (Get-ChildItem $sherpaDir -Recurse -Directory -Filter 'bin' | Select-Object -First 1).FullName,
        (Join-Path $pdfiumDir 'bin')
    )) {
        if ($source -and (Test-Path $source)) {
            Get-ChildItem $source -Filter '*.dll' | ForEach-Object { Copy-Item -Force $_.FullName $packageDir }
        }
    }
    Write-Host "clarora_asr.dll and runtime DLLs placed in $packageDir"
} finally { Pop-Location }
