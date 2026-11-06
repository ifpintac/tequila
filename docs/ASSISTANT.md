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
hand since. **Stop** ends a request part-way, at once.

While it works, the line under the conversation says what it is doing and
for how long: *Loading the model*, *Reading the request*, *Thinking*,
*Writing*, or *Calling* a tool, with a running count of what it has written
and which step it is on. A model that reasons before it answers shows its
reasoning as it comes, dimmed, under **Thinking**; click the heading to fold
it away. Afterwards it is kept, folded, as *Thought for …*. The reply is
waited for as long as the model keeps producing it. If something fails, the
message says what helps, and the conversation is kept: ask again to carry
on from where it stopped.

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

### Using a model on this computer

Ollama and LM Studio run a model on your own computer: free, private, and
as fast as your graphics card. Three things decide whether it works:

1. **A model that can use tools.** The Assistant works through tools, so a
   model without them can only talk. Ollama's library marks the ones that
   can with *tools*, Qwen 3 among them; in LM Studio, look for tool use. Tequila asks Ollama and says if a model
   can't.
2. **A long enough context.** The Assistant's instructions and tools come
   to about 5,000 tokens before anything is said; the command catalog it
   reads before editing materials is about 16,000 more; and every step
   adds to the conversation. Ollama gives a model 4,096 tokens unless told
   otherwise (more only on a graphics card with 24 GB or more), and it does
   not refuse a longer request: it drops the start of it, the instructions
   and the tools, and the model wanders. So Tequila asks Ollama what the
   model gets, does not send a request that won't fit, and says what to
   set. Set **32k or more**:
   - in the Ollama app: **Settings → Context length**;
   - or set the environment variable `OLLAMA_CONTEXT_LENGTH=32768` and
     restart Ollama.

   In LM Studio, set **Context Length** when you load the model. **Test**,
   in Providers…, checks both for Ollama.
3. **A model that fits in the graphics card's memory.** A model on this
   computer reads the whole request before it writes a word. On the
   graphics card that takes seconds; on the processor, minutes. `ollama ps`
   shows where a loaded model runs (*100% GPU* is what you want), and
   Tequila says when part of it runs on the processor. A smaller model, a
   shorter context length, or `OLLAMA_FLASH_ATTENTION=1` with
   `OLLAMA_KV_CACHE_TYPE=q8_0` (which roughly halves the memory the context
   takes) helps it fit. On a 6 GB card, `qwen3:4b` with a 16k context takes
   5.4 GB, a fifth of it on the processor; with those two settings, 3.9 GB,
   all on the graphics card. With 32k and those settings it takes 5.7 GB,
   about three quarters of it on the graphics card, and writes about three
   tokens a second: too slow for real work. A small model on a small card
   answers questions about the scene well; for editing materials, a
   bigger graphics card or a service does better.

The first request after starting Ollama also loads the model: the panel
says *Loading the model*. A model that cannot see images works without
screenshots, and Tequila says so. To a model on this computer the Assistant
sends only the newest screenshot, where a service gets two.

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

## As built, November 6: models on this computer, and slow replies

A report: Ollama connected, asked to texture something, the panel said it
was thinking for about fifteen minutes, then *"…/v1/chat/completions took
too long to answer"*. Two causes, both fixed.

**One silent wait per step.** Every step was one POST that returned only
when the whole reply was written, under a 300-second limit for everything.
Replies are now streamed in both formats: OpenAI-compatible server-sent
events and Anthropic's event stream (`crates/tequila-ai/src/sse.rs`,
`wire::Assembler`). Words, tool calls arriving in pieces (by `index`, with
`id`, their arguments split anywhere), and reasoning — `reasoning` (Ollama,
LM Studio), `reasoning_content` (vLLM, DeepSeek), `<think>…</think>` written
into the answer, Anthropic `thinking` blocks — are put together as they
come. The limit is liveness, not a total (`http::Limits`): a reply may take
as long as it keeps coming. Until its first byte, 20 minutes for a server
on this computer (it reads the whole request first, and on the processor
that can take minutes) and 5 for a service; then 5 and 3 minutes of
silence between pieces. A server that refuses to stream (a 4xx naming
*stream*) is asked again for the whole reply, for the rest of the
conversation; one that answers whole anyway is read as it is. **Stop**
closes the connection at once (`http::Hangup` keeps a handle on the
socket, through `Agent::with_parts` and a TCP connector of its own), so a
local server stops generating; through a proxy, at the next piece.

