# Open the newest packaged Tequila, bringing it up to date first.
#
# Every package is a folder of its own in dist/, named by when it was built,
# so the newest is the last by name. A shortcut to one package goes stale the
# moment the next is built; this one never does. "Tequila (newest).lnk"
# runs it with no console window.
#
# A package records the commit it was built from (BUILD.txt). When the
# checkout has moved on since, the package is rebuilt before it opens -
# with a small window saying so - so the shortcut always opens the newest
# Tequila, not the newest one somebody remembered to package. A rebuild
# that fails opens the previous package, after saying why; the details are
# in dist\update.log.

$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
$dist = Join-Path $root 'dist'
Add-Type -AssemblyName System.Windows.Forms

function Get-Newest {
    Get-ChildItem -LiteralPath $dist -Directory -Filter 'Tequila-Windows-*' -ErrorAction SilentlyContinue |
        Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'Tequila.exe') } |
        Sort-Object Name |
        Select-Object -Last 1
}

# The commit a package was built from, and the checkout's.
function Get-BuiltFrom($package) {
    $build = Join-Path $package.FullName 'BUILD.txt'
    if (-not (Test-Path -LiteralPath $build)) { return $null }
    $line = Get-Content -LiteralPath $build | Where-Object { $_ -like 'Source: *' } | Select-Object -First 1
    if ($line) { $line.Substring(8).Trim() } else { $null }
}

function Get-Head {
    $git = Get-Command git -ErrorAction SilentlyContinue
    if (-not $git) { return $null }
    $head = & $git.Source -C $root rev-parse HEAD 2>$null
    if ($LASTEXITCODE -eq 0) { "$head".Trim() } else { $null }
}

# Rebuild the package, with a window saying so; true when it worked.
function Update-Package {
    $form = New-Object System.Windows.Forms.Form
    $form.Text = 'Tequila'
    $form.Size = New-Object System.Drawing.Size(420, 150)
    $form.StartPosition = 'CenterScreen'
    $form.FormBorderStyle = 'FixedDialog'
    $form.MaximizeBox = $false
    $form.MinimizeBox = $false
    $form.TopMost = $true
    $label = New-Object System.Windows.Forms.Label
    $label.Text = "Updating Tequila to the newest version.`nThis takes a few minutes after big changes, seconds after small ones."
    $label.Location = New-Object System.Drawing.Point(16, 14)
    $label.Size = New-Object System.Drawing.Size(380, 40)
    $bar = New-Object System.Windows.Forms.ProgressBar
    $bar.Style = 'Marquee'
    $bar.Location = New-Object System.Drawing.Point(16, 64)
    $bar.Size = New-Object System.Drawing.Size(372, 18)
    $form.Controls.AddRange(@($label, $bar))
    $form.Show()

    # A process of its own, its output to files: cargo writes its progress
    # to standard error, which PowerShell would otherwise take for failure.
    $log = Join-Path $dist 'update.log'
    $errors = Join-Path $dist 'update-errors.log'
    New-Item -ItemType Directory -Force -Path $dist | Out-Null
    $build = Start-Process -FilePath powershell.exe `
        -ArgumentList '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$(Join-Path $PSScriptRoot 'package-windows.ps1')`"" `
        -WorkingDirectory $root -RedirectStandardOutput $log -RedirectStandardError $errors -WindowStyle Hidden -PassThru
    # Held now, or Windows PowerShell reports no exit code once it has
    # exited, and a build that worked reads as one that failed.
    $null = $build.Handle
    while (-not $build.HasExited) {
        [System.Windows.Forms.Application]::DoEvents()
        Start-Sleep -Milliseconds 100
    }
    $form.Close()
    return ($build.ExitCode -eq 0)
}

# One update at a time: a second double-click while one runs waits for it
# rather than building alongside it.
$mutex = New-Object System.Threading.Mutex($false, 'Local\TequilaLauncherUpdate')
$null = $mutex.WaitOne()
try {
    $newest = Get-Newest
    $head = Get-Head
    $current = $newest -and (-not $head -or (Get-BuiltFrom $newest) -eq $head)
    if (-not $current) {
        if (Update-Package) {
            $newest = Get-Newest
        } else {
            $message = if ($newest) {
                "Tequila could not be updated, so the previous version opens.`n`nThe details are in`n$(Join-Path $dist 'update.log')"
            } else {
                "Tequila could not be built.`n`nThe details are in`n$(Join-Path $dist 'update.log')"
            }
            [System.Windows.Forms.MessageBox]::Show($message, 'Tequila') | Out-Null
        }
    }
} finally {
    $mutex.ReleaseMutex()
}

if ($newest) {
    Start-Process -FilePath (Join-Path $newest.FullName 'Tequila.exe') -WorkingDirectory $newest.FullName
} elseif ($current) {
    [System.Windows.Forms.MessageBox]::Show(
        "No packaged Tequila was found in`n$dist`n`nBuild one with scripts\package-windows.ps1.",
        'Tequila') | Out-Null
}
