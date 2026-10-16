param([string]$Out)
# Package the Claude skill for working in Tequila (skills/tequila) as a ZIP
# to upload to a Claude account: Settings > Capabilities > Skills > Upload.
#
# The ZIP holds the folder `tequila` with SKILL.md at its root, the hand-
# written references, and reference/commands.md - docs/scripting-api.md,
# which is generated from the command catalog and tested against the build,
# so the skill's list of commands is always this version's.

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
if (-not $Out) { $Out = Join-Path $projectRoot 'dist\Tequila-Claude-Skill.zip' }

$staging = Join-Path ([System.IO.Path]::GetTempPath()) "tequila-skill-$PID"
if (Test-Path -LiteralPath $staging) { Remove-Item -LiteralPath $staging -Recurse -Force }
$skill = Join-Path $staging 'tequila'
New-Item -ItemType Directory -Force -Path (Join-Path $skill 'reference') | Out-Null
Copy-Item -LiteralPath (Join-Path $projectRoot 'skills\tequila\SKILL.md') -Destination $skill
Get-ChildItem -LiteralPath (Join-Path $projectRoot 'skills\tequila\reference') -File | ForEach-Object {
    Copy-Item -LiteralPath $_.FullName -Destination (Join-Path $skill 'reference')
}

$preface = @'
# Every Tequila command and operation

Generated from Tequila's own command catalog. With the `tequila_*` tools:

- a command `{"command": "<name>", ...}` is sent as `tequila_apply {"command": {...}}`, or inside `tequila_batch` as `{"op": "apply", "command": {...}}`;
- an operation `{"op": "<name>", ...}` (library, texture_set, reimport, export...) goes through `tequila_request {"request": {...}}`;
- queries are `tequila_query {"what": ...}`.

Ids and keys in the examples are placeholders: read the real ones from the scene first. The Python lines (`fs.commands...`) are for Tequila's Python module and can be ignored here.

---

'@
$reference = Get-Content -LiteralPath (Join-Path $projectRoot 'docs\scripting-api.md') -Raw -Encoding UTF8
[System.IO.File]::WriteAllText((Join-Path $skill 'reference\commands.md'), $preface + $reference, (New-Object System.Text.UTF8Encoding $false))

New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Out) | Out-Null
if (Test-Path -LiteralPath $Out) { Remove-Item -LiteralPath $Out -Force }
# Entries named with forward slashes, as the ZIP format has them: Windows
# PowerShell's Compress-Archive writes `tequila\SKILL.md`, which other
# systems read as one file with a backslash in its name, not a folder.
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [System.IO.Compression.ZipFile]::Open($Out, [System.IO.Compression.ZipArchiveMode]::Create)
try {
    Get-ChildItem -LiteralPath $skill -Recurse -File | ForEach-Object {
        $name = 'tequila/' + $_.FullName.Substring($skill.Length + 1).Replace('\', '/')
        [System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, $_.FullName, $name) | Out-Null
    }
} finally {
    $zip.Dispose()
}
Remove-Item -LiteralPath $staging -Recurse -Force
Write-Output "Skill: $Out"
