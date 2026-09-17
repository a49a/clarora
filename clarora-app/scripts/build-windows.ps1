$ErrorActionPreference = 'Stop'
Push-Location (Join-Path $PSScriptRoot '..')
try {
    # Use the same shared entry point as macOS, Android and iOS.
    & npx react-native autolink-windows --no-telemetry
    if ($LASTEXITCODE -ne 0) { throw 'Windows native module linking failed.' }
    $bundleDirectory = Join-Path (Get-Location) 'windows/Clarora/Bundle'
    New-Item -ItemType Directory -Force $bundleDirectory | Out-Null
    & npx react-native bundle --platform windows --dev false --entry-file index.js --bundle-output "$bundleDirectory/index.windows.bundle" --assets-dest $bundleDirectory
    if ($LASTEXITCODE -ne 0) { throw 'Windows JavaScript bundle failed.' }
    $msbuild = Get-Command msbuild -ErrorAction SilentlyContinue
    if (-not $msbuild) {
        $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
        $path = & $vswhere -latest -products '*' -requires Microsoft.Component.MSBuild -find 'MSBuild\**\Bin\MSBuild.exe' | Select-Object -First 1
        if (-not $path) { throw 'Install Visual Studio 2022 with UWP and C++ development tools.' }
    } else { $path = $msbuild.Source }
    $packages = Join-Path (Get-Location) 'windows/AppPackages/'
    # The managed code generator is invoked through a raw MSBuild task (not a
    # ProjectReference), so the solution-level /restore does not cover it and a
    # fresh machine has no obj/project.assets.json for it yet.
    & $path node_modules/react-native-windows/Microsoft.ReactNative.Managed.CodeGen/Microsoft.ReactNative.Managed.CodeGen.csproj /t:Restore
    if ($LASTEXITCODE -ne 0) { throw 'NuGet restore for the managed code generator failed.' }
    & $path windows/Clarora.sln /restore /m /p:Configuration=Release /p:Platform=x64 /p:AppxBundle=Never /p:AppxBundlePlatforms=x64 /p:UapAppxPackageBuildMode=SideloadOnly /p:AppxPackageSigningEnabled=false /p:GenerateAppxPackageOnBuild=true "/p:AppxPackageDir=$packages"
    if ($LASTEXITCODE -ne 0) { throw 'Windows application build failed.' }
    Write-Host "Unsigned application packages: $packages"
} finally { Pop-Location }
