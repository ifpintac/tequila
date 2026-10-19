"""The ZBrush plugin against a stand-in for zbrush.commands (tests/stub), as
far as it can be checked without ZBrush:

    python check_bridge.py


1. Registration: the subpalette and button exist; running twice is safe.
2. The error path: TEQUILA_EXE = this Python, which rejects --send (exit 2),
   so the plugin must report the failure in a message.
3. The success path, with _send patched to return 0: the merged OBJ holds
   the two visible SubTools, de-duplicated names, polypaint on the first only,
   and the originally active SubTool is selected again.
"""
import os
import runpy
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "stub"))
os.environ["HOME"] = os.environ["USERPROFILE"] = tempfile.mkdtemp(prefix="zbhome-")
os.environ["TEQUILA_EXE"] = sys.executable

from zbrush import commands as zbc  # noqa: E402

plugin_path = os.path.join(HERE, "..", "tequila_bridge", "init.py")
g = runpy.run_path(plugin_path, run_name="__main__")
runpy.run_path(plugin_path, run_name="__main__")  # second run must not break
button = "zplugin:tequila:send to tequila"
assert "zplugin:tequila" in zbc.ITEMS, zbc.ITEMS
callback = zbc.ITEMS[button]
assert callable(callback)

# 2. error path
callback(button)
assert zbc.MESSAGES and "could not take" in str(zbc.MESSAGES[-1]), zbc.MESSAGES
print("error path message:", repr(zbc.MESSAGES[-1])[:160])
assert zbc.STATE["active"] == 3, zbc.STATE
selects = [e for e in zbc.LOG if e[0] == "select"]
assert [s[1] for s in selects] == [0, 3, 3], selects  # visible 0 and 3, then restore 3

# 3. success path
zbc.MESSAGES.clear()
callback.__globals__["_send"] = lambda exe, merged, log: 0
callback(button)
assert zbc.MESSAGES and zbc.MESSAGES[-1][0] == "note", zbc.MESSAGES
folder = os.path.join(tempfile.gettempdir(), "TequilaZBrush")
merged = [os.path.join(r, f) for r, _, fs in os.walk(folder) for f in fs if f == "Demo_Head.obj"]
assert len(merged) == 1, merged
text = open(merged[0]).read()
print(text)
assert "o Body\n" in text and "o Body (2)\n" in text
assert text.count("\nv ") == 8
assert "v 0 0 0 1.0000 0.0000 0.0000" in text  # painted SubTool: colour on v lines
assert "v 0 0 0\n" in text  # unpainted SubTool: plain
assert "f 5/5 6/6 7/7 8/8" in text  # second part re-indexed
assert "Hidden" not in text and "InClosedFolder" not in text
assert zbc.STATE["active"] == 3
print("ALL CHECKS PASSED")
