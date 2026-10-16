param([string]$Package, [switch]$Rebuild, [string]$PatchFrom)
# Build Tequila-Setup-<version>.exe, an installer for another Windows PC,
# from a package made by package-windows.ps1 (scripts/installer/tequila.iss).
#
# With -PatchFrom <the package an installed version was built from>, build
# Tequila-Update-<that version>-to-<this one>.exe instead: only the files
# that changed between the two, for a PC that has that version installed
# (scripts/installer/tequila-patch.iss). It refuses any other version.
#
# The package is the newest in dist/, made again first when it was built from
# another commit than the checkout's (or with -Rebuild); -Package names one.
# Needs Inno Setup 6 or later (jrsoftware.org), found on the PATH or where it
# installs, for this user or for everyone.

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$dist = Join-Path $projectRoot 'dist'

$iscc = (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source
if (-not $iscc) {
    $iscc = @(
        "$env:LOCALAPPDATA\Programs\Inno Setup 7\ISCC.exe",
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 7\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 7\ISCC.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
    ) | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
}
if (-not $iscc) { throw 'Inno Setup was not found. Install it from jrsoftware.org, then run this again.' }

function Get-Newest {
    Get-ChildItem -LiteralPath $dist -Directory -Filter 'Tequila-Windows-*' -ErrorAction SilentlyContinue |
        Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'Tequila.exe') } |
        Sort-Object Name | Select-Object -Last 1
}

if ($Package) {
    $source = Get-Item -LiteralPath $Package
} else {
    $source = Get-Newest
    $head = (& git -C $projectRoot rev-parse HEAD).Trim()
    $built = if ($source) {
        (Get-Content -LiteralPath (Join-Path $source.FullName 'BUILD.txt') -ErrorAction SilentlyContinue |
            Where-Object { $_ -like 'Source: *' } | Select-Object -First 1) -replace '^Source: ', ''
    }
    if ($Rebuild -or -not $source -or $built.Trim() -ne $head) {
        Write-Host 'Packaging the current checkout first...'
        & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'package-windows.ps1')
        if ($LASTEXITCODE -ne 0) { throw 'Packaging failed; no installer was made.' }
        $source = Get-Newest
    }
}
if (-not (Test-Path -LiteralPath (Join-Path $source.FullName 'Tequila.exe'))) {
    throw "$($source.FullName) is not a Tequila package."
}

# The version is when the package was built: Tequila-Windows-20261013-0951xx
# becomes 2026.10.13.0951, the file's version 2026.10.13.951.
if ($source.Name -notmatch '(\d{4})(\d{2})(\d{2})-(\d{2})(\d{2})') { throw "Cannot tell when $($source.Name) was built." }
$version = '{0}.{1}.{2}.{3}{4}' -f $Matches[1], $Matches[2], $Matches[3], $Matches[4], $Matches[5]
$numeric = '{0}.{1}.{2}.{3}' -f [int]$Matches[1], [int]$Matches[2], [int]$Matches[3], [int]($Matches[4] + $Matches[5])

if ($PatchFrom) {
    $base = Get-Item -LiteralPath $PatchFrom
    if ($base.Name -notmatch '(\d{4})(\d{2})(\d{2})-(\d{2})(\d{2})') { throw "Cannot tell when $($base.Name) was built." }
    $baseVersion = '{0}.{1}.{2}.{3}{4}' -f $Matches[1], $Matches[2], $Matches[3], $Matches[4], $Matches[5]
    if ($baseVersion -eq $version) { throw 'The base and the new package are the same version.' }
    # Every file by its checksum, as the installed copy has it: the
    # portable read-me is not installed.
    $files = {
        param($root)
        Get-ChildItem -LiteralPath $root -Recurse -File | Where-Object { $_.Name -ne 'START-HERE.txt' } | ForEach-Object {
            [pscustomobject]@{ Path = $_.FullName.Substring($root.Length + 1); Hash = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash }
        }
    }
    $old = @{}
    & $files $base.FullName | ForEach-Object { $old[$_.Path] = $_.Hash }
    $new = @(& $files $source.FullName)
    $changed = @($new | Where-Object { $old[$_.Path] -ne $_.Hash })
    $removed = @($old.Keys | Where-Object { -not (Test-Path -LiteralPath (Join-Path $source.FullName $_)) })
    if ($changed.Count -eq 0 -and $removed.Count -eq 0) { throw 'Nothing changed between the two packages.' }
    $staging = Join-Path $dist "update-$baseVersion-to-$version"
    if (Test-Path -LiteralPath $staging) { Remove-Item -LiteralPath $staging -Recurse -Force }
    foreach ($file in $changed) {
        $target = Join-Path $staging $file.Path
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $target) | Out-Null
        Copy-Item -LiteralPath (Join-Path $source.FullName $file.Path) -Destination $target
    }
    $removedList = "$staging-removed.iss"
    # Written even when nothing was removed: piped nothing, Set-Content makes
    # no file, and the update's #include of it stopped the build.
    $lines = @('; Files the new version no longer has.') + @($removed | ForEach-Object { 'Type: files; Name: "{app}\' + $_ + '"' })
    Set-Content -LiteralPath $removedList -Value $lines -Encoding utf8
    & $iscc /Q "/DSourceDir=$staging" "/DRemovedList=$removedList" "/DAppVersion=$version" "/DNumericVersion=$numeric" "/DBaseVersion=$baseVersion" "/DOutputDir=$dist" (Join-Path $PSScriptRoot 'installer\tequila-patch.iss')
    if ($LASTEXITCODE -ne 0) { throw 'Inno Setup could not build the update.' }
    $update = Join-Path $dist "Tequila-Update-$baseVersion-to-$version.exe"
    Write-Output "Update: $update ($([math]::Round((Get-Item -LiteralPath $update).Length / 1MB, 1)) MB: $($changed.Count) files changed, $($removed.Count) removed, from $($base.Name) to $($source.Name))"
    return
}

& $iscc /Q "/DSourceDir=$($source.FullName)" "/DAppVersion=$version" "/DNumericVersion=$numeric" "/DOutputDir=$dist" (Join-Path $PSScriptRoot 'installer\tequila.iss')
if ($LASTEXITCODE -ne 0) { throw 'Inno Setup could not build the installer.' }
$installer = Join-Path $dist "Tequila-Setup-$version.exe"
Write-Output "Installer: $installer ($([math]::Round((Get-Item -LiteralPath $installer).Length / 1MB)) MB, from $($source.Name))"
