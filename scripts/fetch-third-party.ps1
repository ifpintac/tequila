# Fetch the prebuilt SDKs Tequila links against.
#
# Embree and Open Image Denoise ship as prebuilt releases rather than as
# crates. Both are Apache-2.0. They land in third_party/, which is gitignored:
# these are build inputs, not project source.
#
# Usage:  pwsh scripts/fetch-third-party.ps1

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$thirdParty = Join-Path $root "third_party"
New-Item -ItemType Directory -Force -Path $thirdParty | Out-Null

$embreeVersion = "4.4.1"
$oidnVersion = "2.5.1"

function Get-Sdk {
    param([string]$Name, [string]$Url, [string]$Expected)

    $target = Join-Path $thirdParty $Expected
    if (Test-Path (Join-Path $target "lib")) {
        Write-Host "$Name already present at $target"
        return
    }

    $archive = Join-Path $thirdParty "$Name.zip"
    Write-Host "Downloading $Name from $Url"
    Invoke-WebRequest -Uri $Url -OutFile $archive -UseBasicParsing
    Write-Host "Extracting $Name"
    Expand-Archive -Path $archive -DestinationPath $thirdParty -Force
    Remove-Item $archive
}

if ($IsWindows -or $null -eq $IsWindows) {
    Get-Sdk -Name "embree" `
        -Url "https://github.com/RenderKit/embree/releases/download/v$embreeVersion/embree-$embreeVersion.x64.windows.zip" `
        -Expected "embree-$embreeVersion.x64.windows"
    Get-Sdk -Name "oidn" `
        -Url "https://github.com/RenderKit/oidn/releases/download/v$oidnVersion/oidn-$oidnVersion.x64.windows.zip" `
        -Expected "oidn-$oidnVersion.x64.windows"
} else {
    throw "Use scripts/fetch-third-party.sh on Linux and macOS."
}

Write-Host ""
Write-Host "Done. Both SDKs are in third_party/ and the build scripts will find"
Write-Host "them there; no environment variables are needed."
