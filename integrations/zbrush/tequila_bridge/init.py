"""Send to Tequila: a ZBrush 2026 Python plugin.

Adds ZPlugin > Tequila > Send to Tequila. The button exports every visible
SubTool of the active Tool as an OBJ file, one file per SubTool. ZBrush writes
polypaint into those files as its #MRGB block. objmerge.py, which sits next to
this file, merges the OBJs into one file at a fixed path for each Tool. The
plugin then runs `"<Tequila>" --send "<merged.obj>"`.

Installation: copy init.py and objmerge.py together into the Python folder
of ZBrush's user assets (Preferences > Asset Directory > Open Directory, then
Python; make it if it is not there), and restart ZBrush. ZBrush runs the
Python files there at startup. Or put this folder on PYTHONPATH, where ZBrush
runs every init.py it finds.

Tequila is found from, in order: the TEQUILA_EXE environment variable, the
path saved after you last picked it, the installer's registry keys (Windows),
and the default install folders. If none of these work, the plugin asks you
to locate it once.

SPDX-License-Identifier: Apache-2.0
"""
import hashlib
import importlib.util
import json
import os
import re
import subprocess
import sys
import tempfile
import time

from zbrush import commands as zbc

SUBPALETTE = "ZPlugin:Tequila"
BUTTON = SUBPALETTE + ":Send to Tequila"

# Tool:SubTool status flags (zbrush.commands.get_subtool_status).
EYE = 0x0001         # the SubTool's own eye
FOLDER_EYE = 0x0002  # the eye of the folder the SubTool is in

# The Tequila installer's AppId (scripts/installer/tequila.iss). Inno Setup
# stores its uninstall entry under "<AppId>_is1".
INNO_UNINSTALL_KEY = (r"Software\Microsoft\Windows\CurrentVersion\Uninstall"
                      r"\{EDD88596-57CD-4475-98DD-4D3B719E8AB3}_is1")

# The longest `Tequila --send` is waited for. It returns as soon as an open
# Tequila has the model - or, with none open, as soon as it has started one -
# so this is only for a very large model's import.
SEND_WAIT_SECONDS = 300.0

HERE = os.path.dirname(os.path.abspath(__file__))
SETTINGS_PATH = os.path.join(os.path.expanduser("~"), ".tequila", "zbrush_bridge.json")


# --------------------------------------------------------------------------
# Settings and locating Tequila
# --------------------------------------------------------------------------

