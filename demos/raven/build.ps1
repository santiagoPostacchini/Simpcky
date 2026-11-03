# Compila la demo de Simpcky con Raven.
#
#   1. Baja Raven (el commit fijado en Cargo.toml) a .raven\src si no está.
#   2. Lo compila como DLL con MSVC + Ninja (necesita Visual Studio 2022 con
#      "Desarrollo para el escritorio con C++"; CMake y Ninja se instalan con
#      pip si no están).
#   3. Compila la demo y deja raven.dll junto a simpcky-raven.exe.
#
# Uso:  powershell -ExecutionPolicy Bypass -File demos\raven\build.ps1

$ErrorActionPreference = "Stop"
$here = $PSScriptRoot
$rev = "3b1a4df67cc453f68f366ebdd4466c518e318795"
$src = Join-Path $here ".raven\src"
$build = Join-Path $src "build"

if (-not (Test-Path $src)) {
    git clone https://github.com/oscar-soirey/Raven $src
}
git -C $src fetch --depth 1 origin $rev 2>$null
git -C $src checkout --quiet $rev

if (-not (Get-Command cmake -ErrorAction SilentlyContinue)) {
    python -m pip install --user --quiet cmake ninja
    $scripts = python -c "import sysconfig;print(sysconfig.get_path('scripts','nt_user'))"
    $cmakeBin = python -c "import cmake;print(cmake.CMAKE_BIN_DIR)"
    $env:PATH = "$cmakeBin;$scripts;$env:PATH"
}

if (-not (Test-Path (Join-Path $build "raven.lib"))) {
    $vs = & "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe" -latest -property installationPath
    $vcvars = Join-Path $vs "VC\Auxiliary\Build\vcvars64.bat"
    $cmd = "call `"$vcvars`" >nul && cmake -S `"$src`" -B `"$build`" -G Ninja -DCMAKE_BUILD_TYPE=Release -DRAVEN_BUILD_SHARED=ON && cmake --build `"$build`" --target raven --parallel"
    cmd /c $cmd
    if ($LASTEXITCODE -ne 0) { throw "No se pudo compilar Raven" }
}

$env:RAVEN_LIB_DIR = $build
Push-Location $here
try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "No se pudo compilar la demo" }
} finally {
    Pop-Location
}
Copy-Item (Join-Path $build "bin\raven.dll") (Join-Path $here "target\release\raven.dll") -Force
Write-Host "Listo: $here\target\release\simpcky-raven.exe"
