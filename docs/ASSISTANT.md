# The Assistant: AI inside Tequila

Click **Assistant** on the toolbar and describe what you want, for example
*"make the strap black velvet, the lenses dark glass, and light it like a
studio product shot"*. The AI you have chosen does it in the open scene:
- It finds the parts.
- It changes materials, lighting and camera.
- It takes a screenshot to check its own work, then adjusts.

## The chat

The Assistant is a chat, docked on the right of the window. The button at
its top right moves it into **a window of its own**, to put beside Tequila
or on another display; the same button brings it back.

- **The conversation** shows your messages, the reply as it is written,
  the model's reasoning folded away under *Thought for …*, and each step it
  takes in the scene with a tick when it worked. A screenshot it took is
  shown under its step.
- **Past chats** are kept. The clock button at the top left lists them,
  newest first; click one to read it and carry on. The pencil starts a
  **new chat**.
- **The message box**, from left to right: **+** attaches a picture (or
  drop one on the box), the camera sends **the viewport as it is now**, and
  **@** names a part, a material or the selection - or type `@` and the
  first letters. On the right: how hard the model **thinks**, where it
  offers that, the **model**, and Send. Enter sends; Shift+Enter is a new
  line.
- While it works, the line above the box says what it is doing and for how
  long, and Send becomes **Stop**, which ends the request at once.
