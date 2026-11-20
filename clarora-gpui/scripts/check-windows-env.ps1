param(
    [switch]$Runtime,
    [switch]$Installer,
    [switch]$AllowNewerRust,
    [string]$Json,
    [string]$MinimumPython = '3.10'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$checks = [System.Collections.Generic.List[object]]::new()
$missing = [System.Collections.Generic.List[string]]::new()
$warnings = [System.Collections.Generic.List[string]]::new()

function Add-Check {
    param(
        [string]$Name,
        [bool]$Ok,
        [string]$Detail,
        [string]$Hint = ''
    )
    $check = [ordered]@{
        name = $Name
        ok = $Ok
        detail = $Detail
    }
    if ($Hint) {
        $check['hint'] = $Hint
    }
    $checks.Add($check)
    if ($Ok) {
        Write-Host "[ok] $Name - $Detail" -ForegroundColor Green
    } else {
        $missing.Add($Name)
        Write-Host "[!!] $Name - $Detail" -ForegroundColor Red
        if ($Hint) {
            Write-Host "     hint: $Hint" -ForegroundColor DarkGray
        }
    }
}

function Add-Warning {
    param([string]$Text)
    $warnings.Add($Text)
    Write-Host "[--] $Text" -ForegroundColor Yellow
}

function Get-CommandPath {
    param([string]$Name)
    $command = Get-Command $Name -ErrorAction SilentlyContinue
    if ($command -and $command.Source) { return $command.Source }
    return $null
}

function Get-VersionText {
    param([string]$Text)
    if ($Text -match '(\d+\.\d+\.\d+)') { return $Matches[1] }
    return ''
}

function Resolve-ProgramFile {
    param(
        [string]$Command,
        [string[]]$Candidates
    )
    $path = Get-CommandPath $Command
    if ($path) { return $path }
    foreach ($candidate in $Candidates) {
        if ($candidate -and (Test-Path -LiteralPath $candidate)) {
            return (Get-Item -LiteralPath $candidate).FullName
        }
    }
    return $null
}

# Operating system and shell -------------------------------------------------

$isWindowsHost = ([Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT)
$architecture = if ($env:PROCESSOR_ARCHITECTURE) { $env:PROCESSOR_ARCHITECTURE } else { 'unknown' }
Add-Check 'Windows x64' ($isWindowsHost -and $architecture -eq 'AMD64') `
    "$([Environment]::OSVersion.VersionString) / $architecture" `
    'Run this script on Windows 10/11 x64.'

$psVersion = $PSVersionTable.PSVersion
Add-Check 'PowerShell 7+' ($psVersion.Major -ge 7) "$psVersion" `
    'Install PowerShell 7: winget install Microsoft.PowerShell'

# Rust toolchain pinned by CI ------------------------------------------------

$expectedRust = '1.93.1'
$rustc = Get-CommandPath 'rustc.exe'
if (-not $rustc) {
    Add-Check 'Rust toolchain' $false 'rustc.exe is not on PATH' `
        "Install Rust 1.93.1 with rustup, then add the MSVC target and components."
} else {
    $rustVersionText = (& $rustc --version 2>&1 | Out-String).Trim()
    $rustVersion = Get-VersionText $rustVersionText
    $matchesPinned = $rustVersion -eq $expectedRust
    $matchesNewer = $false
    if ($AllowNewerRust -and $rustVersion) {
        $matchesNewer = ([version]$rustVersion -ge [version]$expectedRust)
    }
    Add-Check 'Rust toolchain' ($matchesPinned -or $matchesNewer) $rustVersionText `
        "Use rustup toolchain install $expectedRust (or pass -AllowNewerRust)."
}

$cargo = Get-CommandPath 'cargo.exe'
if (-not $cargo) {
    Add-Check 'Cargo' $false 'cargo.exe is not on PATH' 'Install Rust with rustup.'
} else {
    $cargoVersionText = (& $cargo --version 2>&1 | Out-String).Trim()
    Add-Check 'Cargo' $true $cargoVersionText
}

$rustup = Get-CommandPath 'rustup.exe'
if (-not $rustup) {
    Add-Check 'Rust components' $false 'rustup.exe is not on PATH' `
        'Install Rust with rustup so rustfmt, clippy and the MSVC target can be checked.'
} else {
    $installedComponents = @(& $rustup component list --installed 2>&1 | ForEach-Object { "$_".Trim() })
    foreach ($component in @('rustfmt', 'clippy')) {
        $present = (@($installedComponents | Where-Object { $_ -match "^$component" }).Count -gt 0)
        Add-Check "Rust component: $component" $present `
            $(if ($present) { 'installed' } else { 'missing' }) `
            "rustup component add $component"
    }
    $installedTargets = @(& $rustup target list --installed 2>&1 | ForEach-Object { "$_".Trim() })
    Add-Check 'Rust target: x86_64-pc-windows-msvc' `
        ($installedTargets -contains 'x86_64-pc-windows-msvc') `
        'x86_64-pc-windows-msvc' `
        'rustup target add x86_64-pc-windows-msvc'
}

# Visual Studio / Windows SDK ------------------------------------------------

$vcTools = $env:VCToolsInstallDir
$vcRedist = $env:VCToolsRedistDir
$windowsSdk = $env:WindowsSdkDir
$cl = Get-CommandPath 'cl.exe'
$link = Get-CommandPath 'link.exe'
Add-Check 'Visual Studio C++ tools' `
    ([bool]$vcTools -and (Test-Path -LiteralPath $vcTools) -and [bool]$cl -and [bool]$link) `
    $(if ($cl) { $cl } else { 'cl.exe is not on PATH' }) `
    'Open an x64 Native Tools Command Prompt for VS 2022 (or run the VS Developer PowerShell).'
Add-Check 'Windows SDK' ([bool]$windowsSdk -and (Test-Path -LiteralPath $windowsSdk)) `
    $(if ($windowsSdk) { $windowsSdk } else { 'WindowsSdkDir is not set' }) `
    'Install the Windows 10/11 SDK from the Visual Studio 2022 installer.'

# Python ---------------------------------------------------------------------

$python = Get-CommandPath 'python.exe'
if (-not $python) {
    Add-Check 'Python' $false 'python.exe is not on PATH' `
        'Install Python 3.12: winget install Python.Python.3.12'
} else {
    $pythonVersionText = (& $python --version 2>&1 | Out-String).Trim()
    $pythonVersion = Get-VersionText $pythonVersionText
    $pythonOk = $false
    if ($pythonVersion) {
        $pythonOk = ([version]$pythonVersion -ge [version]$MinimumPython)
    }
    $pythonHint = "Use Python $MinimumPython or newer."
    Add-Check 'Python' $pythonOk $pythonVersionText $pythonHint
}

# Runtime and installer dependencies ----------------------------------------

if ($Runtime -or $Installer) {
    $cmake = Get-CommandPath 'cmake.exe'
    if (-not $cmake) {
        Add-Check 'CMake' $false 'cmake.exe is not on PATH' `
            'Install CMake with the Visual Studio installer or: winget install Kitware.CMake'
    } else {
        $cmakeVersionText = (& $cmake --version 2>&1 | Select-Object -First 1).Trim()
        Add-Check 'CMake' $true $cmakeVersionText
        $generators = (& $cmake --help 2>&1 | Out-String)
        Add-Check 'CMake Visual Studio 2022 generator' `
            ($generators -match 'Visual Studio 17 2022') `
            'Visual Studio 17 2022' `
            'Install the Visual Studio 2022 C++ workload and CMake.'
    }

    $programFiles = if ($env:ProgramFiles) { $env:ProgramFiles } else { '' }
    $sevenZipCandidates = @()
    if ($programFiles) {
        $sevenZipCandidates += (Join-Path $programFiles '7-Zip\7z.exe')
    }
    $sevenZip = Resolve-ProgramFile '7z.exe' $sevenZipCandidates
    Add-Check '7-Zip' ([bool]$sevenZip) `
        $(if ($sevenZip) { $sevenZip } else { '7z.exe is not on PATH' }) `
        'Install 7-Zip: winget install 7zip.7zip'

    Add-Check 'MSVC redistributable files' `
        ([bool]$vcRedist -and (Test-Path -LiteralPath $vcRedist)) `
        $(if ($vcRedist) { $vcRedist } else { 'VCToolsRedistDir is not set' }) `
        'Open an x64 Native Tools Command Prompt for VS 2022.'
}

if ($Installer) {
    $programFilesX86 = if (${env:ProgramFiles(x86)}) { ${env:ProgramFiles(x86)} } else { '' }
    $isccCandidates = @()
    if ($programFilesX86) {
        $isccCandidates += (Join-Path $programFilesX86 'Inno Setup 6\ISCC.exe')
    }
    $iscc = Resolve-ProgramFile 'ISCC.exe' $isccCandidates
    Add-Check 'Inno Setup 6' ([bool]$iscc) `
        $(if ($iscc) { $iscc } else { 'ISCC.exe is not on PATH' }) `
        'Install Inno Setup 6: winget install JRSoftware.InnoSetup'
}

# Report ---------------------------------------------------------------------

$passed = $missing.Count -eq 0
$platform = if ($isWindowsHost) { 'windows' } else { 'other' }
$scope = if ($Installer) { 'installer' } elseif ($Runtime) { 'runtime' } else { 'core' }
$report = [ordered]@{
    schema_version = 1
    platform = $platform
    scope = $scope
    passed = $passed
    checks = $checks
    missing = $missing
    warnings = $warnings
}

if ($Json) {
    $path = [IO.Path]::GetFullPath($Json)
    $directory = Split-Path -Parent $path
    if ($directory) {
        New-Item -ItemType Directory -Force -Path $directory | Out-Null
    }
    $report | ConvertTo-Json -Depth 6 | Set-Content -Encoding utf8 -LiteralPath $path
    Write-Host "Report: $path"
}

if (-not $passed) {
    Write-Host ''
    Write-Host "Windows environment is missing $($missing.Count) required item(s)." -ForegroundColor Red
    exit 1
}

Write-Host ''
Write-Host 'Windows environment is ready.' -ForegroundColor Green
exit 0
