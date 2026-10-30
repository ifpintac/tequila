# The Assistant: AI inside Tequila

Click **Assistant** on the toolbar and describe what you want, for example
*"make the strap black velvet, the lenses dark glass, and light it like a
studio product shot"*. The AI you have connected does it in the open scene:
- It finds the parts.
- It changes materials, lighting and camera.
- It switches the viewport to rendered and takes a screenshot to check its
  own work, then adjusts.

Each step is listed in the panel as it happens. Each step also shows in the
console (in amber, with the AI's name) and can be undone. **Undo request**
takes back everything one request changed, as long as you haven't edited by
hand since. **Stop** ends a request part-way.

The assistant works on the scene. It cannot import files, save anywhere but
the project's own file, or quit. It saves or renders only when you ask it to.

It can also make **materials of your own** from a description (*"blue
terrazzo with brass flecks"*), showing you each try on a ball, and write
**plugins** when you ask for one (*"a panel that renames the selected parts
with a prefix"*). A plugin runs Python on your computer, so Tequila asks you
before it starts - and asks again whenever the assistant changes the code of
one that is on.

## Connecting an AI

Click **Providers…**, choose a starting point, fill in the form, and click
**Test**:

| Provider | Type | Address | Key |
|---|---|---|---|
| **OpenAI** | OpenAI-compatible | `https://api.openai.com/v1` | An OpenAI API key (platform.openai.com) |
| **Claude** | Anthropic | `https://api.anthropic.com` | An Anthropic API key (console.anthropic.com) |
| **Google Gemini** | OpenAI-compatible | `https://generativelanguage.googleapis.com/v1beta/openai` | A Gemini API key (aistudio.google.com) |
| **OpenRouter**, hundreds of models behind one key | OpenAI-compatible | `https://openrouter.ai/api/v1` | An OpenRouter key (openrouter.ai) |
| **Mistral** | OpenAI-compatible | `https://api.mistral.ai/v1` | A Mistral key (console.mistral.ai) |
| **DeepSeek**, text only | OpenAI-compatible | `https://api.deepseek.com/v1` | A DeepSeek key (platform.deepseek.com) |
| **xAI (Grok)** | OpenAI-compatible | `https://api.x.ai/v1` | An xAI key (console.x.ai) |
| **Groq** | OpenAI-compatible | `https://api.groq.com/openai/v1` | A Groq key (console.groq.com) |
| **Ollama**, free, on this computer | OpenAI-compatible | `http://localhost:11434/v1` | None |
| **LM Studio**, free, on this computer | OpenAI-compatible | `http://localhost:1234/v1` | None |
| **Your company's AI** (e.g. Uliverse) | usually OpenAI-compatible | from your IT/AI team | from your IT/AI team |

A model that cannot look at images still works, without checking its own
screenshots: untick **The model can see images** for it. DeepSeek's and
Groq's presets start that way.

**Fetch models** asks the service which models it has; pick one from the
list. Several providers can be connected at once. The drop-down at the top of
the panel switches between them, even in the middle of a conversation.

An API key is not the same as a ChatGPT, Claude or Gemini subscription. The
APIs are billed separately, per use, by each company.

### Only have a key?

Paste it. Where the key's own start says which service it belongs to,
Tequila fills in the rest: `sk-ant-` is Anthropic, `sk-proj-` OpenAI,
`sk-or-` OpenRouter, `AIza` Google Gemini, `gsk_` Groq, `xai-` xAI. Then click
**Fetch models** and pick one. Only the start is looked at, and nothing is
sent anywhere until you click Fetch models or Test.

A key that starts with just `sk-` could be OpenAI's or one from a company
gateway (they often look the same), and Tequila will not send a company key
to OpenAI to find out: click **It's an OpenAI key** if it is. Any other key
does not say where it belongs, and the service's address has to come from
whoever gave you the key.

### Uliverse or another company AI

Most company AI platforms offer an **OpenAI-compatible** endpoint; some are
Anthropic-compatible. Ask the team that runs it for:
1. the API address (for OpenAI-compatible services it usually ends in `/v1`);
2. a model name;
3. an API key;
4. if the key goes in a header other than `Authorization: Bearer`, the
   header's name (for example Azure-style `api-key`). Put it under
   **Advanced → Key header**.

If the model cannot look at images, untick **The model can see images**. It
still works, but it cannot check its results visually. If Uliverse has its
own API that is neither format, Tequila needs a small adapter
(`crates/tequila-ai/src/wire.rs`). Its API documentation is enough to write
one.

### Where keys are kept

In `~/.tequila/assistant.json`, encrypted with Windows' Data Protection API
for your Windows account. The file is useless on another computer or to
another user. A key is sent only to the address of the provider it belongs
to, and only over `https://`; plain `http://` is allowed only for a server on
this computer. You can also keep a key in an environment variable and give
Tequila just the variable's name.

## ChatGPT (the app on the web)

ChatGPT's custom apps reach MCP servers over the internet. OpenAI's **Secure
MCP Tunnel** connects ChatGPT to Tequila's MCP bridge without opening
anything on your computer to the internet: its `tunnel-client` makes only
outgoing connections to OpenAI. `scripts/chatgpt-tunnel.ps1` sets it up, with
the one-time account steps in its header:
1. Create a tunnel and a runtime key at
   platform.openai.com/settings/organization/tunnels.
2. Download `tunnel-client` from github.com/openai/tunnel-client/releases.
3. In ChatGPT: Settings → Apps & Connectors → Developer mode. Create an app
   with **Tunnel** as the connection.

Then, with Tequila open:

```powershell
$env:CONTROL_PLANE_API_KEY = "<the tunnel's runtime key>"
.\scripts\chatgpt-tunnel.ps1 -TunnelId tunnel_... -TunnelClient C:\path\to\tunnel-client.exe
```

Over the tunnel the bridge runs `--restricted`: scene work only, with no
importing files, no raw requests, and no saving or rendering to paths the
caller chooses. Developer mode is available to ChatGPT Plus, Pro, Business,
Enterprise and Education accounts on the web. The tunnel needs Tunnels
permissions on your OpenAI platform organization.

## AI apps on this computer

The Claude app, Cursor, VS Code with GitHub Copilot, OpenAI's Codex, Gemini
CLI, Windsurf, LM Studio and Claude Code start Tequila's MCP bridge
(`tequila-cli --mcp`) themselves. **Connect an AI…** lists the ones on this
computer, each with a **Connect** button (Claude Code's is a command to
copy), and **Copy the settings** for any other MCP app. Without it, an app
has no Tequila tools and may fall back to clicking around the screen. See
`docs/SCRIPTING.md`, section *Connecting an AI app (MCP)*.