- **Undo request** takes back everything one request changed, as long as
  you haven't edited by hand since. Each step can also be undone on its
  own, and shows in the console (in amber, with the assistant's name).

If something fails, the message says what helps, and the conversation is
kept: ask again to carry on from where it stopped.

## What it may do

The assistant may use Tequila's tools and the files in the open project's
folder. It cannot run commands, reach the internet, or touch a file
anywhere else on the computer.

**Changes to the scene happen at once**, and can be undone. Before anything
that leaves the scene, the chat asks, with **Allow** and **Deny**:

- importing a file, or bringing in a texture pack;
- saving the project to a new place (saving it where it already is does
  not ask);
- rendering to a file, and exporting the scene;
- changing or resetting a preference;
- writing, running or restarting a plugin (turning one on has its own
  question, in the Plugins window);
- saving to, importing into, exporting from or deleting from your library;
- writing or changing a file in the project's folder. Reading there does
  not ask.

Tequila asks these itself, on the request as it arrives, whichever AI sent
it and whatever that AI's own settings say. Deny tells the assistant you
said no, and it carries on without. Stop refuses whatever was waiting.

It can also make **materials of your own** from a description (*"blue
terrazzo with brass flecks"*), showing you each try on a ball, and write
**plugins** when you ask for one (*"a panel that renames the selected parts
with a prefix"*). A plugin runs Python on your computer, so Tequila asks you
before its code is written, and again before it starts.

## Who answers

The model picker at the bottom right of the message box lists everything
connected, and **Connections…** at its foot opens the page where each is
set up:

- **Subscriptions and agents**: Claude, ChatGPT and Gemini, each on a card that
  says where it stands - *Not on this computer*, *Installed, not signed
  in*, *Signed in · paid for by a subscription* or *by an API key* - with
  the button for the next step (**Install**, **Sign in**, **Check
  again**) and, once it is signed in, its models to choose from.
- **API keys and this computer**: the services and local models Tequila's
  own loop talks to. Keys are kept in the Mac's Keychain, or encrypted to
  your Windows account.
- **AI apps that can use Tequila**, folded away: the other direction, for
  the Claude app, Cursor and the rest to work in Tequila from outside.

There are two kinds of answerer.

**An agent** is the AI company's own program, running on this computer:
**Claude**, **ChatGPT** and **Gemini**. Tequila starts it, hands it
Tequila's tools, and shows what it says and does. None is shipped with
Tequila: the first time you choose one, the chat offers **Install**. For
Claude and ChatGPT that downloads the
adapter into Tequila's own folder, `~/.tequila/agents` - Anthropic's
`@agentclientprotocol/claude-agent-acp` with the Claude program it runs, or
`@agentclientprotocol/codex-acp` with OpenAI's Codex, each Apache-2.0 and
about 400 MB. It needs Node.js on the computer; the chat says so when it is
missing. Delete that folder to remove them. One you installed yourself
(`npm install -g`) is used as it is.

**An API key or a model on this computer** is answered by Tequila's own
loop: OpenAI, Claude, Gemini, OpenRouter, a company gateway, Ollama, LM
Studio. See *API keys and models on this computer* below.

You can switch in the middle of a chat. Whoever was not there for the
earlier part is told how it went.

### Claude, and how it is paid for

Claude as an agent needs an account. Choose **Claude** and the chat says
what is missing:

- **Anthropic Console** opens Anthropic's own sign-in in a terminal window.
  It is billed by use, like an API key. Tequila never sees the login.
- **Add an API key…** connects an Anthropic key (console.anthropic.com).
  A key you have already connected for Claude under *API keys* is used
  without asking again.
- **Use my Claude subscription** signs in with your Claude plan instead,
  in the same terminal window, and bills nothing by use. A plan already
  signed in to Claude on this computer is used straight away.

After signing in, click **Try again**.

**The subscription is off until you turn it on,** with that button or the
checkbox on Claude's card under Connections, which also turns it off
again. It is your choice rather than Tequila's because Anthropic's terms
do not plainly allow it in an application that is not theirs:

- The Agent SDK's documentation says: *"Unless previously approved,
  Anthropic does not allow third party developers to offer claude.ai login
  or rate limits for their products, including agents built on the Claude
  Agent SDK."*
- Their legal page forbids a third-party application offering Claude.ai
  login or handling its credentials, and adds that this does not *"prevent
  an end user from signing in to the unmodified Claude Code binary with
  their own Claude subscription"*. Tequila does the second - the sign-in
  is Anthropic's program, in a terminal, and Tequila never sees it - but
  whether a button that starts it counts as "offering" is not said.

(Read on 2026-11-13: code.claude.com/docs/en/agent-sdk/overview and
code.claude.com/docs/en/legal-and-compliance.)

While it is off, Tequila starts the adapter with `--hide-claude-auth`,
which makes it refuse a subscription even when one is signed in on the
computer, and the card says so. Anthropic's Console sign-in page may
itself offer *Continue with* your plan when the account has no Console
organisation: that signs the plan in, and the card then offers to use it.
The choice is kept as `"claude_subscription"` in
`~/.tequila/assistant.json`.

### ChatGPT, and how it is paid for

ChatGPT as an agent is OpenAI's Codex. Choose **ChatGPT** and the chat
says what is missing:

- **ChatGPT** opens OpenAI's own sign-in page in your browser. Sign in
  with your ChatGPT account there - a Plus, Pro, Business or Enterprise
  plan - and the chat carries on by itself. Tequila never sees the login.
  This is the sign-in Codex itself offers, and what it uses counts against
  your plan's Codex limits.
- **Add an API key…** connects an OpenAI key (platform.openai.com), billed
  by use. A key you have already connected for OpenAI is used without
  asking.

**ChatGPT works through Tequila's tools alone.** Codex looks at files by
running commands, and its sandbox for those reads the whole disk without
asking. So here its command tool is switched off altogether, and its web
search with it: the same rule Claude is held to, no shell and no internet.
What that costs is that ChatGPT cannot read the files in the project's
folder, as Claude can; it can still write one there when you allow it. The
chat says so before you ask it anything.

### Gemini, and how it is paid for

Gemini as an agent is Google's Antigravity CLI (`agy`), which speaks the
protocol itself. One already on the computer is used; otherwise
**Install** runs Google's own installer, which puts `agy` where Google
puts it (`~/.local/bin` on a Mac, `%LOCALAPPDATA%\agy\bin` on Windows)
and adds it to your terminal's path. Then the chat says what is missing:

- **Log in with Google** opens Google's own sign-in page in your browser,
  for a personal Google account and its Google AI plan. **Log in from the
  terminal** does the same in a terminal window, by a code. Tequila never
  sees the login.
- **Log in with Gemini Enterprise** and **Gemini Enterprise Agent
  Platform** are for an organisation's account.
- **Add an API key…** connects a Gemini key (aistudio.google.com), billed
  by use. A key you have already connected for Google Gemini is used
  without asking.

It is started in its own sandbox (`--sandbox`), with its usage reporting
off (`--disable-telemetry`), in the mode that asks before it acts.

**What Google's terms say.** Antigravity's terms (antigravity.google/terms)
say: *"Using third party software, tools, or services to access the
Service (e.g. using OpenClaw with Antigravity OAuth) is a breach of this
Agreement."* Google's guide to this very protocol
(antigravity.google/docs/ide/extensions/acp) says: *"Any ACP client can
launch Antigravity CLI."* Nothing Google has published says which holds
for a personal plan used from an application like Tequila. Tequila starts
Google's own program and never touches the sign-in or its tokens; whether
to sign in with a personal account is yours to weigh. (Read 2026-11-13.)

## API keys and models on this computer

These are answered by Tequila's own loop, in the same chat, with the same
questions before anything leaves the scene. Its tools are Tequila's own:
the scene, materials and the library's presets, lighting, camera, studios,
renders, import, save, preferences and plugins. It has no tools for files
or commands at all.

Open the model picker, click **Connections…**, choose a starting point,
fill in the form, and click **Test**:

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
| **Custom**: a company's own AI, or any other compatible service | usually OpenAI-compatible | from whoever runs it | from whoever runs it |

A model that cannot look at images still works, without checking its own
screenshots: untick **The model can see images** for it. DeepSeek's and
Groq's presets start that way.

**Fetch models** asks the service which models it has; pick one from the
list. Several providers can be connected at once. The model picker switches
between them, even in the middle of a conversation.

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

### Custom: a company's AI or another service

Most company AI platforms offer an **OpenAI-compatible** endpoint; some are
Anthropic-compatible. Ask the team that runs it for:
1. the API address (for OpenAI-compatible services it usually ends in `/v1`);
2. a model name;
3. an API key;
4. if the key goes in a header other than `Authorization: Bearer`, the
   header's name (for example Azure-style `api-key`). Put it under
   **Advanced → Key header**.

If the model cannot look at images, untick **The model can see images**. It
still works, but it cannot check its results visually. If the service has its
own API that is neither format, Tequila needs a small adapter
(`crates/tequila-ai/src/wire.rs`). Its API documentation is enough to write
one.

### Where keys are kept

On a Mac, in your login Keychain: `~/.tequila/assistant.json` holds only a
note of where. On Windows, in `~/.tequila/assistant.json` (a portable
Tequila's `data` folder), encrypted with Windows' Data Protection API for
your Windows account. The file is useless on another computer or to another
user: a portable copy taken to another PC asks for its keys again. A key is
sent only to the address of the provider it belongs
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
(`tequila-cli --mcp`) themselves. **Connections…**, under *AI apps that can
use Tequila*, lists the ones on this computer, each with a **Connect**
button (Claude Code's is a command to
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

## As built, November 13: a chat, and Claude as an agent

The panel was a list of steps with a text box under it, answered only by
Tequila's own loop. It is now a chat that an agent can answer as well,
which is the first stage of letting the Assistant use the programs people
already have: Claude here, then ChatGPT (`codex-acp`) and Gemini.

**The pieces.**
- `crates/tequila-ai/src/chat.rs`: the chat every backend writes into
  (`Item`: You, Said, Thought, Step, Asked, Problem, Note), streamed into
  with `say`, `think` and `step`, and kept as
  `~/.tequila/chats/<id>.json` with its pictures as `<id>/N.png`.
- `crates/tequila-ai/src/acp.rs`: a client for the Agent Client Protocol,
  version 1 (agentclientprotocol.com), written against schema 1.31.0:
  JSON-RPC, a message a line, over the agent's standard input and output.
  `initialize`, `session/new` (or `session/resume`, `session/load`),
  `session/prompt`, `session/cancel`, `session/set_config_option`, the
  `session/update` notifications, and `session/request_permission`. It
  never blocks: the agent is read on a thread and `Agent::poll` hands the
  panel events once a frame. Files and terminals are not lent to the agent
  (no `fs` or `terminal` capability).
- `acp::judge` answers an agent asking to use a tool: Tequila's own tools
  go ahead, reading inside the project's folder goes ahead, writing there
  asks, and a command, the internet or a path outside the folder (through
  `..` or a link) is refused.
- `crates/tequila-scripting/src/approval.rs`: the questions. The bridge is
  started for the agent as `tequila-cli --mcp --assistant MARK`, and every
  request it sends carries the mark (`Request::agent`). The application
  names those requests for the Assistant in the console, counts their
  changes as the request's for Undo request, and keeps the ones
  `approval::question` has a question for (`Channel::drain_taken`,
  `Assistant::hold`) until the person answers. The bridge waits as long as
  that takes (`Connection::set_patience`).
- `app/src/assistant_agents.rs`: finding the adapter (Tequila's own
  install, then the usual places for Node programs, since an application
  opened from the Finder has a short `PATH`), installing it with npm,
  opening its sign-in in a terminal, and Claude's options: no `Bash`,
  `WebFetch`, `WebSearch`, sub-agents or skills; `Write` and `Edit` always
  ask; the mode pinned to the one that asks (the adapter starts in `auto`,
  where Claude decides for itself); bypass removed.
- `app/src/assistant.rs` is the state and `app/src/assistant_panel.rs`
  the panel. `python/stand_in_agent.py` is an agent with no model behind
  it, for tests and for looking at the panel without an account
  (`TEQUILA_ASSISTANT_AGENT`).

**The MCP bridge covers Tequila already**, and was not rebuilt. What has a
tool of its own: the scene, parts and materials; every command and batches;
undo; selection, camera, panels; screenshots and quick renders; background
renders to files; materials from recipes; studios; preferences; import;
save and save as; plugins. What is reached only through `tequila_request`,
the raw request: the material library (putting a preset on a part, saving,
importing, exporting), exporting the scene (glTF, USD, web, spin), texture
packs, updating a model from its file, the studio kit, Substance, and a
new or another project. Worth tools of their own later, the library first.

**Tried on this Mac (M3 Max), in a copy of the application with a home
folder of its own:**
- With the stand-in agent, through the real bridge and channel: a
  screenshot shown in the chat; a part renamed and taken back with Undo
  request; a save to a new place denied (nothing written, the agent told
  so) and a 4K render allowed; a file written in the project's folder only
  after Allow; a shell command refused without a question; the chat kept,
  listed and reopened; the panel in its own window.
- With Anthropic's adapter 0.106.0: **Install** put it in
  `~/.tequila/agents` (385 MB); it started, opened a conversation with
  Tequila's bridge, and listed its models (Default, Opus 5.5, Fable 5.2,
  Sonnet 5.5, Haiku 5.5) and thinking levels in the pickers; signed out,
  the chat showed the sign-in card with **Anthropic Console**, and a
  message was answered *Authentication required* and kept for after.

**Not tried:** a real conversation with Claude. That needs an Anthropic
account, and signing in is the owner's to do. Until it is, what Claude's
adapter sends for a tool call (its titles, how it names Tequila's tools,
whether a screenshot comes back as a picture) is known from its source and
the protocol, not from watching it. The terminal sign-in was not clicked.

**Left:**
- Pasting a picture from the clipboard (egui 0.29 hands over pasted text
  only); attach and drop work.
- A dropped picture was not tried with a real drag.
- Reopening a chat carries on the agent's own conversation only where the
  project's folder is the same; otherwise the agent is told how it went,
  in words.
- The Windows side is written (`node.exe`, `npm.cmd`, a console for the
  sign-in, no console window behind the agent) and compiles in CI, but
  nothing of it has run on a PC.
- The Connections page (which programs are installed and signed in, with
  Install and Sign in for each) is a later stage; today the chat's own
  card says what Claude still needs.

Held by tests: `tequila-ai` `acp::` (11, against a stand-in agent on a
local socket: a whole turn, sign-in, permissions and Stop, resume and
load, settings, the account, an agent that ends, reach) and `chat::` (7);
`tequila-scripting` `approval::` (4); `tequila-cli`
`for_the_assistant_every_tool_is_offered`; the app's
`an_agent_answers_in_the_chat_and_is_held_to_its_reach`,
`what_leaves_the_scene_waits_for_the_person` (the real channel, the
bridge's own client), `stopping_refuses_what_was_waiting_for_an_answer`,
`chats_are_kept_and_opened_again`,
`an_agent_that_ends_as_it_starts_is_said_once`,
`the_panel_draws_every_kind_of_thing`, and `assistant_agents::` (6).

## As built, November 13: ChatGPT as an agent

The second agent, on the first's footing: `assistant_agents::Kind::ChatGpt`
names the adapter (`@agentclientprotocol/codex-acp` 2.6.0, which brings
`@openai/codex`), `start_with` gives each agent what it is started with,
and the rest - Install, the chat, the pickers, the questions - is what
Claude uses.

- **Started read-only.** `INITIAL_AGENT_MODE=read-only` and the `mode`
  setting pinned to `read-only` (the adapter's default is `agent`, "Auto
  review", where Codex judges its own actions): approval on request, a
  read-only sandbox, no network. `CODEX_CONFIG={"web_search":"disabled"}`.
- **Signing in** is one of the agent's own ways (`authenticate`, method
  `chat-gpt`), not a terminal: the adapter opens OpenAI's page and answers
  when it is done. The card shows *Signing in…* meanwhile
  (`Setup::SigningIn`); a sign-in that fails is said in the chat and the
  ways are offered again. A connected OpenAI key is handed over as
  `OPENAI_API_KEY` with `DEFAULT_AUTH_REQUEST`, and only a key whose
  address is `api.openai.com`: a company gateway's key is never given to
  Codex.
- **Quarantine.** The task expected the Codex program to need its
  quarantine flag cleared on a Mac. Installed through npm it has none, and
  it is signed by OpenAI (checked with `xattr` and `codesign` on the
  installed `codex`), so nothing is cleared. That would matter only for a
  release downloaded by hand in a browser.

**Tried on this Mac,** in a copy with a home folder of its own: ChatGPT
chosen, *not on this computer*, **Install** (383 MB), the adapter started
and said *Not logged in*, and the card offered **ChatGPT** and **Add an
API key…**. Asked directly with a key that is not one, the adapter opened
a conversation in `read-only` and listed its models (6.1 Sol and eight
more) and reasoning levels; OpenAI answered that key 401, as it should.

**Not tried:** the ChatGPT sign-in itself and a conversation: both need
the owner's account. So how Codex names Tequila's tools when it asks, and
whether it asks before each one in `read-only`, is not yet seen; the chat
lets through what it recognises as Tequila's and asks about what it does
not. OpenAI's terms for signing in to Codex from another application were
not read; it is the sign-in Codex offers every client of its adapter.

**Left:** reading outside the project's folder (closed later the same
day: see *ChatGPT held to Claude's rules*). Windows is unverified, as for
Claude.

Held by tests: `acp::signing_in_through_the_agent_opens_the_conversation_or_says_why_not`
and `assistant_agents::chatgpt_starts_read_only_and_signs_in_with_a_connected_openai_key`.

## As built, November 13: the same questions for API keys and local models

Tequila's own loop already answered in the new chat. What it did not have
was the questions: its tools simply left out what leaves the scene (no
import, no save to another place, no render to a chosen file, no
preferences). Now it has those tools, and is asked about them as an agent
is.

- `tools.rs`: `tequila_import`, `tequila_save` with a `path`,
  `tequila_queue` with `folder` and `filename`, `tequila_preferences`,
  and the library - `tequila_query` `library` and `tequila_library_apply`,
  which closes the old gap between "prefer a library preset" in its
  instructions and no tool to use one. Its instructions say that Tequila
  asks, so the model calls the tool rather than asking first.
- `Assistant::carry_out_agreed`: the request a call becomes is put to
  `approval::question`, the same function an agent's requests go through.
  A call with a question waits (`call_waiting`) for Allow or Deny; Deny
  answers the model in words, and Stop drops it.
- A path the model gives without a folder is taken to be in the project's
  folder (`in_project`), not wherever Tequila was started from.
- Running a plugin now asks here as well. Turning one on, and the change
  to one that is on, keep the questions they had.

Tried on this Mac with Ollama 0.40.16 and `qwen3-coder:30b`, in a copy
with a home folder of its own: *"Save a copy of this project as
copy.tequila"* - the model called the tool, the chat asked, Deny, and the
model answered that it was declined, with nothing written; asked again
for `copy2.tequila` and allowed, the file was saved beside the project.
(In the same request the model failed to rename a part, guessing at
command names; that is the model, and the chat showed each failed step.)

Held by tests: `the_own_loops_tools_that_leave_the_scene_ask_first`,
`a_plugin_run_by_the_assistant_answers_with_its_log` (now through the
question), and in `tequila-ai`
`what_leaves_the_scene_is_a_request_the_application_can_ask_about`.

## As built, November 13: the Connections page

*Connect an AI* listed the AI apps first and the providers under them, and
knew nothing of agents: what Claude still needed was said only in the
chat. It is **Connections** now (`app/src/assistant_connections.rs`), with
the agents first.

- **Where an agent stands is asked of the agent.** While the page is open
  each agent that is installed is started for a moment with nothing to
  work on (`Connection::look`): it says who is signed in and what pays
  (`_auth/status_update`), or refuses to open a conversation (Codex,
  -32000), and lists its models; then it is let go. An agent the person is
  signing in to through the agent itself (ChatGPT, in the browser) is kept
  until that is done. Closing the page lets them all go, and the chat
  looks again at what its own agent needs.
- **Install** and the terminal sign-ins are the ones the chat's card uses
  (`assistant_agents`). **Check again** asks the agent afresh, for after
  a sign-in in a terminal, which Tequila cannot see finish.
- **A model per agent**: the one chosen is kept
  (`Settings::choices`, `agent:claude/model`) and set on the chat's own
  agent at once if it is running, otherwise when it next starts.
- API keys are where they were: the Mac's Keychain, or encrypted to the
  Windows account (`secret.rs`).
- Also here: Claude's sign-in card shows as its conversation opens again.
  The ChatGPT change had put that off until the first message failed.

Tried on this Mac, in a copy with a home folder of its own, with
Anthropic's adapter installed and OpenAI's not: the page said *Claude:
Installed, not signed in* with **Sign in: Anthropic Console**, and
*ChatGPT: Not on this computer* with **Install**; an Ollama provider was
listed under it.

Not tried: a signed-in agent on the page (needs an account; the stand-in
tests cover the states), and Windows.

Held by tests: `assistant_connections::` (4: signed in with its models,
signed out both ways and signed in through the agent, an agent that will
not start, the page drawn in every state) and
`an_agent_nobody_is_signed_in_to_says_so_as_it_opens`.

## As built, November 13: Gemini as an agent

The third agent. Unlike the other two it is not an npm package with an
adapter: Google's Antigravity CLI speaks the protocol itself, so
`assistant_agents` now knows two ways a program gets onto the computer
(`Source::Npm`, `Source::Program`).

- **Tried on this Mac first, as asked.** CLI 1.5.7, taken from Google's
  bucket and checked against Google's manifest (sha512) and macOS
  (*Notarized Developer ID, Google LLC*). `agy acp` answers `initialize`
  as ACP version 1; signed out, `session/new` answers -32000 and it
  offers `oauth-personal`, `oauth-business`, `gemini-api-key`,
  `agent-platform` and `terminal-login` (`--login`). With a key that is
  not one, `authenticate` succeeded, a conversation opened **with
  Tequila's MCP bridge in `mcpServers`**, and it listed its models
  (Gemini 4 Argon, 3.8/3.7/3.6 Flash, 3.1 Pro), two thinking levels and
  the modes `default`, `accept-edits` and `plan`. It still opened with
  `--sandbox`, `--disable-telemetry` and Tequila's instructions under
  `_meta.antigravity.systemPrompt.append`.
- **Install** fetches Google's installer and runs it as Google says to
  (`curl … | bash`; `irm … | iex` in PowerShell). The installer checks its
  download against its manifest, clears the quarantine flag itself, and
  runs `agy install`, which adds the folder to the shell's path.
- **A key that wants asking for.** Antigravity has no way to be told at
  start to use a key, so a connected Gemini key is put in its environment
  (`GEMINI_API_KEY`) and it is asked to sign in with it
  (`Kind::key_sign_in`), once, when it says it needs signing in.
- The mode is pinned to `default`, which asks.

Tried in a copy with a home folder of its own: Gemini chosen, *not on this
computer*, **Install** (Google's installer put `agy` in that home's
`.local/bin`), the real program started and the card offered its
sign-ins.

**Not tried:** a Google sign-in and a conversation, which need the owner's
account. So what Antigravity asks permission for, and whether its sandbox
keeps its reading to the project's folder, is not yet seen; the chat lets
through what it recognises as Tequila's, lets it read in the project's
folder, asks before it writes there, and refuses a command, the internet
and any other folder when it is asked. Whether the instructions under
`_meta` are read is unknown (they were not refused). Windows is
unverified.

Held by tests: `gemini_is_googles_own_program_started_in_its_sandbox` and
`a_connected_key_signs_an_agent_in_that_wants_asking`.

## As built, November 13: ChatGPT held to Claude's rules

ChatGPT was started in Codex's `read-only` mode, which lets its commands
read any file on the disk without asking. The owner's rule is no shell and
no other folders, for every AI.

- Codex has a switch for its command tool, `features.shell_tool`
  ("Enable command execution tools (stable; on by default)", in its
  configuration reference; in its source the tools are not registered at
  all when it is off). It goes in the `CODEX_CONFIG` Tequila already
  passes, beside `web_search = "disabled"`.
- The `read-only` mode stays, so a change to a file still asks.
- Considered and left: a Codex permission profile that keeps reading to
  the project's folder. It would let ChatGPT read project files, but it is
  marked beta, has to live in a Codex home of Tequila's own (so the person
  would sign in again there), and on Windows needs a sandbox that not
  every PC has.

Checked without an account, with the Codex program Install puts in place
(0.167.2): `codex features list -c features.shell_tool=false` reports the
switch off, `codex debug prompt-input` no longer offers the model
`write_stdin` and offers `exec_command` in half as many places, and the
adapter opens a conversation with the setting. **Not checked:** a real
conversation, where ChatGPT would be seen to have no way of running a
command.

Held by test:
`chatgpt_starts_read_only_and_signs_in_with_a_connected_openai_key`.

## As built, November 13: the Claude subscription is the person's own switch

The subscription was off, with a line in `assistant.json` as the only way
to turn it on. The first person to try it, the owner, clicked **Anthropic
Console**; Anthropic's page answered that the account had no Console
organisation and offered *Continue with Claude Ultra*; and Tequila then
refused the plan it had just been signed in with, with nothing in the
window to change that.

- The card has **Use my Claude subscription**, and Claude's card under
  Connections a checkbox that turns it on and off
  (`Assistant::set_claude_subscription`). Either lets Claude's program go
  and starts it again as chosen. It is still off until the person turns
  it on, and the card says why it is theirs to decide.
- The terminal a sign-in opens in is given the home folder Tequila has
  (`open_terminal`), so the sign-in lands where the agent will look. They
  are the same in ordinary use; they differ when Tequila is run with a
  home of its own, as a test copy is.

Found on the way, about test copies and not about Tequila: on a Mac the
login Keychain is found through the home folder, so a copy run with a
home of its own cannot see a Claude sign-in made in the real one
(`security find-generic-password` answers 44 there). A copy for trying
sign-ins has to run on the real home.

Held by test: `the_person_turns_their_claude_subscription_on_themselves`.

## As built, November 13: the library first, and less to guess

The first real session with Claude, the owner's, put materials on a
product and stood it on a pedestal. It got there, and its own account
of the way was a list of what Tequila made hard:

- **It made materials from nothing instead of looking in the library.**
  There was no tool for the library in the bridge the agents use. The only
  way to see it was the raw request `query library`: every preset with its
  tags and files, 355,000 characters for 602 presets, which the assistant
  did not open.
- **It guessed field names.** The whole catalog of commands is tens of
  thousands of characters, so it wrote a state as `visible` where the
  field is `value`. That read as false, hid the pedestal it meant to show,
  and said nothing. The floor reflection's name (`ground_reflections`) it
  found by trying.
- **There was no way to add a cylinder.** It imported a stand-in and
  reshaped it. The reshape read back right and the viewport went on
  drawing the stand-in, because it named the mesh the part already had and
  the viewport keeps what it built for a mesh by its key.
- **It looked at pictures that had not finished**: grainy and dark, a
  second after a change, and judged them.

What changed:

- **`tequila_library`** searches the library: words that must all be in a
  preset's name, category, type, tags or description, the ones with them
  in their name first, twenty unless more are asked for, each as a name, a
  category, a type and a line about it. With nothing asked it lists the
  categories. A search answers in a page (`document::library_search`; the
  request is `library` with the action `search`). **`tequila_library_apply`**
  puts a preset on parts by that name. The instructions every assistant is
  given now say to look there first, the tool's own description says so,
  and `tequila_material_create` says it is for when the library has
  nothing close. The same in Tequila's own loop for API keys and local
  models, and in the Tequila skill, which used to send an AI to the whole
  list.
- **`tequila_command`** answers for one command: its fields, an example
  that is itself a valid command, and what the example cannot say - that a
  state's `value` is required, the environment's parameter names, that a
  reshape may leave its mesh out (`document::help`; the request `help`). A
  wrong name is answered with the ones it might have meant. Every command
  has a page under 12,000 characters.
- **A command that cannot be read is answered with how it is written**:
  the reason, then an example of that command, or for a name that is no
  command the ones it might have meant (`scene.delete` → `scene.delete_nodes`).
  A local model tried on the new build guessed `scene.set_transform` three
  times before looking it up; the refusal now carries what the lookup
  would have said (`protocol::unread_command`).
- **A state with no `value` is refused**, with the shape it should have,
  rather than read as false (`journal::read_flags`). Projects and journals
  always wrote it, so nothing saved is affected.
- **`tequila_add_shape`** adds a plane, cube, cylinder, sphere, disc,
  backdrop or wall of given sizes as one undo step (the request
  `add_shape`). A round shape may be sized by its `diameter` or `radius`
  as well as its `width`: a model that gave a cylinder a radius, which
  only a backdrop read, got the default size four times running.
- **A reshaped part gets a mesh of its own** when a script names none, or
  names the one the part has (`channel::with_fresh_mesh`). This is at the
  channel, for scripts and assistants; the viewport's habit of keeping a
  mesh by its key is unchanged.
- **A screenshot waits for the picture.** `tequila_screenshot` gives the
  viewport up to twelve seconds (`settle`; sixty at most) to finish: in the
  Rendered view, until the picture shown is of that view, answers the
  newest request - not the one from before the change - and has 64
  samples, 16 when it denoises, or has mostly converged
  (`channel::picture_settled`). It returns as soon as that is so.
  It is taken as it is when the time runs out, and the tool says with the
  picture whether it had finished and how many samples it had, so a grainy
  one is not taken for the material. The Assistant's own screenshots wait
  the same.
- **`tequila_materials` is short**: ids, names, types and who wears them.
  It answered 291,000 characters on that scene. **`tequila_material`**
  reads one whole, for when a preset's colour turns out to be inside a
  pattern.

**Tried on this Mac (M3 Max), in a copy with a home folder of its own:**
- Through the real bridge: the categories (552 characters), a search for
  "black leather" (ten presets, 2,400 characters), a preset put on the
  sphere, a cylinder added and reshaped with no mesh named and drawn at its
  new size, a state written with `visible` refused with how it is written,
  `scene.set_visible` answered with `scene.set_visibility`.
- The screenshot: taken at once after switching to the Rendered view it
  said "NOT finished"; given time after a material change it came back in
  3 seconds at 20 denoised samples and said so. The first try called a
  picture finished at 0 samples - the quick view's, still up a moment
  after the switch - which is why a picture now has to be of the Rendered
  view and answer the newest request. Cycles starting cold took longer
  than the wait and was reported unfinished, correctly.
- A model on this computer (qwen3-coder 30b, through Tequila's own loop),
  asked for black leather on a low round walnut pedestal: it searched the
  library for both and applied presets, before and after. Before the
  refusals carried examples and a round shape took a radius, it took 27
  steps with 9 refused; after, 15 with 1.

Not tried: Claude, ChatGPT or Gemini on this build. The owner's session was
with Claude on the build before, and is what this answers.

Not changed here, and worth doing:

- **The view moved closer to the product after some material changes.**
  Seen in that session, cause not found.
- **Tinting a library preset whose colour is set inside a pattern** still
  means reading the material and rewriting that pattern. A plain "make
  this preset this colour" would save it. (Built since: "one step to
  recolour a material", below.)
- **The viewport keeping a stale mesh under a reused key** is worked
  around for scripts, not fixed where it happens.
- An assistant still cannot read the library's files directly, which is as
  intended: they are outside the project's folder. The search is the way
  in.

Held by test: `the_library_is_searched_in_a_size_an_ai_reads`,
`one_command_is_looked_up_by_its_name`,
`the_requests_that_spare_a_guess_are_read`,
`a_state_without_its_value_is_refused`,
`a_script_adds_a_shape_of_a_size_as_one_step`,
`a_reshaped_part_is_given_a_mesh_of_its_own`,
`a_script_searches_the_library_and_looks_up_a_command`,
`a_screenshot_waits_for_the_picture_to_finish`, and in the bridge
`the_library_comes_first` and
`a_screenshot_says_whether_its_picture_had_finished`.

## As built, November 13: one step to recolour a material

In the same first session the assistant put a library leather on a part
and then held back from making it dark green. It was right to: that
leather's brown is not in `base_color` but inside the node that multiplies
its grain, so setting `base_color` would have changed nothing, and the
only other way was to read the whole material and rewrite its pattern.
More than half of the library is like that: of 599 shipped presets, 316
keep their colour in a pattern (a multiplied colour, the colours threads
are mixed from, the stops of a ramp), and 283 in a plain value.

What changed:

- **`tequila_material_tint`** takes a material's id and a hex colour and
  makes it the same material in that colour, wherever the colour is kept.
  The colour asked for becomes the material's main colour; the other
  colours of the same family keep their relation to it, so a pattern keeps
  its contrast; colours of another family (white threads in a blue cloth,
  rust on a grey steel) are left exactly as they are. Pattern, grain,
  roughness and bump are not touched. It is one command on the bus, so
  one undo step. The rule is written out in `docs/SCRIPTING.md` under
  "Tinting a material", and is `tequila_materials::tint`.
- **Its answer says what it did**: the colour it was and is, each
  parameter changed with how many colours and whether they were in a
  pattern or a plain value, how many colours it left alone, and in words
  anything worth knowing - that other parts share the material and changed
  too, or that a metal could not be as bright as the colour asked for. It
  comes back with a picture of the material on the ball, as
  `tequila_material_update` does.
- **A material with no colour to change is refused in a sentence** that
  names it and says why: a thin film, whose colours come from its
  thickness, or a colour that is a picture's. Nothing is changed.
- **`tequila_library_apply` takes `color`**, to put a preset on already
  tinted: a material of its own named for the colour
  (`Leather Tan #1f4d2b`). It also answers with the material's id now, so
  the next step need not look it up.
- The instructions every assistant is given, both tools' descriptions,
  Tequila's own loop for API keys and local models, and the Tequila skill
  all say to use the tint and not `base_color` for "the same material in
  another colour". The request on the scripting channel is `material` with
  the action `tint`; in Python `material.tint('#1f4d2b')` and
  `fs.library.apply(name, part, color='#1f4d2b')`.

Three things the survey of the library decided:

- **Which colour is "the" colour of a weave.** The value a preset names is
  often a blend of its threads, nearer the white thread of an Oxford cloth
  than the blue one. So under a pattern the main colour is the pattern's
  own colour nearest that value among those at least as vivid: the blue
  thread, which comes out exactly the colour asked for.
- **A colour that is only a grey level.** Two presets (Cast Iron, Steel
  Galvanised) wire a pattern of numbers straight into the colour, with no
  colour anywhere. A tint puts one between them, so that what is seen is
  unchanged until it is tinted and every later tint finds it.
- **A clear material's colour is not the colour it looks.** It is what one
  transmission depth lets through, a pale blue for an acrylic that looks
  deep blue on a part five times as thick. Given the amber asked for as it
  stood, that acrylic rendered nearly black. So a clear material that has a
  colour keeps its clarity - the new colour thinned as a dye is, to take as
  much out of the light as the old one did - and only a colourless one
  takes the colour as given. The answer says when this was done and what
  the colour came to.

**Tried on this Mac (M3 Max):** every one of the 599 shipped presets tinted
dark green in a test, each held to come out that colour (a metal as near as
the metal allows), to compile, to read back from its own file, and to stay
put when tinted the same colour again. Twenty presets rendered on the
library's ball before and after, and looked at: leathers, suede, denim,
Oxford cloth, felt, walnut, marble, terrazzo, a carbon and aramid weave,
rusted steel, plastic, anodised aluminium, chrome, a metallic paint, tinted
acrylic, clear glass, an amethyst, a gummy sweet, a neon tube and a backlit
button.

Not tried: an assistant using the tool in a real session. The tool's
description is the only thing that tells it to, and whether it reaches for
it unprompted is for the next session to show.

Not changed here:

- **A tint is not offered in the Material tab.** The rule is in a crate
  the window can call (`tequila_materials::tint::tint`); a "Tint..." beside
  a patterned material's colour would be the same step by hand.
- **A material keeps its name when it is tinted**, as it does when its
  colour is changed by hand, so a `Leather Tan` made green is still called
  that. Applied already tinted, it is named for the colour.
- **The pattern's light and dark keep how light they look, not how much
  light they reflect**, so a deep blue made a red of the same lightness
  reflects more. That is the point of working in Oklab, and is what a
  person means by the same contrast.

Held by test: in `tequila-materials`, the `tint::tests` (a plain value, a
colour times a pattern, a ramp's contrast, grey beside a colour, a colour
on a grey, a black pattern, a sheen, glass and glow, a tinted clear
material, a metal, a grey-level pattern, the refusals, a second tint); in `tequila-format`,
`every_shipped_preset_tints_or_says_why_not`; on the channel,
`a_plain_preset_is_tinted_in_its_value`,
`a_patterned_leather_is_tinted_in_its_pattern`,
`a_cloth_keeps_its_weave_in_the_new_colour`,
`a_preset_is_applied_already_tinted`,
`a_material_with_no_colour_is_refused_plainly` and
`a_tint_carries_a_material_and_a_colour`; and `a_tint_is_one_tool` in each
bridge.
