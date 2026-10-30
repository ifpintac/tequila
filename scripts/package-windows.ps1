param([switch]$SkipBuild)
# Packages build in a folder of their own. A development build is often
# running straight out of target/release, and Windows will not let a build
# replace an executable that is open, so sharing it made packaging fail
# whenever Tequila was open.

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location $projectRoot
try {
    $cargoCommand = Get-Command cargo -ErrorAction SilentlyContinue
    $cargoPath = if ($cargoCommand) { $cargoCommand.Source } else {
        Join-Path $env:USERPROFILE '.cargo/bin/cargo.exe'
    }
    if (-not $SkipBuild) {
        & $cargoPath build --release --offline --target-dir target-package -p tequila-app -p tequila-cli
        if ($LASTEXITCODE -ne 0) { throw 'Release build failed; no package was created.' }
    }
    $releasePath = Join-Path $projectRoot 'target-package/release'
    foreach ($required in @('tequila-app.exe', 'tequila.exe', 'embree4.dll',
        'OpenImageDenoise.dll', 'OpenImageDenoise_core.dll', 'OpenImageDenoise_device_cpu.dll', 'tbb12.dll')) {
        if (-not (Test-Path -LiteralPath (Join-Path $releasePath $required))) {
            throw "Missing runtime file: $required"
        }
    }
    # A new folder for each build also permits packaging while an older copy is open.
    # Named for when it was built, which is its version; or for the release
    # it is built for (TEQUILA_PACKAGE_STAMP, 20261021-093000), as the
    # release workflow builds it.
    $stamp = if ($env:TEQUILA_PACKAGE_STAMP) { $env:TEQUILA_PACKAGE_STAMP } else { Get-Date -Format 'yyyyMMdd-HHmmss' }
    $packageName = 'Tequila-Windows-' + $stamp
    $packagePath = Join-Path $projectRoot "dist/$packageName"
    New-Item -ItemType Directory -Path $packagePath | Out-Null
    Copy-Item -LiteralPath (Join-Path $releasePath 'tequila-app.exe') -Destination (Join-Path $packagePath 'Tequila.exe')
    Copy-Item -LiteralPath (Join-Path $releasePath 'tequila.exe') -Destination (Join-Path $packagePath 'tequila-cli.exe')
    # The program's symbols, beside the package and not in it: a crash
    # report gives Windows' frames as Tequila.exe+offset, and these turn
    # them back into names. Too large to ship (tens of megabytes).
    $symbolsPath = Join-Path $projectRoot "dist/$packageName-symbols"
    $appSymbols = Join-Path $releasePath 'tequila_app.pdb'
    if (Test-Path -LiteralPath $appSymbols) {
        New-Item -ItemType Directory -Path $symbolsPath | Out-Null
        Copy-Item -LiteralPath $appSymbols -Destination $symbolsPath
    }
    Get-ChildItem -LiteralPath $releasePath -Filter '*.dll' | ForEach-Object {
        Copy-Item -LiteralPath $_.FullName -Destination $packagePath
    }
    # Microsoft's C++ runtime, beside the executable ("app-local"). Tequila,
    # Embree, TBB, Open Image Denoise and Cycles all link against it, and a
    # PC without the Visual C++ Redistributable - a fresh or locked-down work
    # machine - stopped at "VCRUNTIME140.dll was not found". Visual Studio's
    # licence lists these files as redistributable; the newest installed set
    # is the one the build used.
    $vsRoots = @("${env:ProgramFiles(x86)}\Microsoft Visual Studio", "$env:ProgramFiles\Microsoft Visual Studio")
    $crt = $vsRoots | Where-Object { Test-Path -LiteralPath $_ } |
        ForEach-Object { Get-ChildItem -Path (Join-Path $_ '*\*\VC\Redist\MSVC\*\x64\Microsoft.VC14*.CRT') -Directory -ErrorAction SilentlyContinue } |
        Where-Object { $_.Parent.Parent.Name -match '^\d+(\.\d+)+$' } |
        Sort-Object { [version]$_.Parent.Parent.Name } |
        Select-Object -Last 1
    if ($crt) {
        Get-ChildItem -LiteralPath $crt.FullName -Filter '*.dll' | ForEach-Object {
            Copy-Item -LiteralPath $_.FullName -Destination $packagePath
        }
        Write-Host "C++ runtime included from $($crt.FullName)"
    } else {
        Write-Warning 'No Visual C++ runtime found to include: the package needs the Visual C++ Redistributable on the PC it runs on.'
    }
    # Cycles, when it has been built (native/cycles/build-cycles.ps1): its
    # DLL, the libraries it brings, and the GPU kernels, in a folder of their
    # own that Tequila looks in. Embree, TBB and Open Image Denoise are not
    # copied again: Cycles uses the ones above.
    $cyclesWork = if ($env:TEQUILA_CYCLES_WORK) { $env:TEQUILA_CYCLES_WORK } else {
        Join-Path (Split-Path -Parent $projectRoot) 'cycles-spike'
    }
    $cyclesBin = Join-Path $cyclesWork 'cycles/build/bin'
    if (Test-Path -LiteralPath (Join-Path $cyclesBin 'tequila_cycles.dll')) {
        $cyclesPath = Join-Path $packagePath 'cycles'
        New-Item -ItemType Directory -Path $cyclesPath | Out-Null
        Get-ChildItem -LiteralPath $cyclesBin -Filter '*.dll' |
            Where-Object { $_.Name -match '^(tequila_cycles|OpenImageIO|OpenImageIO_Util|OpenColorIO_2_\d+|Iex|IlmThread|OpenEXR|OpenEXRCore|OpenEXRUtil|imath)\.dll$' } |
            ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination $cyclesPath }
        $kernels = Join-Path $cyclesWork 'kernels/lib'
        if (Test-Path -LiteralPath $kernels) {
            Copy-Item -LiteralPath $kernels -Destination (Join-Path $cyclesPath 'lib') -Recurse
        }
        Write-Host "Cycles included from $cyclesBin"
    } else {
        Write-Host 'Cycles not built; packaging without it'
    }
    # OpenCASCADE's STEP reader, when it has been built (native/occt/
    # build-occt.bat): a program of its own with its libraries, in a folder
    # Tequila looks in. It is a separate process, so Windows looks for the
    # C++ runtime beside it, not beside Tequila: it gets its own copy.
    $occtBuilt = Join-Path $projectRoot 'native/occt/build/occt'
    if (Test-Path -LiteralPath (Join-Path $occtBuilt 'tequila-occt.exe')) {
        $occtPath = Join-Path $packagePath 'occt'
        Copy-Item -LiteralPath $occtBuilt -Destination $occtPath -Recurse
        if ($crt) {
            Get-ChildItem -LiteralPath $crt.FullName -Filter '*.dll' | ForEach-Object {
                Copy-Item -LiteralPath $_.FullName -Destination $occtPath
            }
        }
        Write-Host "OpenCASCADE STEP reader included from $occtBuilt"
    } else {
        Write-Host 'OpenCASCADE reader not built; STEP files are read by truck alone'
    }
    $assetsPath = Join-Path $packagePath 'assets'
    $pythonPath = Join-Path $packagePath 'python'
    New-Item -ItemType Directory -Path $pythonPath | Out-Null
    Get-ChildItem -LiteralPath (Join-Path $projectRoot 'python') -Filter '*.py' -File | ForEach-Object {
        Copy-Item -LiteralPath $_.FullName -Destination $pythonPath
    }
    Copy-Item -LiteralPath (Join-Path $projectRoot 'docs/SCRIPTING.md') -Destination $packagePath
    Copy-Item -LiteralPath (Join-Path $projectRoot 'docs/plugins.md') -Destination (Join-Path $packagePath 'PLUGINS.md')
    Copy-Item -LiteralPath (Join-Path $projectRoot 'docs/materials-authoring.md') -Destination (Join-Path $packagePath 'MATERIALS.md')
    # The Claude skill for working in Tequila, to upload to a Claude account
    # (Connect an AI says so).
    & (Join-Path $PSScriptRoot 'package-skill.ps1') -Out (Join-Path $packagePath 'Tequila-Claude-Skill.zip') | Out-Null
    # The example plugins: read-only, and off until turned on (the .bundled
    # file marks the folder as Tequila's own).
    Copy-Item -LiteralPath (Join-Path $projectRoot 'plugins/examples') -Destination (Join-Path $packagePath 'plugins') -Recurse
    New-Item -ItemType Directory -Path $assetsPath | Out-Null
    Copy-Item -LiteralPath (Join-Path $projectRoot 'assets/library') -Destination $assetsPath -Recurse
    # The built-in studio presets, found beside the library.
    Copy-Item -LiteralPath (Join-Path $projectRoot 'assets/studios') -Destination $assetsPath -Recurse
    # The sample scenes the start screen opens; the folder's model files are
    # the tests' fixtures.
    $samplesPath = Join-Path $assetsPath 'samples'
    New-Item -ItemType Directory -Path $samplesPath | Out-Null
    Copy-Item -Path (Join-Path $projectRoot 'assets/samples/*.tequila') -Destination $samplesPath
    foreach ($name in @('LICENSE', 'NOTICE', 'THIRD_PARTY_LICENSES.md', 'TRADEMARKS.md')) {
        Copy-Item -LiteralPath (Join-Path $projectRoot $name) -Destination $packagePath
    }
    $licensePath = Join-Path $packagePath 'licenses'
    New-Item -ItemType Directory -Path $licensePath | Out-Null
    # Include the SDK redistribution notices, including their bundled dependencies.
    Get-ChildItem -LiteralPath (Join-Path $projectRoot 'third_party') -Directory | ForEach-Object {
        $sdkPath = Join-Path $licensePath $_.Name
        foreach ($folder in @('doc', 'share')) {
            $sourcePath = Join-Path $_.FullName $folder
            if (Test-Path -LiteralPath $sourcePath) {
                New-Item -ItemType Directory -Path $sdkPath -Force | Out-Null
                Copy-Item -LiteralPath $sourcePath -Destination $sdkPath -Recurse
            }
        }
    }
    $metadataText = & $cargoPath metadata --format-version 1 --offline --filter-platform x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw 'Could not gather dependency notices.' }
    $metadata = $metadataText | ConvertFrom-Json
    $notices = foreach ($dependency in $metadata.packages) {
        # Registry crates, and crates vendored in `vendor/` with a fix: both
        # are someone else's code in the binary, and both owe a notice.
        $vendored = $dependency.manifest_path -match '[\\/]vendor[\\/]'
        if ($dependency.source -or $vendored) {
            $dependencyRoot = Split-Path -Parent $dependency.manifest_path
            $dependencyNotices = Get-ChildItem -LiteralPath $dependencyRoot -File |
                Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE|COPYRIGHT)' }
            if ($dependencyNotices) {
                $destination = Join-Path $licensePath "$($dependency.name)-$($dependency.version)"
                New-Item -ItemType Directory -Path $destination | Out-Null
                $dependencyNotices | ForEach-Object {
                    Copy-Item -LiteralPath $_.FullName -Destination $destination
                }
            }
            "$($dependency.name) $($dependency.version): $($dependency.license)"
        }
    }
    $notices | Set-Content -LiteralPath (Join-Path $licensePath 'rust-dependencies.txt') -Encoding utf8
    # Notices a crate compiles in but does not ship (blosc-src builds c-blosc
    # from source and includes none of its licence files), kept in the repo.
    Get-ChildItem -LiteralPath (Join-Path $projectRoot 'scripts/notices') -Directory |
        ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination $licensePath -Recurse }
    @'
