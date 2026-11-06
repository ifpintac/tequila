param([Parameter(Mandatory = $true)][string]$Package, [string]$Out)
# Tequila-Portable-<version>-windows.zip: a package made by
# package-windows.ps1, with portable.txt added. That file beside Tequila.exe
# makes the copy keep everything it remembers - settings, library, plugins,
# caches, crash reports, the journal - in a data folder beside it
# (tequila_format::library::portable_directory), not in the computer's user
# folders: for a USB drive, or a PC where nothing may be installed.
#
# The files are at the zip's root, so extracting it over a portable copy's
# folder updates it, and its data folder, which is not in the zip, stays.
# The marker goes into the zip only, never into the package's folder: a
# package in dist/ is what the launcher opens on this PC, with this PC's
# settings, and the installers are made from that folder.
#
# Written entry by entry with .NET's zip, streamed: Compress-Archive in
# Windows PowerShell reads each file whole, and fails on large ones.
#
# -Out names the zip or the folder to put it in; by default it goes beside
# the package.

$ErrorActionPreference = 'Stop'
$source = (Get-Item -LiteralPath $Package).FullName.TrimEnd('\')
if (-not (Test-Path -LiteralPath (Join-Path $source 'Tequila.exe'))) { throw "$source is not a Tequila package." }

# The version the package says it is (BUILD.txt), else when it was built:
# Tequila-Windows-20261021-093000 is 2026.10.21.0930.
$version = Get-Content -LiteralPath (Join-Path $source 'BUILD.txt') -ErrorAction SilentlyContinue |
    Where-Object { $_ -like 'Version: *' } | Select-Object -First 1 | ForEach-Object { $_.Substring(9).Trim() }
if (-not $version) {
    if ((Split-Path -Leaf $source) -notmatch '(\d{4})(\d{2})(\d{2})-(\d{2})(\d{2})') { throw "Cannot tell which version $source is." }
    $version = '{0}.{1}.{2}.{3}{4}' -f $Matches[1], $Matches[2], $Matches[3], $Matches[4], $Matches[5]
}
$name = "Tequila-Portable-$version-windows.zip"
if (-not $Out) {
    $Out = Join-Path (Split-Path -Parent $source) $name
} elseif (Test-Path -LiteralPath $Out -PathType Container) {
    $Out = Join-Path (Get-Item -LiteralPath $Out).FullName $name
}
# .NET reads a relative path from the process's folder, not PowerShell's.
$Out = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Out)

Add-Type -AssemblyName System.IO.Compression
$partial = "$Out.partial"
Remove-Item -LiteralPath $partial -ErrorAction SilentlyContinue
# The zip format cannot date anything before 1980; such a file is dated now.
$epoch = Get-Date -Year 1980 -Month 1 -Day 2
$level = [IO.Compression.CompressionLevel]::Optimal
$stream = [IO.File]::Open($partial, [IO.FileMode]::CreateNew)
$zip = New-Object IO.Compression.ZipArchive($stream, [IO.Compression.ZipArchiveMode]::Create)
$count = 0
try {
    foreach ($item in Get-ChildItem -LiteralPath $source -Recurse -Force) {
        $entryName = $item.FullName.Substring($source.Length + 1).Replace('\', '/')
        # Never a stray marker or data folder of the package's own: the zip's
        # marker is written below, and a data folder is someone's things.
        if ($entryName -eq 'portable.txt' -or $entryName -eq 'data' -or $entryName -like 'data/*') { continue }
        if ($item.PSIsContainer) {
            # A folder with nothing in it is kept as itself.
            if (-not (Get-ChildItem -LiteralPath $item.FullName -Force | Select-Object -First 1)) {
                [void]$zip.CreateEntry("$entryName/")
            }
            continue
        }
        $entry = $zip.CreateEntry($entryName, $level)
        $entry.LastWriteTime = if ($item.LastWriteTime -lt $epoch) { Get-Date } else { $item.LastWriteTime }
        $from = [IO.File]::OpenRead($item.FullName)
        $to = $entry.Open()
        try { $from.CopyTo($to) } finally { $to.Dispose(); $from.Dispose() }
        $count++
    }
    $marker = $zip.CreateEntry('portable.txt', $level)
    $writer = New-Object IO.StreamWriter($marker.Open(), (New-Object Text.UTF8Encoding($false)))
    try {
        $writer.Write("This file makes this copy of Tequila portable: it keeps your settings, library, plugins, caches and everything else it remembers in the data folder beside it, rather than in this computer's user folders. Delete this file to have Tequila use the computer's usual folders again.`r`n")
    } finally { $writer.Dispose() }
} finally {
    $zip.Dispose()
    $stream.Dispose()
}
Move-Item -LiteralPath $partial -Destination $Out -Force
Write-Output "Portable: $Out ($([math]::Round((Get-Item -LiteralPath $Out).Length / 1MB)) MB, $count files and portable.txt, from $(Split-Path -Leaf $source))"
