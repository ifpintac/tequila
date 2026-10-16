# Tequila LiveLink for Blender.
#
# Sends the scene to a running Tequila as you model: the meshes (with
# modifiers applied), where they are, and on first send their materials,
# mapped from the Principled BSDF. Tequila keeps every material, label and
# visibility change made there, so you model in Blender and finish in
# Tequila without exporting. See docs/livelink-protocol.md in the Tequila
# repository for the protocol, which other applications can speak too.
#
# Talks only to this computer: the port and token Tequila writes to
# ~/.tequila/channel.json. Nothing goes anywhere else.
#
# SPDX-License-Identifier: Apache-2.0

bl_info = {
    "name": "Tequila LiveLink",
    "author": "Tequila",
    "version": (1, 0, 0),
    "blender": (4, 1, 0),
    "location": "3D Viewport > Sidebar > Tequila",
    "description": "Send the scene to a running Tequila as you model",
    "category": "Import-Export",
}

import base64
import json
import os
import socket
import time

import bpy
import numpy as np

PROTOCOL = 1
# Blender is Z up; Tequila, as glTF, Y up.
Z_UP_TO_Y_UP = np.array([[1, 0, 0, 0], [0, 0, 1, 0], [0, -1, 0, 0], [0, 0, 0, 1]], dtype=np.float64)


# ---- the connection ----------------------------------------------------------

class Link:
    """One connection to the Tequila that ~/.tequila/channel.json names."""

    def __init__(self):
        path = os.path.join(os.path.expanduser("~"), ".tequila", "channel.json")
        if not os.path.exists(path):
            raise ConnectionError("Tequila is not running (no ~/.tequila/channel.json)")
        with open(path, encoding="utf-8") as f:
            info = json.load(f)
        self.token = info["token"]
        self.socket = socket.create_connection(("127.0.0.1", int(info["port"])), timeout=30)
        self.stream = self.socket.makefile("rwb")
        self.next_id = 0
        self.hello = self.request("livelink", message="hello", client="blender", version=bpy.app.version_string, protocol=PROTOCOL)

    def request(self, op, **fields):
        self.next_id += 1
        line = json.dumps(dict(fields, id=self.next_id, token=self.token, op=op), allow_nan=False).encode("utf-8") + b"\n"
        self.stream.write(line)
        self.stream.flush()
        reply = self.stream.readline()
        if not reply:
            raise ConnectionError("Tequila closed the connection")
        reply = json.loads(reply)
        if not reply.get("ok"):
            raise RuntimeError(reply.get("error", "Tequila refused the request"))
        return reply.get("result")

    def close(self):
        try:
            self.stream.close()
            self.socket.close()
        except OSError:
            pass


_link = None


def link():
    global _link
    if _link is None:
        _link = Link()
    return _link


def drop_link():
    global _link
    if _link is not None:
        _link.close()
    _link = None


def send(op, **fields):
    """A request, reconnecting once if Tequila was restarted."""
    try:
        return link().request(op, **fields)
    except (OSError, ConnectionError):
        drop_link()
        return link().request(op, **fields)


# ---- what is sent --------------------------------------------------------------

def source_name():
    path = bpy.data.filepath
    return "blender:" + (bpy.path.abspath(path) if path else "unsaved:" + bpy.context.scene.name)


def label():
    path = bpy.data.filepath
    return "Blender · " + (os.path.splitext(os.path.basename(path))[0] if path else bpy.context.scene.name)


def key_of(obj):
    """The object's stable key: its name, and its library when linked."""
    if obj.library is not None:
        return obj.library.filepath + "/" + obj.name
    return obj.name


def b64(array, dtype):
    return base64.b64encode(np.ascontiguousarray(array, dtype=dtype).tobytes()).decode("ascii")


def matrix_of(obj, scene):
    """The world matrix in Tequila's axes and metres, column-major."""
    m = np.array(obj.matrix_world, dtype=np.float64)
    scale = scene.unit_settings.scale_length or 1.0
    units = np.diag([scale, scale, scale, 1.0])
    out = units @ Z_UP_TO_Y_UP @ m
    return [float(out[r][c]) for c in range(4) for r in range(4)]


