@echo off
rem Build tequila-occt.exe against OpenCASCADE 8.0.1 and gather it, with the
rem libraries it loads, into native\occt\build\occt - the folder packaging
rem copies beside Tequila.
rem
rem Needs: Visual Studio's C++ build tools (vcvars64.bat), OCCT's release
rem build in third_party\occt-8.0.1 (opencascade-release-no-pch.zip from
rem github.com/Open-Cascade-SAS/OCCT, release V8.0.1) and its third-party
rem libraries in third_party\occt-3rdparty (3rdparty-vc14-64.zip, the same
rem release).
setlocal
set HERE=%~dp0
set ROOT=%HERE%..\..
set OCCT=%ROOT%\third_party\occt-8.0.1
set THIRD=%ROOT%\third_party\occt-3rdparty
set OUT=%HERE%build
if not defined VCVARS set VCVARS=C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC\Auxiliary\Build\vcvars64.bat
call "%VCVARS%" >nul || exit /b 1
if not exist "%OUT%" mkdir "%OUT%"
if not exist "%OUT%\occt" mkdir "%OUT%\occt"

set LIBS=TKernel.lib TKMath.lib TKG2d.lib TKG3d.lib TKGeomBase.lib TKBRep.lib TKTopAlgo.lib TKMesh.lib TKShHealing.lib TKXSBase.lib TKDE.lib TKDESTEP.lib TKXCAF.lib TKLCAF.lib TKCAF.lib TKCDF.lib TKRWMesh.lib TKService.lib TKV3d.lib TKVCAF.lib
cl /nologo /std:c++17 /O2 /EHsc /MD /DNDEBUG /I"%OCCT%\inc" "%HERE%tequila_occt.cpp" /Fo"%OUT%\\" /Fe"%OUT%\occt\tequila-occt.exe" /link /LIBPATH:"%OCCT%\win64\vc14\lib" %LIBS% || exit /b 1

rem OCCT's libraries the helper loads, and theirs.
for %%d in (TKernel TKMath TKG2d TKG3d TKGeomBase TKGeomAlgo TKBRep TKTopAlgo TKPrim TKBO TKHLR TKMesh TKShHealing TKXSBase TKDE TKDESTEP TKXCAF TKLCAF TKCAF TKCDF TKVCAF TKService TKV3d TKRWMesh) do (
  copy /y "%OCCT%\win64\vc14\bin\%%d.dll" "%OUT%\occt\" >nul || exit /b 1
)
for %%d in (tbb-2021.13.0-x64\bin\tbb12.dll jemalloc-vc14-64\bin\jemalloc.dll openvr-1.14.15-64\bin\win64\openvr_api.dll freeimage-3.18.0-x64\bin\FreeImage.dll freetype-2.13.3-x64\bin\freetype.dll ffmpeg-3.3.4-64\bin\avcodec-57.dll ffmpeg-3.3.4-64\bin\avformat-57.dll ffmpeg-3.3.4-64\bin\avutil-55.dll ffmpeg-3.3.4-64\bin\swscale-4.dll) do (
  copy /y "%THIRD%\%%d" "%OUT%\occt\" >nul || exit /b 1
)
rem Their licence texts (THIRD_PARTY_LICENSES.md lists what each covers).
if not exist "%OUT%\occt\licenses" mkdir "%OUT%\occt\licenses"
copy /y "%OCCT%\LICENSE_LGPL_21.txt" "%OUT%\occt\licenses\" >nul || exit /b 1
copy /y "%OCCT%\OCCT_LGPL_EXCEPTION.txt" "%OUT%\occt\licenses\" >nul || exit /b 1
copy /y "%THIRD%\freeimage-3.18.0-x64\license-fi.txt" "%OUT%\occt\licenses\FreeImage-license-fi.txt" >nul || exit /b 1
copy /y "%THIRD%\freetype-2.13.3-x64\LICENSE.TXT" "%OUT%\occt\licenses\FreeType-LICENSE.txt" >nul || exit /b 1
copy /y "%THIRD%\tbb-2021.13.0-x64\LICENSE.txt" "%OUT%\occt\licenses\oneTBB-LICENSE.txt" >nul || exit /b 1
rem The linker's by-products for the helper are not part of it.
del /q "%OUT%\occt\tequila-occt.exp" "%OUT%\occt\tequila-occt.lib" 2>nul
echo built %OUT%\occt\tequila-occt.exe
