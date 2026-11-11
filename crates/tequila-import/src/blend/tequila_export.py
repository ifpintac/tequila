# What Tequila reads out of a Blender file.
#
# Tequila runs this inside Blender, in the background, on a copy of the
# scene in memory that is never saved:
#
#   blender --background --factory-startup --disable-autoexec model.blend \
#           --python-exit-code 1 --python tequila_export.py -- <folder>
#
# and reads back two files from <folder>: scene.json, which says what is in
# the scene, and scene.bin, the meshes' arrays it points into. Blender does
# the work only Blender can do - modifiers applied, curves and text made
# into meshes, instances placed, packed images unpacked - and this writes
# down what it finds as it finds it: node trees are written node for node,
# not translated, so the translation is Tequila's and can be tested without
# Blender (crates/tequila-import/src/blend.rs).
#
# Nothing in the file runs: --disable-autoexec keeps its scripts and
# drivers' Python off. Nothing is sent anywhere and nothing is saved.
#
# SPDX-License-Identifier: Apache-2.0

import json
import math
import os
import sys

import bpy
import numpy as np

FORMAT = "tequila-blend"
VERSION = 1
# The most instances placed: a scatter of a million pebbles is a warning,
# not a million parts.
MOST_INSTANCES = 200000


class Arrays:
    """scene.bin: every array one after another, little-endian, each at a
    multiple of 4 bytes."""

    def __init__(self, path):
        self.file = open(path, "wb")
        self.offset = 0

    def add(self, array, dtype):
        data = np.ascontiguousarray(array, dtype=dtype).tobytes()
        at = self.offset
        self.file.write(data)
        self.offset += len(data)
        return [at, len(data)]

    def close(self):
        self.file.close()


def matrix_rows(m):
    return [float(m[r][c]) for r in range(4) for c in range(4)]


def value_of(socket):
    """A socket's own value, as JSON: a number, a list, a string, or None
    for a socket that has none (a shader)."""
    if not hasattr(socket, "default_value"):
        return None
    value = socket.default_value
    if isinstance(value, (bool, int, float)):
        return value if not isinstance(value, float) or math.isfinite(value) else 0.0
    if isinstance(value, str):
        return value
    try:
        return [float(v) for v in value]
    except TypeError:
        # A menu socket, an object, an image: named, if it has a name.
        return getattr(value, "name", None)


# Properties every node has, which say nothing about what it does.
COMMON = {
    "rna_type", "type", "location", "location_absolute", "width", "width_hidden", "height", "dimensions",
    "name", "label", "inputs", "outputs", "internal_links", "parent", "select", "show_options",
    "show_preview", "hide", "mute", "show_texture", "bl_idname", "bl_label", "bl_description", "bl_icon",
    "bl_static_type", "bl_width_default", "bl_width_min", "bl_width_max", "bl_height_default",
    "bl_height_min", "bl_height_max", "use_custom_color", "color", "color_tag", "warning_propagation",
    "node_tree", "image", "image_user", "color_ramp", "mapping", "texture_mapping", "color_mapping",
    "object", "script", "filepath", "bytecode", "bytecode_hash", "ies", "parent", "is_active_output",
    "debug_zone_body_lazy_function_graph", "debug_zone_lazy_function_graph",
}


def settings_of(node):
    """A node's settings that are not sockets: its menus, switches and
    numbers, by their Python names."""
    out = {}
    for prop in node.bl_rna.properties:
        key = prop.identifier
        if key in COMMON or prop.type in {"POINTER", "COLLECTION"}:
            continue
        try:
            value = getattr(node, key)
        except AttributeError:
            continue
        if isinstance(value, (bool, int, str)):
            out[key] = value
        elif isinstance(value, float):
            out[key] = value if math.isfinite(value) else 0.0
        elif isinstance(value, set):
            out[key] = sorted(value)
        else:
            try:
                out[key] = [float(v) for v in value]
            except TypeError:
                pass
    return out


def ramp_of(ramp):
    return {
        "interpolation": ramp.interpolation,
        "color_mode": ramp.color_mode,
        "hue_interpolation": ramp.hue_interpolation,
        "stops": [[float(e.position), [float(c) for c in e.color]] for e in ramp.elements],
    }


