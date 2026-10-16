<#
.SYNOPSIS
    Let ChatGPT drive the Tequila that is open on this computer, through
    OpenAI's Secure MCP Tunnel.

.DESCRIPTION
    ChatGPT's custom apps reach MCP servers over the internet. OpenAI's
    tunnel-client connects outward to OpenAI and runs Tequila's MCP bridge
    locally, so ChatGPT can use it without anything on this computer being
    opened to the internet.

    The bridge runs in --restricted mode: the open scene only, with no
    importing files, no raw requests, and no saving or rendering to paths the
    caller chooses.

    One-time setup, in your OpenAI account (this script cannot do it for you):
      1. https://platform.openai.com/settings/organization/tunnels
         Create a tunnel. Note its id (tunnel_...). Create a runtime API key
         for it; your key needs the Tunnels Read + Use permissions.
      2. Download tunnel-client for Windows from
         https://github.com/openai/tunnel-client/releases/latest
      3. In ChatGPT on the web: Settings > Apps & Connectors > enable Developer
         mode, then create an app, choose Tunnel under Connection, and pick
         your tunnel.

    Then, each time you want ChatGPT connected, with Tequila open:
      $env:CONTROL_PLANE_API_KEY = "<the runtime key>"
      .\scripts\chatgpt-tunnel.ps1 -TunnelId tunnel_... -TunnelClient C:\path\to\tunnel-client.exe

    The key is read from the environment and is never written anywhere by
    this script. Leave the window open while ChatGPT works; close it (or
    Ctrl+C) to disconnect.

.PARAMETER TunnelId
    The tunnel's id from the OpenAI platform, starting tunnel_.

.PARAMETER TunnelClient
    Path to tunnel-client.exe. Defaults to tunnel-client on the PATH.

.PARAMETER Python
    The Python that runs the bridge. Defaults to the one on the PATH.
#>
param(
    [Parameter(Mandatory = $true)][string] $TunnelId,
    [string] $TunnelClient = "tunnel-client",
    [string] $Python = ""
)

$ErrorActionPreference = "Stop"

if ($TunnelId -notmatch '^tunnel_[A-Za-z0-9]+$') {
    throw "The tunnel id should look like tunnel_0123456789abcdef... (from platform.openai.com/settings/organization/tunnels)."
}
if (-not $env:CONTROL_PLANE_API_KEY) {
    throw 'Set the tunnel''s runtime key first:  $env:CONTROL_PLANE_API_KEY = "<the runtime key>"'
}
$client = Get-Command $TunnelClient -ErrorAction SilentlyContinue
if (-not $client) {
    throw "tunnel-client was not found. Download it from https://github.com/openai/tunnel-client/releases/latest and pass -TunnelClient <path>."
}
if (-not $Python) {
    # The real interpreter, not the Microsoft Store stub that `python` can be.
    $Python = (Get-Command python -ErrorAction SilentlyContinue |
        Where-Object { $_.Source -notmatch 'WindowsApps' } |
        Select-Object -First 1).Source
    if (-not $Python) { throw "Python was not found; pass -Python C:\path\to\python.exe" }
}

$bridge = Join-Path $PSScriptRoot "..\python\tequila_mcp.py" | Resolve-Path
$discovery = Join-Path $HOME ".tequila\channel.json"
if (-not (Test-Path $discovery)) {
    Write-Warning "Tequila does not seem to have been started yet (no $discovery). Open it before asking ChatGPT to do anything."
}

$command = "`"$Python`" `"$bridge`" --restricted"
Write-Host "Bridge: $command"

& $client.Source init --sample sample_mcp_stdio_local --profile tequila --tunnel-id $TunnelId --mcp-command $command
if ($LASTEXITCODE -ne 0) { throw "tunnel-client init failed." }

& $client.Source doctor --profile tequila --explain
if ($LASTEXITCODE -ne 0) { throw "tunnel-client doctor found a problem; see above." }

Write-Host ""
Write-Host "Connected. ChatGPT can now use Tequila through your tunnel app." -ForegroundColor Green
Write-Host "Everything it does appears in Tequila's console. Close this window to disconnect."
& $client.Source run --profile tequila
