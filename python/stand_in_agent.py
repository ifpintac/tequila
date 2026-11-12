#!/usr/bin/env python3
"""A stand-in agent for Tequila's Assistant: no account, no model.

It speaks the Agent Client Protocol (version 1) over standard input and
output, as Claude's adapter does, starts the MCP bridge Tequila hands it,
and answers a message by doing what a few words in it say - through the
bridge, so the whole path is the real one: the panel, the bridge, the
scripting channel, the questions Tequila asks before something leaves the
scene.

Run Tequila with it in place of a real agent:

    TEQUILA_ASSISTANT_AGENT=/usr/bin/python3 \\
    TEQUILA_ASSISTANT_AGENT_ARGS=/path/to/python/stand_in_agent.py \\
    tequila-app

and choose Claude in the Assistant. What it understands, anywhere in a
message: `look` (a screenshot), `rename` (the first part), `save a copy`,
`render`, `import`, `preference`, `write` (a file in the project's folder,
asking first), `shell` (a command, which Tequila refuses), `slow` (takes its
time, to try Stop). `STAND_IN_SIGNED_OUT=1` makes it ask to be signed in.
"""

import json
import os
import queue
import subprocess
import sys
import threading
import time

incoming = queue.Queue()
answers = {}
answered = threading.Condition()
cancelled = threading.Event()
write_lock = threading.Lock()


def send(message):
    with write_lock:
        sys.stdout.write(json.dumps(message) + "\n")
        sys.stdout.flush()


def reader():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        message = json.loads(line)
        if "method" in message:
            if message["method"] == "session/cancel":
                cancelled.set()
                with answered:
                    answered.notify_all()
            else:
                incoming.put(message)
        else:
            with answered:
                answers[message.get("id")] = message
                answered.notify_all()
    incoming.put(None)


class Bridge:
    """Tequila's MCP bridge, started as the session was told to."""

    def __init__(self, server):
        env = dict(os.environ)
        env.update({pair["name"]: pair["value"] for pair in server.get("env", [])})
        self.process = subprocess.Popen(
            [server["command"]] + server.get("args", []), stdin=subprocess.PIPE, stdout=subprocess.PIPE, env=env, text=True
        )
        self.next = 0
        self.request("initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "stand-in", "version": "1"}})

    def request(self, method, params):
        self.next += 1
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.next, "method": method, "params": params}) + "\n")
        self.process.stdin.flush()
        return json.loads(self.process.stdout.readline())

    def call(self, name, arguments):
        answer = self.request("tools/call", {"name": name, "arguments": arguments})
        return answer.get("result", {"content": [{"type": "text", "text": json.dumps(answer.get("error"))}], "isError": True})


SETTINGS = [
    {
        "id": "model",
        "name": "Model",
        "category": "model",
        "type": "select",
        "currentValue": "small",
        "options": [
            {"value": "small", "name": "Stand-in Small", "description": "Answers at once"},
            {"value": "large", "name": "Stand-in Large", "description": "Says the same, slower"},
        ],
    },
    {
        "id": "effort",
        "name": "Thinking",
        "category": "thought_level",
        "type": "select",
        "currentValue": "medium",
        "options": [{"value": "low", "name": "Low"}, {"value": "medium", "name": "Medium"}, {"value": "high", "name": "High"}],
    },
]


