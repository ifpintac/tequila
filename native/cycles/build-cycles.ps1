# Build tequila_cycles.dll: Cycles (Apache-2.0) with Tequila's C interface.
#
# Cycles is built from the standalone repository at v5.0.0, with its engine
# sources brought to Blender 5.0.1's, so that the GPU kernels shipped with
# Blender 5.0.1 (kernel_sm_*.cubin, kernel_optix*.ptx) match it exactly and no
# CUDA toolkit is needed to try the GPU. A release build will compile its own
# kernels instead.
#
# Usage:  .\native\cycles\build-cycles.ps1 [-Work C:\path\to\work]
# Needs:  Git with LFS, Python 3, Visual Studio Build Tools (with CMake).
# Result: <Work>\cycles\build\bin\tequila_cycles.dll and its libraries.
#         Point TEQUILA_CYCLES at that folder (or copy it to a `cycles`
#         folder beside tequila.exe), and TEQUILA_CYCLES_KERNELS at
#         <Work>\kernels for the GPU.

param(
    [string]$Work = (Join-Path (Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))) 'cycles-spike'),
    [string]$Blender = 'C:\Program Files\Blender Foundation\Blender 5.0',
    # CUDA alone, without NVIDIA's OptiX headers (see THIRD_PARTY_LICENSES.md).
    [switch]$NoOptiX
)
$ErrorActionPreference = 'Stop'
$shim = $PSScriptRoot
New-Item -ItemType Directory -Force $Work | Out-Null
Set-Location $Work

# 1. Cycles, standalone, v5.0.0.
if (-not (Test-Path cycles)) {
    git clone --depth 1 --branch v5.0.0 https://github.com/blender/cycles.git cycles
}

# 2. Only the 17 precompiled libraries Cycles needs (about 230 MB, not the
#    whole 48 GB library repository), at the commit Cycles pins.
$lib = Join-Path $Work 'lib\windows_x64'
if (-not (Test-Path (Join-Path $lib 'embree'))) {
    $commit = (git -C cycles ls-tree HEAD lib/windows_x64).Split()[2]
    New-Item -ItemType Directory -Force $lib | Out-Null
    git -C $lib init -q
    git -C $lib remote add origin https://projects.blender.org/blender/lib-windows_x64.git
    git -C $lib fetch -q --depth 1 --filter=blob:none origin $commit
    $env:MSYS_NO_PATHCONV = '1'
    git -C $lib sparse-checkout set --no-cone /OpenImageIO/ /pugixml/ /openjpeg/ /jpeg/ /tiff/ /webp/ /png/ /zlib/ /openexr/ /imath/ /tbb/ /embree/ /openimagedenoise/ /zstd/ /opencolorio/ /openpgl/ /pthreads/ /.gitattributes
    git -C $lib checkout -q FETCH_HEAD
}
if (-not (Test-Path cycles\lib\windows_x64\embree)) {
    Remove-Item -Force -Recurse cycles\lib\windows_x64 -ErrorAction SilentlyContinue
    New-Item -ItemType Junction -Path cycles\lib\windows_x64 -Target $lib | Out-Null
}

# 3. Engine sources at Blender 5.0.1, leaving the standalone build files and
#    its eager node registration (graph/node_type.h) alone.
if (-not (Test-Path blender501\intern\cycles)) {
    git clone -q --filter=blob:none --no-checkout --depth 1 --branch v5.0.1 https://github.com/blender/blender.git blender501
    $env:MSYS_NO_PATHCONV = '1'
    git -C blender501 sparse-checkout set --no-cone /intern/cycles/
    git -C blender501 checkout -q v5.0.1
}
python -c @"
import os, shutil
A, B = 'cycles/src', 'blender501/intern/cycles'
keep = lambda p: p.endswith('CMakeLists.txt') or p.startswith(('cmake/', 'app/', 'hydra/')) or p in ('util/guarded_allocator.h', 'graph/node_type.h')
for d, _, fs in os.walk(B):
    for f in fs:
        src = os.path.join(d, f); p = os.path.relpath(src, B).replace(os.sep, '/'); dst = os.path.join(A, p)
        if os.path.exists(dst) and not keep(p) and open(src, 'rb').read().replace(b'\r\n', b'\n') != open(dst, 'rb').read().replace(b'\r\n', b'\n'):
            shutil.copyfile(src, dst)
"@

# 3b. NVIDIA's OptiX headers, at the version Blender 5.0.1's kernels were
#     built with, unless asked not to.
$optix = Join-Path $Work 'optix-8.0.0'
if (-not $NoOptiX -and -not (Test-Path (Join-Path $optix 'include\optix.h'))) {
    git clone -q --depth 1 --branch v8.0.0 https://github.com/NVIDIA/optix-dev.git $optix
}
$optixFlags = if ($NoOptiX) { '-DWITH_CYCLES_DEVICE_OPTIX=OFF' } else { "-DWITH_CYCLES_DEVICE_OPTIX=ON -DOPTIX_ROOT_DIR=$($optix -replace '\\', '/')" }