def _load_settings():
    try:
        with open(SETTINGS_PATH, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def _save_settings(settings):
    try:
        os.makedirs(os.path.dirname(SETTINGS_PATH), exist_ok=True)
        with open(SETTINGS_PATH, "w", encoding="utf-8") as f:
            json.dump(settings, f, indent=2)
    except OSError as error:
        print(f"[Tequila] could not save settings: {error}")


def _app_binary(path):
    """On a Mac, return the program inside a .app bundle. Return any other
    path unchanged."""
    if path and path.rstrip("/").endswith(".app"):
        return os.path.join(path.rstrip("/"), "Contents", "MacOS", "Tequila")
    return path


def _registry_candidates():
    try:
        import winreg
    except ImportError:
        return []
    found = []
    for root in (winreg.HKEY_CURRENT_USER, winreg.HKEY_LOCAL_MACHINE):
        # The installer's uninstall entry, which records InstallLocation.
        for view in (winreg.KEY_WOW64_64KEY, winreg.KEY_WOW64_32KEY):
            try:
                with winreg.OpenKey(root, INNO_UNINSTALL_KEY, 0, winreg.KEY_READ | view) as key:
                    location, _ = winreg.QueryValueEx(key, "InstallLocation")
                found.append(os.path.join(location, "Tequila.exe"))
            except OSError:
                pass
        # The optional .tequila file association: "C:\...\Tequila.exe" "%1".
        try:
            with winreg.OpenKey(root, r"Software\Classes\Tequila.Project\shell\open\command") as key:
                command, _ = winreg.QueryValueEx(key, "")
            match = re.match(r'\s*"([^"]+)"', command or "")
            if match:
                found.append(match.group(1))
        except OSError:
            pass
    return found


def _find_tequila():
    candidates = [os.environ.get("TEQUILA_EXE", ""), _load_settings().get("tequila", "")]
    if sys.platform == "win32":
        candidates += _registry_candidates()
        program_files = os.environ.get("ProgramFiles", r"C:\Program Files")
        local = os.environ.get("LOCALAPPDATA", "")
        candidates.append(os.path.join(program_files, "Tequila", "Tequila.exe"))
        if local:
            candidates.append(os.path.join(local, "Programs", "Tequila", "Tequila.exe"))
    elif sys.platform == "darwin":
        candidates += ["/Applications/Tequila.app",
                       os.path.expanduser("~/Applications/Tequila.app")]
    for candidate in candidates:
        path = _app_binary(candidate)
        if path and os.path.isfile(path):
            return path
    return None


def _ask_for_tequila():
    if sys.platform == "win32":
        picked = zbc.ask_filename("Tequila (*.exe)|*.exe||", "", "Locate Tequila.exe")
    else:
        picked = zbc.ask_filename("*", "", "Locate Tequila (Tequila.app/Contents/MacOS/Tequila)")
    path = _app_binary(picked)
    if path and os.path.isfile(path):
        settings = _load_settings()
        settings["tequila"] = path
        _save_settings(settings)
        return path
    return None


# --------------------------------------------------------------------------
# SubTools
# --------------------------------------------------------------------------

def _is_visible(index):
    status = zbc.get_subtool_status(index)
    if not status & EYE:
        return False
    # The ZScript reference says SubToolGetFolderIndex returns -1 when a
    # SubTool is not in a folder.
    in_folder = zbc.get_subtool_folder_index(index) >= 0
    return not in_folder or bool(status & FOLDER_EYE)


def _active_subtool_name(index):
    """The active SubTool's name. This is the long-standing ZScript idiom
    [IGetTitle, Tool:ItemInfo], which returns the name followed by a '.'."""
    try:
        name = (zbc.get_title("Tool:ItemInfo") or "").strip()
    except Exception:
        name = ""
    if name.endswith("."):
        name = name[:-1].rstrip()
    return name or f"SubTool {index + 1}"


def _work_dir():
    """The same folder every time for the same Tool, because Tequila matches
    a resent model by the file path it was sent from."""
    tool = zbc.get_active_tool_path() or "Tool"
    stem = os.path.splitext(os.path.basename(tool.replace("\\", "/")))[0]
    stem = re.sub(r"[^\w.-]+", "_", stem)[:40] or "Tool"
    digest = hashlib.sha1(tool.encode("utf-8", "replace")).hexdigest()[:8]
    folder = os.path.join(tempfile.gettempdir(), "TequilaZBrush", f"{stem}-{digest}")
    os.makedirs(os.path.join(folder, "parts"), exist_ok=True)
    return folder, stem


def _export_visible(indices, parts_dir, result):
    """Run inside zbc.freeze(). Writes into `result` and does not raise,
    because the docs don't say whether freeze() passes exceptions on."""
    try:
        names_seen = {}
        for n, index in enumerate(indices):
            zbc.set_notebar_text(f"Tequila: exporting SubTool {n + 1} of {len(indices)}",
                                 (n + 0.5) / len(indices))
            if zbc.select_subtool(index) != 0:
                raise RuntimeError(f"could not select SubTool {index}")
            name = _active_subtool_name(index)
            # Tequila keys parts by name, so names must be unique.
            names_seen[name] = names_seen.get(name, 0) + 1
            if names_seen[name] > 1:
                name = f"{name} ({names_seen[name]})"
            path = os.path.join(parts_dir, f"part_{n:03d}.obj")
            if os.path.exists(path):
                os.remove(path)  # so a press that wrote nothing is noticed
            zbc.set_next_filename(path)
            zbc.press("Tool:Export")
            if not os.path.isfile(path):
                raise RuntimeError(f"ZBrush did not write {path} for SubTool '{name}'")
            result["parts"].append((name, path))
    except Exception as error:  # noqa: BLE001 - reported to the user below
        result["error"] = error


# --------------------------------------------------------------------------
# Merging and sending
# --------------------------------------------------------------------------

def _load_objmerge():
    """Load objmerge.py from this folder under a private module name. It is
    not added to sys.path or sys.modules, because ZBrush shares one
    interpreter between all plugins."""
    path = os.path.join(HERE, "objmerge.py")
    spec = importlib.util.spec_from_file_location("tequila_bridge_objmerge", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _send(exe, merged, log_path):
    """Run `exe --send merged` and return its exit code, its output in
    `log_path`. None when it is still importing after SEND_WAIT_SECONDS: the
    model will still arrive."""
    kwargs = {}
    if sys.platform == "win32":
        kwargs["creationflags"] = getattr(subprocess, "CREATE_NO_WINDOW", 0)
    with open(log_path, "w", encoding="utf-8", errors="replace") as log:
        try:
            return subprocess.run([exe, "--send", merged], stdin=subprocess.DEVNULL, stdout=log,
                                  stderr=subprocess.STDOUT, cwd=os.path.dirname(exe),
                                  timeout=SEND_WAIT_SECONDS, **kwargs).returncode
        except subprocess.TimeoutExpired:
            return None


def _read_log(path):
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            return f.read().strip()
    except OSError:
        return ""


def send_to_tequila(sender=""):
    """The button's callback. ZBrush passes the button's item path."""
    try:
        count = zbc.get_subtool_count()
        if count < 1 or not zbc.is_enabled("Tool:Export"):
            zbc.message_ok("Select a PolyMesh tool with SubTools first.", "Send to Tequila")
            return
        visible = [i for i in range(count) if _is_visible(i)]
        if not visible:
            zbc.message_ok("No SubTool is visible.", "Send to Tequila")
            return

        exe = _find_tequila() or _ask_for_tequila()
        if not exe:
            zbc.message_ok("Tequila was not found. Set TEQUILA_EXE, or locate it when asked.",
                           "Send to Tequila")
            return

        folder, stem = _work_dir()
        parts_dir = os.path.join(folder, "parts")
        for old in os.listdir(parts_dir):
            if old.startswith("part_"):
                os.remove(os.path.join(parts_dir, old))

        original_index = zbc.get_active_subtool_index()
        original_id = zbc.get_subtool_id()
        result = {"parts": [], "error": None}
        try:
            zbc.freeze(lambda: _export_visible(visible, parts_dir, result))
        finally:
            # Select the SubTool that was active before. Look it up by its
            # unique ID first, then fall back to its index.
            restore = zbc.locate_subtool(original_id)
            zbc.select_subtool(restore if restore >= 0 else original_index)
            zbc.set_notebar_text("", 0)
        if result["error"] is not None:
            raise result["error"]

        merged = os.path.join(folder, stem + ".obj")
        _load_objmerge().merge(merged, result["parts"])

        log_path = os.path.join(folder, "send.log")
        code = _send(exe, merged, log_path)
        if code not in (None, 0):
            zbc.message_ok(f"Tequila could not take the model:\n{_read_log(log_path)}",
                           "Send to Tequila")
            return
        print(f"[Tequila] sent {len(result['parts'])} SubTool(s): {merged}")
        zbc.show_note(f"Sent {len(result['parts'])} SubTool(s) to Tequila", display_duration=1.5)
    except Exception as error:  # noqa: BLE001 - a plugin must not die silently
        print(f"[Tequila] {type(error).__name__}: {error}")
        zbc.message_ok(f"Send to Tequila failed:\n{error}", "Send to Tequila")


def register():
    if not zbc.exists(SUBPALETTE):
        zbc.add_subpalette(SUBPALETTE)
    # If ZBrush runs this file twice (for example, its folder is on both
    # PYTHONPATH and ZBRUSH_PLUGIN_PATH, or someone reloads it), delete the
    # old button so the new one uses the current callback.
    if zbc.exists(BUTTON):
        zbc.delete_interface_item(BUTTON)
    zbc.add_button(BUTTON, "Export the visible SubTools and open them in Tequila.",
                   send_to_tequila, width=1.0)


if __name__ == "__main__":
    register()