TEQUILA FOR WINDOWS

Double-click Tequila.exe. No terminal or Rust installation is needed.
Keep the executable, DLLs, assets and licenses together in this folder.
To move the app, copy the entire folder. Extract the ZIP before launching.

Import: Ctrl+I (STEP, IGES, glTF, GLB, OBJ, STL, USD, IFC)
New / open / save project: Ctrl+N / Ctrl+O / Ctrl+S (File > Open recent)
Pause / resume viewport: P
Frame selection: F; frame everything: Shift+F
Orbit: left drag; pan: middle drag; zoom: scroll
Materials: drag one from the library onto a part. Dropped on any selected
part it goes on the whole selection; double-click a part to select its
group (a body that came in pieces), right-click a library material for
Apply to selection.
Use the Fast view for lightweight scene navigation.
Faster final renders: Render settings > Engine > Cycles (graphics card).
The viewport pauses while minimized or exporting and resumes automatically.
Closing prompts you to save unsaved work and warns about an active export.

AI apps: Assistant > Connect an AI... > Connect the Claude app lets Claude
work in Tequila - materials, procedural materials from a description,
lighting, renders, even plugins. Tequila-Claude-Skill.zip teaches it how
(upload it in Claude's Settings > Capabilities > Skills).
Plugins: Plugins > Manage plugins... (PLUGINS.md says how to write one).
Guides: SCRIPTING.md, PLUGINS.md, MATERIALS.md, also under Help > Guides.
If Tequila closes unexpectedly, the next launch shows what happened; Copy
for Discord puts it on the clipboard for #bug-reports. Help > Crash reports.

tequila-cli.exe is the optional command-line renderer; with --worker it is
a render node for network rendering (Render window > Network rendering).
python\ holds the scripting module: tequila.open() needs no open window.
'@ | Set-Content -LiteralPath (Join-Path $packagePath 'START-HERE.txt') -Encoding utf8
    $revision = & git rev-parse HEAD
    # The release's version, as its installer and tag are named: the
    # package's stamp, 20261021-093000, as 2026.10.21.0930. Help > Check for
    # updates compares it with the latest release's.
    $version = '{0}.{1}.{2}.{3}' -f $stamp.Substring(0, 4), $stamp.Substring(4, 2), $stamp.Substring(6, 2), $stamp.Substring(9, 4)
    "Source: $revision`nVersion: $version`nBuilt: $(Get-Date -Format o)`nIncludes current working-tree changes." |
        Set-Content -LiteralPath (Join-Path $packagePath 'BUILD.txt') -Encoding utf8
    $archivePath = "$packagePath.zip"
    # The zip format cannot date anything before 1980, and some crates ship
    # their licence files stamped 1970 or 1973; copied, they keep the stamp.
    # Date them to the build instead.
    $zipEpoch = Get-Date -Year 1980 -Month 1 -Day 2
    Get-ChildItem -LiteralPath $packagePath -Recurse -File |
        Where-Object { $_.LastWriteTime -lt $zipEpoch } |
        ForEach-Object { $_.LastWriteTime = Get-Date }
    # The zip is a convenience for moving the build to another machine; the
    # folder is the package. Should it still fail, that must not cost the
    # shortcuts below, so a failed zip is reported and passed over.
    try {
        Compress-Archive -LiteralPath $packagePath -DestinationPath $archivePath -CompressionLevel Optimal
    } catch {
        Write-Warning "No zip was made: $($_.Exception.Message)"
        Remove-Item -LiteralPath $archivePath -ErrorAction SilentlyContinue
        $archivePath = '(none)'
    }
    $shortcutShell = New-Object -ComObject WScript.Shell
    # Both shortcuts open the launcher, which opens the newest package and
    # rebuilds it first when the checkout has moved on. One aimed at this
    # package went stale with the next commit: it kept opening an old
    # Tequila after the code had moved on. The icon is the launcher's own
    # where it is installed, else this build's.
    $launcherIcon = Join-Path $env:LOCALAPPDATA 'Tequila\launcher.ico'
    $icon = if (Test-Path -LiteralPath $launcherIcon) { $launcherIcon } else { Join-Path $packagePath 'Tequila.exe' }
    foreach ($name in @('Tequila.lnk', 'Tequila (newest).lnk')) {
        $shortcut = $shortcutShell.CreateShortcut((Join-Path $projectRoot $name))
        $shortcut.TargetPath = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
        $shortcut.Arguments = '-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File "' + (Join-Path $PSScriptRoot 'launch-newest.ps1') + '"'
        $shortcut.WorkingDirectory = $projectRoot
        $shortcut.IconLocation = "$icon,0"
        $shortcut.WindowStyle = 7
        $shortcut.Description = 'Open the newest Tequila'
        $shortcut.Save()
    }
    Write-Output "Executable: $(Join-Path $packagePath 'Tequila.exe')"
    Write-Output "Archive: $archivePath"
    Write-Output "Shortcut: $(Join-Path $projectRoot 'Tequila.lnk')"
} finally {
    Pop-Location
}