def curves_of(mapping, count=33):
    """A curve mapping read at `count` even steps across its range: enough
    to rebuild it as a ramp, whatever kind of curve it is."""
    try:
        mapping.initialize()
    except Exception:
        pass
    low, high = mapping.clip_min_x, mapping.clip_max_x
    if not mapping.use_clip or high <= low:
        low, high = 0.0, 1.0
    out = {"range": [float(low), float(high)], "curves": []}
    for curve in mapping.curves:
        samples = []
        for i in range(count):
            x = low + (high - low) * i / (count - 1)
            try:
                samples.append(float(mapping.evaluate(curve, x)))
            except Exception:
                samples.append(x)
        out["curves"].append(samples)
    return out


class Export:
    def __init__(self, folder):
        self.folder = folder
        self.arrays = Arrays(os.path.join(folder, "scene.bin"))
        self.warnings = []
        self.images = []
        self.image_index = {}
        self.trees = []
        self.tree_index = {}
        self.materials = []
        self.material_index = {}
        self.meshes = []
        self.mesh_index = {}
        self.nodes = []

    # ---- images ---------------------------------------------------------

    def image(self, image):
        if image is None:
            return None
        key = image.name_full
        if key in self.image_index:
            return self.image_index[key]
        entry = {
            "name": image.name,
            "source": image.source,
            "colorspace": image.colorspace_settings.name,
            "alpha_mode": image.alpha_mode,
            "channels": int(image.channels),
            "size": [int(image.size[0]), int(image.size[1])],
            "path": None,
            "file": None,
        }
        path = bpy.path.abspath(image.filepath, library=image.library) if image.filepath else ""
        if image.source == "TILED" and "<UDIM>" in path:
            self.warnings.append(f"the image {image.name} is tiled (UDIM); only its first tile, 1001, is read")
            path = path.replace("<UDIM>", "1001")
        if image.packed_file is not None:
            # Packed in the .blend: written out as the file it was.
            name = os.path.basename(path) if path else ""
            if not name or "." not in name:
                name = f"{image.name}.{(image.file_format or 'png').lower()}"
            name = f"{len(self.images):03d}-{bpy.path.clean_name(os.path.splitext(name)[0])}{os.path.splitext(name)[1].lower()}"
            with open(os.path.join(self.folder, name), "wb") as f:
                f.write(image.packed_file.data)
            entry["file"] = name
        elif path and os.path.isfile(path):
            entry["path"] = os.path.normpath(path)
        elif image.has_data or image.source == "GENERATED":
            # Made in Blender, or painted and not saved: written out as a PNG.
            name = f"{len(self.images):03d}-{bpy.path.clean_name(image.name)}.png"
            try:
                copy = image.copy()
                copy.filepath_raw = os.path.join(self.folder, name)
                copy.file_format = "PNG"
                copy.save()
                entry["file"] = name
            except Exception as e:
                self.warnings.append(f"the image {image.name} could not be written out ({e}) and is left out")
        else:
            self.warnings.append(f"the image {image.name} is not where the file says ({path or 'no path'}) and is left out")
            entry["missing"] = path
        self.image_index[key] = len(self.images)
        self.images.append(entry)
        return self.image_index[key]

    # ---- node trees -----------------------------------------------------

    def tree(self, tree):
        """A node tree, node for node, and the groups it uses."""
        # By the tree itself: every material's own tree is named "Shader
        # Nodetree".
        key = tree.as_pointer()
        if key in self.tree_index:
            return self.tree_index[key]
        index = len(self.trees)
        self.tree_index[key] = index
        self.trees.append(None)
        nodes = []
        for node in tree.nodes:
            entry = {
                "name": node.name,
                "type": node.bl_idname,
                "label": node.label,
                "mute": bool(node.mute),
                "settings": settings_of(node),
                "inputs": [
                    {"id": s.identifier, "name": s.name, "type": s.type, "enabled": bool(s.enabled),
                     "linked": bool(s.is_linked), "value": value_of(s)}
                    for s in node.inputs
                ],
                # An output's own value is what a Value or RGB node holds.
                "outputs": [
                    {"id": s.identifier, "name": s.name, "type": s.type, "enabled": bool(s.enabled), "linked": bool(s.is_linked),
                     "value": value_of(s)}
                    for s in node.outputs
                ],
            }
            if node.mute:
                entry["internal_links"] = [[l.from_socket.identifier, l.to_socket.identifier] for l in node.internal_links]
            if getattr(node, "image", None) is not None and node.bl_idname in {"ShaderNodeTexImage", "ShaderNodeTexEnvironment"}:
                entry["image"] = self.image(node.image)
            if node.bl_idname == "ShaderNodeValToRGB":
                entry["ramp"] = ramp_of(node.color_ramp)
            if node.bl_idname in {"ShaderNodeRGBCurve", "ShaderNodeVectorCurve", "ShaderNodeFloatCurve"}:
                entry["curves"] = curves_of(node.mapping)
            if node.bl_idname == "ShaderNodeGroup" and node.node_tree is not None:
                entry["group"] = self.tree(node.node_tree)
            if node.bl_idname in {"ShaderNodeOutputMaterial", "ShaderNodeOutputWorld", "NodeGroupOutput"}:
                entry["active"] = bool(getattr(node, "is_active_output", True))
            if node.bl_idname == "ShaderNodeTexCoord" and getattr(node, "object", None) is not None:
                entry["object"] = node.object.name
            if getattr(node, "texture_mapping", None) is not None:
                tm = node.texture_mapping
                entry["texture_mapping"] = {
                    "translation": list(tm.translation), "rotation": list(tm.rotation), "scale": list(tm.scale),
                    "default": all(abs(v) < 1e-9 for v in list(tm.translation) + list(tm.rotation)) and all(abs(v - 1) < 1e-9 for v in tm.scale)
                    and tm.mapping_x == "X" and tm.mapping_y == "Y" and tm.mapping_z == "Z",
                }
            nodes.append(entry)
        position = {node.name: i for i, node in enumerate(tree.nodes)}
        links = []
        for link in tree.links:
            if not link.is_valid or getattr(link, "is_muted", False):
                continue
            links.append([position[link.from_node.name], link.from_socket.identifier,
                          position[link.to_node.name], link.to_socket.identifier])
        interface = []
        if getattr(tree, "interface", None) is not None:
            for item in tree.interface.items_tree:
                if getattr(item, "item_type", "") == "SOCKET":
                    interface.append({"id": item.identifier, "name": item.name, "in_out": item.in_out,
                                      "type": getattr(item, "socket_type", ""),
                                      "default": value_of(item) if hasattr(item, "default_value") else None})
        self.trees[index] = {"name": tree.name, "nodes": nodes, "links": links, "interface": interface}
        return index

    # ---- materials ------------------------------------------------------

    def material(self, material):
        if material is None:
            return None
        key = material.name_full
        if key in self.material_index:
            return self.material_index[key]
        entry = {
            "name": material.name,
            "color": [float(c) for c in material.diffuse_color],
            "metallic": float(material.metallic),
            "roughness": float(material.roughness),
            "tree": None,
            "backface_culling": bool(getattr(material, "use_backface_culling", False)),
        }
        # Blender 5 always uses nodes and is retiring the switch.
        uses_nodes = bpy.app.version >= (5, 0, 0) or material.use_nodes
        if uses_nodes and material.node_tree is not None:
            entry["tree"] = self.tree(material.node_tree)
        self.material_index[key] = len(self.materials)
        self.materials.append(entry)
        return self.material_index[key]

    # ---- meshes ---------------------------------------------------------

    def mesh(self, key, name, evaluated):
        """The evaluated geometry of an object, once for every object that
        shares it unchanged."""
        if key in self.mesh_index:
            return self.mesh_index[key]
        try:
            mesh = evaluated.to_mesh()
        except RuntimeError:
            mesh = None
        if mesh is None:
            self.mesh_index[key] = None
            return None
        try:
            mesh.calc_loop_triangles()
            count = len(mesh.loop_triangles)
            if count == 0:
                self.mesh_index[key] = None
                return None
            loops = np.empty(count * 3, dtype=np.int32)
            mesh.loop_triangles.foreach_get("loops", loops)
            corners = np.empty(count * 3, dtype=np.int32)
            mesh.loop_triangles.foreach_get("vertices", corners)
            slots = np.empty(count, dtype=np.int32)
            mesh.loop_triangles.foreach_get("material_index", slots)
            co = np.empty(len(mesh.vertices) * 3, dtype=np.float32)
            mesh.vertices.foreach_get("co", co)
            normals = np.empty(len(mesh.loops) * 3, dtype=np.float32)
            mesh.corner_normals.foreach_get("vector", normals)
            entry = {
                "name": name,
                "triangles": int(count),
                "positions": self.arrays.add(co.reshape(-1, 3)[corners], "<f4"),
                "normals": self.arrays.add(normals.reshape(-1, 3)[loops], "<f4"),
                "slots": self.arrays.add(slots, "<i4"),
                "uvs": [],
                "uv_names": [],
            }
            # The UV map a render uses first, then the rest.
            layers = list(mesh.uv_layers)
            layers.sort(key=lambda layer: not getattr(layer, "active_render", False))
            for layer in layers[:4]:
                uv = np.empty(len(mesh.loops) * 2, dtype=np.float32)
                layer.data.foreach_get("uv", uv)
                entry["uvs"].append(self.arrays.add(uv.reshape(-1, 2)[loops], "<f4"))
                entry["uv_names"].append(layer.name)
            colours = getattr(mesh, "color_attributes", None)
            attribute = None
            if colours is not None and len(colours):
                attribute = colours[colours.render_color_index] if 0 <= colours.render_color_index < len(colours) else colours[0]
            if attribute is not None and attribute.domain in {"POINT", "CORNER"}:
                rgba = np.empty(len(attribute.data) * 4, dtype=np.float32)
                attribute.data.foreach_get("color", rgba)
                rgba = rgba.reshape(-1, 4)
                rgba = rgba[corners] if attribute.domain == "POINT" else rgba[loops]
                entry["colors"] = self.arrays.add(rgba, "<f4")
                entry["color_name"] = attribute.name
            self.mesh_index[key] = len(self.meshes)
            self.meshes.append(entry)
            return self.mesh_index[key]
        finally:
            evaluated.to_mesh_clear()

    # ---- the scene ------------------------------------------------------

    def node(self, **fields):
        self.nodes.append(fields)
        return len(self.nodes) - 1