class Session:
    def __init__(self, params):
        self.id = "stand-in-%d" % int(time.time() * 1000)
        self.folder = params["cwd"]
        servers = params.get("mcpServers", [])
        self.bridge = Bridge(servers[0]) if servers else None
        self.calls = 0

    def update(self, body):
        send({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": self.id, "update": body}})

    def say(self, text):
        for word in text.split(" "):
            if cancelled.is_set():
                return
            self.update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": word + " "}})
            time.sleep(0.02)

    def think(self, text):
        self.update({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": text}})

    def tool(self, name, arguments):
        """One of Tequila's tools, through the bridge, told as it goes."""
        self.calls += 1
        call = "call-%d" % self.calls
        self.update({"sessionUpdate": "tool_call", "toolCallId": call, "title": name, "name": "mcp__tequila__" + name, "kind": "other", "status": "pending", "rawInput": arguments})
        result = self.bridge.call(name, arguments)
        content = [{"type": "content", "content": block} for block in result.get("content", [])]
        failed = bool(result.get("isError"))
        self.update({"sessionUpdate": "tool_call_update", "toolCallId": call, "status": "failed" if failed else "completed", "content": content})
        text = " ".join(block.get("text", "") for block in result.get("content", []) if block.get("type") == "text")
        return text, failed

    def permission(self, title, kind, raw, locations=()):
        """Ask the client before a tool of the agent's own; True if allowed."""
        self.calls += 1
        call = "call-%d" % self.calls
        described = {"toolCallId": call, "title": title, "name": title.split(" ")[0], "kind": kind, "status": "pending", "rawInput": raw, "locations": [{"path": path} for path in locations]}
        self.update(dict(described, sessionUpdate="tool_call"))
        request = "ask-%d" % self.calls
        options = [{"optionId": "yes", "name": "Allow", "kind": "allow_once"}, {"optionId": "no", "name": "Reject", "kind": "reject_once"}]
        send({"jsonrpc": "2.0", "id": request, "method": "session/request_permission", "params": {"sessionId": self.id, "toolCall": described, "options": options}})
        with answered:
            while request not in answers and not cancelled.is_set():
                answered.wait(0.1)
        outcome = answers.pop(request, {}).get("result", {}).get("outcome", {})
        allowed = outcome.get("optionId") == "yes"
        self.update({"sessionUpdate": "tool_call_update", "toolCallId": call, "status": "completed" if allowed else "failed"})
        return call, allowed

    def prompt(self, blocks):
        text = " ".join(block.get("text", "") for block in blocks if block.get("type") == "text").lower()
        pictures = sum(1 for block in blocks if block.get("type") == "image")
        self.think("The person wrote %d words%s. " % (len(text.split()), " and sent %d picture(s)" % pictures if pictures else ""))
        self.think("I am a stand-in, so I do what the words say.")
        if "slow" in text:
            for second in range(30):
                if cancelled.is_set():
                    return
                self.think(" Still here after %d s." % second)
                time.sleep(1)
        scene, _ = self.tool("tequila_scene", {})
        self.say("I am a **stand-in** for a real assistant. The scene is open and I can reach it.")
        if "look" in text:
            self.tool("tequila_screenshot", {"viewport": True})
            self.say("\n\nThat is the viewport as it is now.")
        if "rename" in text:
            nodes, _ = self.tool("tequila_nodes", {"parts_only": True})
            first = json.loads(nodes)["nodes"][0]
            self.tool("tequila_apply", {"command": {"command": "scene.rename", "node": first["id"], "name": "Renamed by the stand-in"}})
            self.say("\n\nI renamed `%s`. Undo request takes it back." % first["name"])
        if "save a copy" in text:
            answer, failed = self.tool("tequila_save", {"path": os.path.join(self.folder, "copy.tequila")})
            self.say("\n\n" + ("The copy was not saved: " + answer if failed else "Saved a copy beside the project."))
        if "render" in text:
            answer, failed = self.tool("tequila_queue", {"width": 3840, "height": 2160, "filename": "front.png"})
            self.say("\n\n" + ("No render was started: " + answer if failed else "The render is in the queue."))
        if "import" in text:
            answer, failed = self.tool("tequila_import", {"path": os.path.join(self.folder, "model.glb")})
            self.say("\n\n" + ("Nothing was imported: " + answer if failed else "Imported."))
        if "preference" in text:
            answer, failed = self.tool("tequila_preferences", {"set": {"viewport.gizmo_size": 150}})
            self.say("\n\n" + ("The preference was left alone: " + answer if failed else "Changed the gizmo's size."))
        if "write" in text:
            path = os.path.join(self.folder, "notes.txt")
            _, allowed = self.permission("Write notes.txt", "edit", {"file_path": path}, [path])
            if allowed:
                with open(path, "w") as notes:
                    notes.write("Written by the stand-in agent.\n")
            self.say("\n\n" + ("Wrote `notes.txt` in the project's folder." if allowed else "I did not write the file."))
        if "shell" in text:
            _, allowed = self.permission("ls", "execute", {"command": "ls"})
            self.say("\n\n" + ("That should not have been allowed." if allowed else "Tequila does not let me run commands, so I did not."))
        self.say("\n\n- Scene edits happen at once\n- What leaves the scene asks first")


def main():
    threading.Thread(target=reader, daemon=True).start()
    session = None
    while True:
        message = incoming.get()
        if message is None:
            return
        method, params, request = message.get("method"), message.get("params", {}), message.get("id")
        reply = lambda result: send({"jsonrpc": "2.0", "id": request, "result": result})
        if method == "initialize":
            reply(
                {
                    "protocolVersion": 1,
                    "agentCapabilities": {"promptCapabilities": {"image": True}, "sessionCapabilities": {"systemPrompt": {}}},
                    "agentInfo": {"name": "stand-in", "title": "Stand-in agent", "version": "1"},
                    "authMethods": [{"id": "console-login", "name": "Anthropic Console (API usage billing)", "type": "terminal", "args": ["--version"]}],
                }
            )
        elif method == "session/new":
            if os.environ.get("STAND_IN_SIGNED_OUT"):
                send({"jsonrpc": "2.0", "id": request, "error": {"code": -32000, "message": "Authentication required"}})
                continue
            session = Session(params)
            reply({"sessionId": session.id, "configOptions": SETTINGS})
        elif method == "session/set_config_option":
            for setting in SETTINGS:
                if setting["id"] == params.get("configId"):
                    setting["currentValue"] = params.get("value")
            reply({"configOptions": SETTINGS})
        elif method == "session/prompt":
            cancelled.clear()
            try:
                session.prompt(params.get("prompt", []))
            except Exception as error:  # Said in the chat rather than lost.
                send({"jsonrpc": "2.0", "id": request, "error": {"code": -32603, "message": "The stand-in tripped: %s" % error}})
                continue
            reply({"stopReason": "cancelled" if cancelled.is_set() else "end_turn"})
        elif request is not None:
            send({"jsonrpc": "2.0", "id": request, "error": {"code": -32601, "message": "Method not found: %s" % method}})


if __name__ == "__main__":
    main()