def mesh_of(obj, depsgraph):
    """The evaluated mesh, modifiers applied, one vertex per triangle
    corner: split normals and UV seams come through exactly, and Tequila
    shares the corners again."""
    evaluated = obj.evaluated_get(depsgraph)
    mesh = evaluated.to_mesh()
    try:
        mesh.calc_loop_triangles()
        count = len(mesh.loop_triangles)
        if count == 0:
            return None
        loops = np.empty(count * 3, dtype=np.int32)
        mesh.loop_triangles.foreach_get("loops", loops)
        corners = np.empty(count * 3, dtype=np.int32)
        mesh.loop_triangles.foreach_get("vertices", corners)
        co = np.empty(len(mesh.vertices) * 3, dtype=np.float32)
        mesh.vertices.foreach_get("co", co)
        positions = co.reshape(-1, 3)[corners]
        normals = np.empty(len(mesh.loops) * 3, dtype=np.float32)
        mesh.corner_normals.foreach_get("vector", normals)
        normals = normals.reshape(-1, 3)[loops]
        out = {"positions": b64(positions, "<f4"), "normals": b64(normals, "<f4")}
        layer = mesh.uv_layers.active
        if layer is not None:
            uv = np.empty(len(mesh.loops) * 2, dtype=np.float32)
            layer.data.foreach_get("uv", uv)
            uv = uv.reshape(-1, 2)[loops].copy()
            # Blender's v runs up the image; Tequila's, as glTF's, down.
            uv[:, 1] = 1.0 - uv[:, 1]
            out["uvs"] = b64(uv, "<f4")
        return out
    finally:
        evaluated.to_mesh_clear()


def socket_value(node, *names, default=None):
    for name in names:
        item = node.inputs.get(name)
        if item is not None and hasattr(item, "default_value"):
            value = item.default_value
            try:
                return [float(v) for v in value][:3]
            except TypeError:
                return float(value)
    return default


def material_of(obj):
    """The first material's Principled BSDF, as LiveLink sends it. Its
    texture inputs are not followed: the values are what it would be with
    none."""
    material = obj.active_material
    if material is None:
        return None
    out = {"name": material.name}
    principled = None
    if material.use_nodes and material.node_tree is not None:
        principled = next((n for n in material.node_tree.nodes if n.type == "BSDF_PRINCIPLED"), None)
    if principled is None:
        out["base_color"] = [float(c) for c in material.diffuse_color][:3]
        out["metallic"] = float(material.metallic)
        out["roughness"] = float(material.roughness)
        return out
    out["base_color"] = socket_value(principled, "Base Color", default=[0.8, 0.8, 0.8])
    out["metallic"] = socket_value(principled, "Metallic", default=0.0)
    out["roughness"] = socket_value(principled, "Roughness", default=0.5)
    out["ior"] = socket_value(principled, "IOR", default=1.5)
    out["alpha"] = socket_value(principled, "Alpha", default=1.0)
    out["transmission"] = socket_value(principled, "Transmission Weight", "Transmission", default=0.0)
    out["coat"] = socket_value(principled, "Coat Weight", "Clearcoat", default=0.0)
    out["coat_roughness"] = socket_value(principled, "Coat Roughness", "Clearcoat Roughness", default=0.03)
    out["sheen"] = socket_value(principled, "Sheen Weight", "Sheen", default=0.0)
    out["sheen_tint"] = socket_value(principled, "Sheen Tint", default=[1.0, 1.0, 1.0])
    if isinstance(out["sheen_tint"], float):
        out["sheen_tint"] = [out["sheen_tint"]] * 3
    out["emission"] = socket_value(principled, "Emission Color", "Emission", default=[0.0, 0.0, 0.0])
    out["emission_strength"] = socket_value(principled, "Emission Strength", default=0.0)
    return out


def shown(obj, view_layer):
    return obj.type == "MESH" and obj.visible_get(view_layer=view_layer)