def camera_of(obj, scene):
    data = obj.data
    render = scene.render
    width = render.resolution_x * render.pixel_aspect_x
    height = render.resolution_y * render.pixel_aspect_y
    fit = data.sensor_fit
    if fit == "AUTO":
        fit = "HORIZONTAL" if width >= height else "VERTICAL"
    # The sensor across the picture, which is how Tequila measures it.
    if fit == "HORIZONTAL":
        sensor = data.sensor_width
    else:
        sensor = (data.sensor_height if data.sensor_fit == "VERTICAL" else data.sensor_width) * width / max(height, 1e-6)
    out = {
        "name": obj.name,
        "matrix": matrix_rows(obj.matrix_world),
        "type": data.type,
        "lens": float(data.lens),
        "sensor": float(sensor),
        "ortho_scale": float(data.ortho_scale),
        "ortho_across": "HORIZONTAL" if fit == "HORIZONTAL" else "VERTICAL",
        "shift": [float(data.shift_x), float(data.shift_y)],
        "dof": bool(data.dof.use_dof),
        "fstop": float(data.dof.aperture_fstop),
        "blades": int(data.dof.aperture_blades),
        "blade_rotation": float(data.dof.aperture_rotation),
        "focus": float(data.dof.focus_distance),
    }
    if data.dof.focus_object is not None:
        target = data.dof.focus_object.matrix_world.translation
        out["focus"] = float((obj.matrix_world.inverted() @ target).length)
    return out