# 4. The shim target, once.
$app = 'cycles\src\app\CMakeLists.txt'
if (-not (Select-String -Path $app -Pattern 'tequila_cycles' -Quiet)) {
    Add-Content $app @'

# Tequila's C interface over Cycles, built with the same flags and libraries
# as the standalone app. TEQUILA_SHIM_DIR points at its sources.
if(TEQUILA_SHIM_DIR)
  add_library(tequila_cycles SHARED
    ${TEQUILA_SHIM_DIR}/tequila_cycles.cpp
    ${TEQUILA_SHIM_DIR}/tequila_cycles.h
  )
  target_link_libraries(tequila_cycles PRIVATE ${LIB})
endif()
'@
}
# Tequila's own block, not any: a work folder that built Cycles for
# Freeshot, as Tequila was called, has Freeshot's, and taking that for this
# one left the library without OpenColorIO, which it refuses to render
# without.
if (-not (Select-String -Path $app -Pattern 'tequila_cycles PRIVATE FSC_WITH_OCIO' -SimpleMatch -Quiet)) {
    Add-Content $app @'

# OpenColorIO for the shim itself: it sets the config Cycles works in.
if(TEQUILA_SHIM_DIR AND WITH_OPENCOLORIO)
  target_include_directories(tequila_cycles PRIVATE ${OPENCOLORIO_INCLUDE_DIRS})
  target_link_libraries(tequila_cycles PRIVATE ${OPENCOLORIO_LIBRARIES})
  target_compile_definitions(tequila_cycles PRIVATE FSC_WITH_OCIO)
  if(WIN32 AND NOT USD_OVERRIDE_OPENCOLORIO)
    target_compile_definitions(tequila_cycles PRIVATE OpenColorIO_SKIP_IMPORTS)
  endif()
endif()
'@
}

# 5. Configure and build with the VS Build Tools' own CMake and Ninja.
$vs = & "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -property installationPath
$cmakeDir = Join-Path $vs 'Common7\IDE\CommonExtensions\Microsoft\CMake'
$shimDir = $shim -replace '\\', '/'
$bat = @"
@echo off
call "$vs\VC\Auxiliary\Build\vcvars64.bat" >nul || exit /b 1
set "PATH=$cmakeDir\CMake\bin;$cmakeDir\Ninja;%PATH%"
cd /d "$Work\cycles" || exit /b 1
cmake -G Ninja -B build -DCMAKE_BUILD_TYPE=Release -DWITH_CYCLES_ALEMBIC=OFF -DWITH_CYCLES_OPENSUBDIV=OFF -DWITH_CYCLES_OPENVDB=OFF -DWITH_CYCLES_NANOVDB=OFF -DWITH_CYCLES_OSL=OFF -DWITH_CYCLES_USD=OFF -DWITH_CYCLES_HYDRA_RENDER_DELEGATE=OFF -DWITH_CYCLES_STANDALONE_GUI=OFF -DWITH_CYCLES_DEVICE_CUDA=ON -DWITH_CUDA_DYNLOAD=ON -DWITH_CYCLES_CUDA_BINARIES=OFF $optixFlags -DWITH_CYCLES_DEVICE_HIP=ON -DWITH_HIP_DYNLOAD=ON -DWITH_CYCLES_HIP_BINARIES=OFF -DWITH_CYCLES_DEVICE_HIPRT=OFF -DWITH_CYCLES_DEVICE_ONEAPI=OFF -DWITH_STRICT_BUILD_OPTIONS=ON -DWITH_CYCLES_PATH_GUIDING=ON -DTEQUILA_SHIM_DIR=$shimDir || exit /b 1
cmake --build build --target tequila_cycles || exit /b 1
"@
Set-Content -Encoding ascii (Join-Path $Work 'build_tequila_cycles.bat') $bat
cmd /c (Join-Path $Work 'build_tequila_cycles.bat')
if ($LASTEXITCODE -ne 0) { throw 'Cycles build failed' }

# 6. Beside the DLL: Cycles' libraries, with Tequila's own Embree, TBB and
#    OIDN in place of Blender's, so one copy of each is loaded. (Blender's
#    Embree also wants Intel's SYCL runtime, which Tequila does not ship.)
$bin = Join-Path $Work 'cycles\build\bin'
$repo = Split-Path -Parent (Split-Path -Parent $shim)
foreach ($d in 'OpenImageIO', 'opencolorio', 'openexr', 'imath') {
    Copy-Item (Join-Path $lib "$d\bin\*.dll") $bin -Exclude '*_d.dll', '*_d_*.dll'
}
Copy-Item (Join-Path $repo 'third_party\embree-4.4.1.x64.windows\bin\embree4.dll'), (Join-Path $repo 'third_party\embree-4.4.1.x64.windows\bin\tbb12.dll') $bin -Force
Copy-Item (Join-Path $repo 'third_party\oidn-2.5.1.x64.windows\bin\OpenImageDenoise*.dll') $bin -Force

# 7. GPU kernels from Blender 5.0.1, if it is installed.
$kernels = Join-Path $Work 'kernels\lib'
$source = Join-Path $Blender '5.0\scripts\addons_core\cycles\lib'
if (Test-Path $source) {
    New-Item -ItemType Directory -Force $kernels | Out-Null
    Copy-Item (Join-Path $source 'kernel_sm_*.cubin.zst'), (Join-Path $source 'kernel_compute_*.ptx.zst'), (Join-Path $source 'kernel_optix*.ptx.zst') $kernels
}
Write-Host "Built: $bin\tequila_cycles.dll"