def object_entry(obj, scene, depsgraph, geometry=True):
    entry = {
        "key": key_of(obj),
        "name": obj.name,
        "matrix": matrix_of(obj, scene),
        "metadata": {"Blender object": obj.name},
    }
    if obj.data is not None:
        entry["metadata"]["Blender mesh"] = obj.data.name
    if geometry:
        mesh = mesh_of(obj, depsgraph)
        if mesh is None:
            return None
        entry["mesh"] = mesh
        material = material_of(obj)
        if material is not None:
            entry["material"] = material
    return entry


# ---- syncing --------------------------------------------------------------------

_sent = set()        # keys Tequila has, from this file
_dirty = {}          # key -> needs geometry
_transaction = 0


def sync(context, objects, geometry=True, deletes=(), complete=False):
    """Send `objects`, and say which keys are gone, as one transaction.
    `complete`: these are all there are, so Tequila hides the rest."""
    global _transaction
    scene = context.scene
    depsgraph = context.evaluated_depsgraph_get()
    upserts = []
    for obj in objects:
        entry = object_entry(obj, scene, depsgraph, geometry=geometry or key_of(obj) not in _sent)
        if entry is not None:
            upserts.append(entry)
    if not upserts and not deletes and not complete:
        return None
    _transaction += 1
    result = send("livelink", message="sync", source=source_name(), label=label(), transaction=_transaction,
                  upserts=upserts, deletes=list(deletes), complete=complete)
    for entry in upserts:
        _sent.add(entry["key"])
    for key in deletes:
        _sent.discard(key)
    return result


def send_all(context, only_selected=False):
    view_layer = context.view_layer
    objects = [o for o in (context.selected_objects if only_selected else context.scene.objects) if shown(o, view_layer)]
    # A full send is complete: Tequila hides what this file sent before
    # and has no more, even across a restart of Blender.
    return sync(context, objects, geometry=True, complete=not only_selected)


def on_depsgraph(scene, depsgraph):
    settings = scene.tequila_livelink
    if not settings.auto:
        return
    for update in depsgraph.updates:
        obj = update.id
        if isinstance(obj, bpy.types.Object) and obj.type == "MESH":
            key = key_of(obj.original)
            _dirty[key] = _dirty.get(key, False) or update.is_updated_geometry or key not in _sent
        elif isinstance(obj, bpy.types.Mesh) or isinstance(obj, bpy.types.Material):
            # A mesh or material edit marks every object using it.
            for user in scene.objects:
                if user.data == getattr(obj, "original", obj) or user.active_material == getattr(obj, "original", obj):
                    _dirty[key_of(user)] = True
    if not bpy.app.timers.is_registered(flush):
        bpy.app.timers.register(flush, first_interval=0.25)


def flush():
    """Send what changed since the last flush, a quarter second after the
    last change: a drag is many updates and one sync."""
    context = bpy.context
    scene = context.scene
    if not scene.tequila_livelink.auto:
        _dirty.clear()
        return None
    pending = dict(_dirty)
    _dirty.clear()
    view_layer = context.view_layer
    by_key = {key_of(o): o for o in scene.objects}
    changed = [by_key[k] for k in pending if k in by_key and shown(by_key[k], view_layer)]
    gone = [k for k in _sent if k not in by_key or not shown(by_key[k], view_layer)]
    try:
        geometry = [o for o in changed if pending[key_of(o)]]
        moved = [o for o in changed if not pending[key_of(o)]]
        if geometry or gone:
            sync(context, geometry, geometry=True, deletes=gone)
        if moved:
            sync(context, moved, geometry=False)
        scene.tequila_livelink.status = "Synced " + time.strftime("%H:%M:%S")
    except Exception as error:  # Shown in the panel; the timer stops.
        scene.tequila_livelink.status = str(error)
        scene.tequila_livelink.auto = False
    return None