def world_of(scene, export):
    world = scene.world
    if world is None:
        return None
    out = {"name": world.name, "color": [float(c) for c in world.color], "tree": None}
    if (bpy.app.version >= (5, 0, 0) or world.use_nodes) and world.node_tree is not None:
        out["tree"] = export.tree(world.node_tree)
    return out


def main(folder):
    if bpy.app.version < (4, 2, 0):
        raise SystemExit(f"Blender {bpy.app.version_string} is too old for Tequila to read its files: 4.2 or later is needed")
    if not bpy.data.filepath:
        # Blender carries on with its own start-up scene when it cannot read
        # a file; that scene is not the file.
        raise SystemExit("Blender could not open the file")
    scene = bpy.context.scene
    view_layer = bpy.context.view_layer
    export = Export(folder)

    others = [s.name for s in bpy.data.scenes if s != scene]
    if others:
        export.warnings.append(f"the file has {len(others) + 1} scenes; Tequila read {scene.name}, the one it was saved in")

    # What renders, before anything is shown: Blender's render leaves out
    # an object switched off for rendering and one whose collections are
    # all excluded or switched off.
    rendered = set()

    def walk(layer, shown):
        shown = shown and not layer.exclude and not layer.collection.hide_render
        if shown:
            rendered.add(layer.collection.name_full)
        for child in layer.children:
            walk(child, shown)

    walk(view_layer.layer_collection, True)
    renders = {obj.name_full: (not obj.hide_render) and any(c.name_full in rendered for c in obj.users_collection) for obj in scene.objects}
    collection_hidden = {}

    def note(layer):
        collection_hidden[layer.collection.name_full] = layer.exclude or layer.collection.hide_render
        for child in layer.children:
            note(child)

    note(view_layer.layer_collection)

    # Everything into the evaluated scene, hidden or not, so a hidden part
    # arrives hidden rather than not at all. This is a copy in memory.
    def show(layer):
        layer.exclude = False
        layer.hide_viewport = False
        layer.collection.hide_viewport = False
        for child in layer.children:
            show(child)

    show(view_layer.layer_collection)
    for obj in scene.objects:
        obj.hide_viewport = False
        try:
            obj.hide_set(False)
        except RuntimeError:
            pass
    view_layer.update()
    depsgraph = bpy.context.evaluated_depsgraph_get()

    geometry = {"MESH", "CURVE", "SURFACE", "META", "FONT", "CURVES"}
    left_out = {}
    cameras = []
    lights = 0
    object_node = {}

    # The collections, as groups.
    collection_node = {}

    def collections(layer, parent):
        c = layer.collection
        index = export.node(kind="group", name=c.name if parent is not None else scene.name, parent=parent,
                            matrix=None, hidden=collection_hidden.get(c.name_full, False) if parent is not None else False,
                            metadata={"Blender collection": c.name} if parent is not None else {"Blender scene": scene.name})
        collection_node[c.name_full] = index
        for child in layer.children:
            collections(child, index)

    collections(view_layer.layer_collection, None)

    def home(obj):
        """The collection an object is listed under: its first one in this
        scene."""
        for c in obj.users_collection:
            if c.name_full in collection_node:
                return collection_node[c.name_full]
        return collection_node[view_layer.layer_collection.collection.name_full]

    def needs_own_mesh(obj):
        return bool(getattr(obj, "modifiers", None) and len(obj.modifiers)) or obj.type != "MESH" or \
            (obj.type == "MESH" and obj.data.shape_keys is not None)

    # Parents before children.
    ordered = []
    seen = set()

    def visit(obj):
        if obj.name_full in seen:
            return
        if obj.parent is not None and obj.parent.name_full in {o.name_full for o in scene.objects}:
            visit(obj.parent)
        seen.add(obj.name_full)
        ordered.append(obj)

    for obj in scene.objects:
        visit(obj)

    for obj in ordered:
        if obj.type == "CAMERA":
            cameras.append(camera_of(obj, scene))
        if obj.type == "LIGHT":
            lights += 1
        parent = object_node.get(obj.parent.name_full) if obj.parent is not None else None
        if parent is None:
            parent_index, matrix = home(obj), obj.matrix_world
        else:
            parent_index, matrix = parent, obj.parent.matrix_world.inverted_safe() @ obj.matrix_world
        mesh = None
        materials = []
        if obj.type in geometry:
            evaluated = obj.evaluated_get(depsgraph)
            key = ("object", obj.name_full) if needs_own_mesh(obj) else ("mesh", obj.data.name_full)
            mesh = export.mesh(key, obj.name, evaluated)
            materials = [export.material(slot.material) for slot in obj.material_slots]
        elif obj.type not in {"EMPTY", "CAMERA", "LIGHT", "ARMATURE", "LIGHT_PROBE", "SPEAKER"}:
            left_out[obj.type] = left_out.get(obj.type, 0) + 1
        metadata = {"Blender object": obj.name}
        if obj.data is not None and hasattr(obj.data, "name"):
            metadata["Blender data"] = obj.data.name
        if obj.library is not None:
            metadata["Blender library"] = obj.library.filepath
        # Cameras and lights stay out of the tree: a camera is a camera in
        # Tequila, and a light is named in the warnings.
        if obj.type in {"CAMERA", "LIGHT", "LIGHT_PROBE", "SPEAKER"} and not obj.children:
            continue
        object_node[obj.name_full] = export.node(
            kind="object", name=obj.name, parent=parent_index, matrix=matrix_rows(matrix),
            hidden=not renders.get(obj.name_full, True), mesh=mesh, materials=materials,
            metadata=metadata, type=obj.type)

    # Instances: collection instances, particles, geometry nodes' instances,
    # each placed under what made it.
    placed = 0
    for instance in depsgraph.object_instances:
        if not instance.is_instance:
            continue
        maker = instance.parent.original if instance.parent is not None else None
        if maker is None or maker.name_full not in object_node:
            continue
        if placed >= MOST_INSTANCES:
            export.warnings.append(f"more than {MOST_INSTANCES} instances; the rest are left out")
            break
        source = instance.object
        if source.type not in geometry:
            continue
        original = instance.instance_object.original if instance.instance_object is not None else None
        # A curve's or text's own mesh is listed as an instance of itself;
        # it is the object, already written.
        if original is not None and original == maker:
            continue
        if original is not None and source.type == "MESH" and not needs_own_mesh(original):
            key = ("mesh", original.data.name_full)
        else:
            key = ("instance", source.data.name_full if source.data is not None else source.name_full)
        mesh = export.mesh(key, (original or source).name, source)
        if mesh is None:
            continue
        materials = [export.material(slot.material) for slot in (original or source).material_slots] \
            or [export.material(m) for m in getattr(source.data, "materials", [])]
        local = maker.matrix_world.inverted_safe() @ instance.matrix_world
        export.node(kind="object", name=(original or source).name, parent=object_node[maker.name_full],
                    matrix=matrix_rows(local), hidden=not renders.get(maker.name_full, True), mesh=mesh,
                    materials=materials, metadata={"Blender object": (original or source).name, "Blender instance of": maker.name},
                    type="INSTANCE")
        placed += 1

    if lights:
        export.warnings.append(f"{lights} light{'s' if lights != 1 else ''} left out: Tequila lights a model with its studios")
    for kind, count in sorted(left_out.items()):
        export.warnings.append(f"{count} {kind.lower().replace('_', ' ')} object{'s' if count != 1 else ''} left out")

    view = scene.view_settings
    render = scene.render
    out = {
        "format": FORMAT,
        "version": VERSION,
        "blender": bpy.app.version_string,
        "scene": scene.name,
        "unit_scale": float(scene.unit_settings.scale_length or 1.0),
        "nodes": export.nodes,
        "meshes": export.meshes,
        "materials": export.materials,
        "trees": export.trees,
        "images": export.images,
        "cameras": cameras,
        "camera": scene.camera.name if scene.camera is not None else None,
        "world": world_of(scene, export),
        "render": {
            "width": int(render.resolution_x * render.resolution_percentage / 100),
            "height": int(render.resolution_y * render.resolution_percentage / 100),
            "transparent": bool(render.film_transparent),
            "view_transform": view.view_transform,
            "look": view.look,
            "exposure": float(view.exposure),
            "gamma": float(view.gamma),
            "engine": render.engine,
        },
        "warnings": export.warnings,
    }
    export.arrays.close()
    with open(os.path.join(folder, "scene.json"), "w", encoding="utf-8") as f:
        json.dump(out, f, allow_nan=False)


if __name__ == "__main__":
    arguments = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    if not arguments:
        raise SystemExit("usage: blender --background file.blend --python tequila_export.py -- <folder>")
    main(arguments[0])
