"""A fake zbrush.commands for exercising the plugin outside ZBrush.

Four SubTools: 0 visible + painted, 1 hidden, 2 in a hidden folder,
3 visible + unpainted (named the same as 0 to test de-duplication).
"""
LOG = []
ITEMS = {}
SUBTOOLS = [
    {"name": "Body", "status": 0x1, "folder": -1, "paint": True, "id": 100},
    {"name": "Hidden", "status": 0x0, "folder": -1, "paint": False, "id": 101},
    {"name": "InClosedFolder", "status": 0x1, "folder": 0, "paint": False, "id": 102},
    {"name": "Body", "status": 0x1, "folder": -1, "paint": False, "id": 103},
]
STATE = {"active": 3, "next": "", "frozen": False}
MESSAGES = []


def _log(*a):
    LOG.append(a)


def exists(path):
    return path.lower() in ITEMS or path.lower() in ("tool:export",)


def add_subpalette(path, title_mode=0, *a, **k):
    ITEMS[path.lower()] = "sub"
    return True


def add_button(path, info="", fn=None, initially_disabled=False, width=0.0, hotkey="", icon_path="", height=0.0):
    ITEMS[path.lower()] = fn
    return True


def delete_interface_item(path):
    return ITEMS.pop(path.lower(), None) is not None


def is_enabled(path):
    return True


def get_subtool_count(index=-1):
    return len(SUBTOOLS)


def get_subtool_status(index=-1):
    return SUBTOOLS[STATE["active"] if index < 0 else index]["status"]


def get_subtool_folder_index(index=-1):
    return SUBTOOLS[STATE["active"] if index < 0 else index]["folder"]


def get_active_subtool_index():
    return STATE["active"]


def get_subtool_id(index=-1, subToolIndex=-1):
    return SUBTOOLS[STATE["active"] if subToolIndex < 0 else subToolIndex]["id"]


def locate_subtool(uid):
    for i, s in enumerate(SUBTOOLS):
        if s["id"] == uid:
            return i
    return -1


def select_subtool(index=-1):
    _log("select", index)
    STATE["active"] = index
    return 0


def get_title(path, full_path=False):
    assert path == "Tool:ItemInfo"
    return SUBTOOLS[STATE["active"]]["name"] + "."


def get_active_tool_path():
    return r"D:\sculpts\Demo Head.ztl"


def set_next_filename(path="", template_path=""):
    STATE["next"] = path


def press(path):
    _log("press", path)
    assert path == "Tool:Export"
    s = SUBTOOLS[STATE["active"]]
    lines = ["# fake ZBrush OBJ", "v 0 0 0", "v 1 0 0", "v 1 1 0", "v 0 1 0"]
    if s["paint"]:
        lines.append("#MRGB ffff0000ff00ff00ff0000ffffffffff")
    lines += ["vt 0 0", "vt 1 0", "vt 1 1", "vt 0 1", "g Polygroup_1", "f 1/1 2/2 3/3 4/4"]
    with open(STATE["next"], "w") as f:
        f.write("\n".join(lines) + "\n")


def freeze(fn, fade_time=0.0):
    STATE["frozen"] = True
    try:
        fn()
    finally:
        STATE["frozen"] = False


def set_notebar_text(text, progress=0.0):
    _log("notebar", text)


def message_ok(message="", title=""):
    MESSAGES.append(message)


def show_note(text, item_path="", display_duration=0.0, *a, **k):
    MESSAGES.append(("note", text, display_duration))
    return 0


def ask_filename(extensions, default_file_name="", title=""):
    return ""