def mirror_camera():
    scene = bpy.context.scene
    settings = scene.tequila_livelink
    if not settings.camera:
        return None
    camera = scene.camera
    if camera is None:
        return 0.25
    m = np.array(camera.matrix_world, dtype=np.float64)
    scale = scene.unit_settings.scale_length or 1.0
    world = np.diag([scale, scale, scale, 1.0]) @ Z_UP_TO_Y_UP @ m
    position = world[:3, 3]
    forward = -world[:3, 2] / np.linalg.norm(world[:3, 2])
    distance = camera.data.dof.focus_distance if camera.data.dof.use_dof else 1.0
    target = position + forward * max(distance, 0.05)
    state = (tuple(np.round(position, 5)), tuple(np.round(target, 5)), round(camera.data.lens, 3))
    if state != getattr(mirror_camera, "last", None):
        mirror_camera.last = state
        try:
            send("livelink", message="camera", position=[float(v) for v in position], target=[float(v) for v in target],
                 focal_length=float(camera.data.lens), sensor=float(camera.data.sensor_width))
        except Exception as error:
            settings.status = str(error)
            settings.camera = False
            return None
    return 0.1


# ---- the panel ------------------------------------------------------------------

def on_auto(self, context):
    if self.auto:
        try:
            send_all(context)
            self.status = "Auto sync on"
        except Exception as error:
            self.status = str(error)
            self.auto = False


def on_camera(self, context):
    if self.camera and not bpy.app.timers.is_registered(mirror_camera):
        mirror_camera.last = None
        bpy.app.timers.register(mirror_camera, first_interval=0.05)


class TequilaLiveLinkSettings(bpy.types.PropertyGroup):
    auto: bpy.props.BoolProperty(name="Auto sync", description="Send each change to Tequila as you make it", update=on_auto)
    camera: bpy.props.BoolProperty(name="Mirror camera", description="Move Tequila's view with the scene camera", update=on_camera)
    status: bpy.props.StringProperty(name="Status", default="")


class TEQUILA_OT_send(bpy.types.Operator):
    """Send the visible meshes to Tequila"""
    bl_idname = "tequila.send"
    bl_label = "Send to Tequila"
    only_selected: bpy.props.BoolProperty(name="Only selected", default=False)

    def execute(self, context):
        try:
            result = send_all(context, self.only_selected)
        except Exception as error:
            self.report({"ERROR"}, "Tequila: " + str(error))
            context.scene.tequila_livelink.status = str(error)
            return {"CANCELLED"}
        if result is None:
            self.report({"INFO"}, "Nothing to send")
        else:
            message = "Tequila: {matched} updated, {added} added, {removed} hidden".format(**{k: int(v) for k, v in result.items() if k in ("matched", "added", "removed")})
            self.report({"INFO"}, message)
            context.scene.tequila_livelink.status = message
        return {"FINISHED"}


class TEQUILA_PT_livelink(bpy.types.Panel):
    bl_label = "Tequila LiveLink"
    bl_space_type = "VIEW_3D"
    bl_region_type = "UI"
    bl_category = "Tequila"

    def draw(self, context):
        settings = context.scene.tequila_livelink
        layout = self.layout
        layout.operator("tequila.send", icon="EXPORT").only_selected = False
        layout.operator("tequila.send", text="Send selected").only_selected = True
        layout.prop(settings, "auto")
        layout.prop(settings, "camera")
        if settings.status:
            layout.label(text=settings.status)


classes = (TequilaLiveLinkSettings, TEQUILA_OT_send, TEQUILA_PT_livelink)


def register():
    for cls in classes:
        bpy.utils.register_class(cls)
    bpy.types.Scene.tequila_livelink = bpy.props.PointerProperty(type=TequilaLiveLinkSettings)
    if on_depsgraph not in bpy.app.handlers.depsgraph_update_post:
        bpy.app.handlers.depsgraph_update_post.append(on_depsgraph)


def unregister():
    if on_depsgraph in bpy.app.handlers.depsgraph_update_post:
        bpy.app.handlers.depsgraph_update_post.remove(on_depsgraph)
    for timer in (flush, mirror_camera):
        if bpy.app.timers.is_registered(timer):
            bpy.app.timers.unregister(timer)
    drop_link()
    del bpy.types.Scene.tequila_livelink
    for cls in reversed(classes):
        bpy.utils.unregister_class(cls)
