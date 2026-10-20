[CmdletBinding()]
param(
  [ValidateSet("x64", "Win32")]
  [string]$Architecture = "x64",

  [string]$BoostRoot = $env:BOOST_ROOT
)

$ErrorActionPreference = "Stop"

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$rimeRoot = Join-Path $repositoryRoot "third_party\librime"
$cmakeDirectory = "C:\Program Files\CMake\bin"
$cmakeExecutable = Join-Path $cmakeDirectory "cmake.exe"

if (-not (Test-Path $cmakeExecutable)) {
  throw "CMake was not found at $cmakeExecutable."
}

if (-not (Test-Path (Join-Path $rimeRoot "build.bat"))) {
  throw "The pinned librime submodule is unavailable at $rimeRoot."
}

if ([string]::IsNullOrWhiteSpace($BoostRoot)) {
  $BoostRoot = Join-Path $rimeRoot "deps\boost-1.89.0"
}
if (-not (Test-Path (Join-Path $BoostRoot "boost"))) {
  throw "Set BOOST_ROOT to an existing Boost source tree. This script does not download Boost."
}

$vswhere = "C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path $vswhere)) {
  throw "Visual Studio 2022 C++ tools were not found. Install the C++ desktop workload before building."
}

$vsPath = & $vswhere -latest -products "*" -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if ([string]::IsNullOrWhiteSpace($vsPath)) {
  throw "Visual Studio 2022 with the x86/x64 C++ tools is required."
}

$env:PATH = "$cmakeDirectory;$env:PATH"
$env:BOOST_ROOT = $BoostRoot
$env:ARCH = $Architecture
$env:CMAKE_GENERATOR = '"Visual Studio 17 2022"'
$env:PLATFORM_TOOLSET = "v143"
$env:build_dir = "build-$Architecture"
$env:deps_install_prefix = "deps-$Architecture"
$env:rime_install_prefix = "dist-$Architecture"

Push-Location $rimeRoot
try {
  & ".\build.bat" deps
  if ($LASTEXITCODE -ne 0) {
    throw "librime dependency build failed with exit code $LASTEXITCODE."
  }

  & ".\build.bat" shared librime
  if ($LASTEXITCODE -ne 0) {
    throw "librime build failed with exit code $LASTEXITCODE."
  }
}
finally {
  Pop-Location
}