**Ollama's context.** Ollama 0.40.13's OpenAI-compatible endpoint cannot
be told a context size per request (*"The OpenAI API does not have a way of
setting the context size for a model"*, docs.ollama.com). A model gets the
server's: `ollama serve --help` gives `OLLAMA_CONTEXT_LENGTH` as *"default:
4k/32k/256k based on VRAM"*, docs.ollama.com/context-length as under
24 GiB, 24–48 GiB and over; on this computer's RTX 2060 the server log says
`vram-based default context total_vram="6.0 GiB" default_num_ctx=4096`. A
longer request is not refused: a test prompt of 11,624 tokens was logged
as `truncating input prompt limit=2050 prompt=11624 keep=4 new=2050`, the
first 4 tokens and the last 2,046 kept, and the model answered without its
instructions. The Assistant's first request is 4,843 tokens by Ollama's
count for qwen3 (the instructions and 32 tools), and `tequila_api`'s answer
is 43,672 characters, about 16,000 tokens: at 4,096, every step was cut,
and read again from the start.

`crates/tequila-ai/src/ollama.rs` asks before each round, for any OpenAI-
or Anthropic-format provider on this computer (whatever its port), on port
11434, or on a host named for Ollama: `GET /api/version` says whether it is
Ollama at all; `POST /api/show` gives `capabilities` and
`model_info.<architecture>.context_length`; `GET /api/ps` gives the loaded
model's `context_length`, `size` and `size_vram`. A model that is not
loaded is loaded first (`POST /api/generate` with only the model, shown as
*Loading the model*), since a request would load it anyway and only then
is its context known; the Modelfile's `num_ctx` is the fallback. The
request is estimated at a token for every four letters, every digit and
every two other characters, and 800 a picture: 5,888 for the first request
where Ollama counts 4,843, and 22,977 where it counts 21,076 once the
command catalog is in. (Four characters a token said 18,095 for the
second: JSON costs more than prose, and Ollama would have cut it.) A
request that does not fit with 2,048 tokens to spare for the answer is not
sent: the message gives its size, what the model
gets, and the Ollama app's next Context length step with room for the
command catalog (`ollama::GROWTH`, 16,000), both ways to set it, and that
the conversation is kept. It also says, before sending, when the model
cannot use tools; sends no pictures to a model without `vision` and says so
once; says once when part of the model runs on the processor; and, when a
request fits but the next long tool answer would not, says so early. A
model on this computer is sent only the newest screenshot
(`session::LOCAL_IMAGES_KEPT`). **Test** in Providers… checks tools and
context for an Ollama model.

**The panel** (`app/src/assistant.rs`) shows what the request is doing,
from `Session::progress`: phase, the reasoning and words so far, an
estimated token count, and the step, with the time since asking on the
right. A failure says what helps (a smaller or faster model, one that fits
the graphics card, the context length) and that asking again carries on.
Scripts can drive it: `{"op":"assistant","ask":…}`, `stop`, `open`, and the
answer's `phase` and `transcript` (`docs/SCRIPTING.md`).

Tried on this computer (RTX 2060, 6 GB) with Ollama 0.40.13 and
`qwen3:4b`, through the scripting channel in an isolated app box, each
server a separate `ollama serve` on a port of its own with
`OLLAMA_CONTEXT_LENGTH` set:

| Context | Model in memory | What happened |
|---|---|---|
| 4,096 | — | *Make the headband red*: the model loaded (19 s), the request (about 5,892 tokens) was refused with the 32k advice; only `/api/version`, `/api/show`, `/api/ps` and `/api/generate` reached the server, nothing to the chat endpoint |
| 16,384, flash attention, `q8_0` cache | 3.9 GB, all on the graphics card | *Which part is the headband, and which material is on it?*: loaded 3.5 s, read 10 s, thought 36 s, `tequila_nodes "headband"`, thought 13 s, answered *Headband, Soft Touch Black* — 67 s in all. *Make the headband red*: thought 2 min 6 s (2,285 tokens) and called `tequila_api`; the next request, about 18,021 tokens, was refused with the 32k advice |
| 32,768, the same | 5.7 GB, 73% on the graphics card | the same conversation asked again carried on: 21,076 tokens read in 68 s, 8 min 56 s of thought at 2.8 tokens a second, `tequila_nodes`, then 20 more minutes of thought without acting (3,135 tokens) until Stop; Ollama logged `cancel task` at once |

(Without flash attention and the `q8_0` cache, 16k took 5.4 GB, 79% on
the graphics card.)

Held by tests: `tequila-ai` (68) has stand-in servers on local sockets for
streamed words seen part-way, tool calls split and interleaved across
pieces, reasoning in all three forms, a slow first byte accepted, silence
part-way ending in an error that says what helps, a server that refuses to
stream and one that answers whole, Anthropic's event stream with thinking
and partial JSON, an error inside a stream, Stop mid-stream (the panel
free at once, the connection closed, the conversation able to go on), and
Ollama's check against canned `/api/version`, `/api/show`, `/api/ps` and
`/api/generate` replies (too long, loaded first then sent, no tools, blind
and half on the processor noted once, a missing model, a local server that
is not Ollama). `http.rs` tests the limits and the hang-up on paced
sockets; `ollama.rs` the estimate, the suggestion and the notes. The app's
`a_reply_on_its_way_is_shown_as_it_comes` draws the panel mid-request, and
`a_script_drives_the_assistant` the scripting op.

Left: the Assistant has no tool for the material library, though its
instructions say to prefer a library preset; `tequila_api`'s answer
(about 16,000 tokens) is most of what a local model has to hold, and a
slimmer catalog would let one work in 16k; LM Studio's context is not
read, so only its error is explained; the Modelfile `num_ctx` fallback and the Anthropic stream are
tested against stand-ins only; the estimate is calibrated on qwen3's
counts; through a proxy, Stop takes effect at the next piece.
