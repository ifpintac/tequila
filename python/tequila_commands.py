"""Every command in Tequila's catalog, as a Python call.

Generated from the build's command catalog by
`cargo run -p tequila-scripting --example bindings`. Do not edit: a test
fails when this falls behind the build.

Each call applies one command through the bus, as the same click would:
one undo step, journalled, validated. A field left out is left out of the
command, and Tequila says so if the command needs it.

    import tequila
    with tequila.open('kettle.tequila') as fs:
        fs.commands.material.set_type(material=2, type='velvet')
"""

CATALOG = (
    'material.add_decal',
    'material.set_decals',
    'scene.import',
    'scene.reimport',
    'scene.add_group',
    'scene.delete_nodes',
    'scene.split_parts',
    'scene.rename',
    'scene.set_transform',
    'scene.set_visibility',
    'scene.set_locked',
    'scene.set_pivot',
    'scene.reparent',
    'scene.group',
    'scene.ungroup',
    'scene.set_metadata',
    'scene.set_units',
    'material.assign',
    'material.unlink',
    'material.set_parameter',
    'material.rename',
    'material.set_graph',
    'material.set_type',
    'material.set_texture',
    'material.set_procedural',
    'material.set_gradient',
    'material.set_mapping',
    'lighting.set_preset',
    'lighting.set_custom',
    'environment.set_parameter',
    'environment.set_pins',
    'environment.set_sky',
    'scene.add_light',
    'scene.set_light',
    'scene.add_cutaway',
    'scene.set_cutaway',
    'scene.set_measurements',
    'material.set_multi_materials',
    'scene.set_model_sets',
    'studio.set',
    'studio.apply',
    'configurator.set',
    'configurator.choose',
    'material.set_templates',
    'camera.add',
    'camera.remove',
    'camera.select',
    'camera.set',
    'camera.rename',
    'image.set_style',
    'image.set_color_management',
    'render.set_settings',
)

_UNSET = object()


class _Commands:
    def __init__(self, client):
        self._client = client

    def _apply(self, command, fields):
        return self._client.apply(command, **{k: v for k, v in fields.items() if v is not _UNSET})


class MaterialCommands(_Commands):
    """The `material.*` commands."""

    def add_decal(self, *, material=_UNSET, name=_UNSET, path=_UNSET, role=_UNSET):
        r"""`material.add_decal`: A label, an image projected onto a material. PNG, JPEG or Photoshop artwork; or a video (MP4, MOV and the rest the system plays), sent with `time` in seconds, whose frame then is the picture (kept under "Video frames" among Tequila's settings).

        { "command": "material.add_decal", "material": 1, "name": "Logo", "path": "logo.png", "role": "color" }
        """
        return self._apply('material.add_decal', {'material': material, 'name': name, 'path': path, 'role': role})

    def set_decals(self, *, decals=_UNSET, material=_UNSET):
        r"""`material.set_decals`: A material's whole stack of labels at once.

        { "command": "material.set_decals", "decals": [ { "enabled": true, "mapping": { "axis": "z", "blend_sharpness": 6, "flip_u": false, "flip_v": false, "offset": [0, 0], "pivot": [0, 0], "projection": "uv", "real_world_units": false, "rotation": 0, "scale": [1, 1, 1], "uv_set": 0, "wrap_u": "repeat", "wrap_v": "repeat" }, "metallic": false, "name": "Logo", "opacity": 1, "roughness": 0.35, "texture": 1 } ], "material": 1 }
        """
        return self._apply('material.set_decals', {'decals': decals, 'material': material})

    def assign(self, *, material=_UNSET, nodes=_UNSET):
        r"""`material.assign`: One material on these nodes, linked: editing it changes every part it is on.

        { "command": "material.assign", "material": 1, "nodes": [2] }
        """
        return self._apply('material.assign', {'material': material, 'nodes': nodes})

    def unlink(self, *, node=_UNSET):
        r"""`material.unlink`: A node given its own copy of its material, so it can be edited alone.

        { "command": "material.unlink", "node": 2 }
        """
        return self._apply('material.unlink', {'node': node})

    def set_parameter(self, *, material=_UNSET, parameter=_UNSET, value=_UNSET):
        r"""`material.set_parameter`: One parameter of a material, by its key (`base_color`, `specular_roughness`, ...).

        { "command": "material.set_parameter", "material": 1, "parameter": "base_color", "value": [0.2, 0.4, 0.8] }
        """
        return self._apply('material.set_parameter', {'material': material, 'parameter': parameter, 'value': value})

    def rename(self, *, material=_UNSET, name=_UNSET):
        r"""`material.rename`: A material's name.

        { "command": "material.rename", "material": 1, "name": "Anodized" }
        """
        return self._apply('material.rename', {'material': material, 'name': name})

    def set_graph(self, *, definition=_UNSET, material=_UNSET):
        r"""`material.set_graph`: The material's whole node graph, replaced.

        { "command": "material.set_graph", "definition": { "base_color": [0.5, 0.5, 0.5], "base_diffuse_roughness": 0, "base_metalness": 0, "base_weight": 1, "coat_color": [1, 1, 1], "coat_darkening": 1, "coat_ior": 1.5, "coat_roughness": 0.02, "coat_weight": 0, "displacement_detail": 0.0005, "edge_radius": 0, "edge_samples": 8, "emission_color": [1, 1, 1], "emission_luminance": 0, "flake_color": [0.9, 0.9, 0.9], "flake_roughness": 0.3, "flake_size": 0.00005, "flake_spread": 0.3, "flake_weight": 0, "fuzz_color": [1, 1, 1], "fuzz_roughness": 0.3, "fuzz_weight": 0, "geometry_occlusion": 1, "geometry_opacity": 1, "geometry_thin_walled": false, "graph": { "decals": [], "links": [ { "from": 2, "socket": "param:specular_roughness", "to": 1 } ], "nodes": [ { "id": 2, "kind": "constant", "position": [0, 0], "value": { "float": 0.35 } } ], "output": 1 }, "id": "mat_00000001", "medium_priority": 0, "metal_ior": { "k": [7.475, 6.4, 5.303], "n": [1.345, 0.965, 0.617] }, "name": "graph", "specular_color": [1, 1, 1], "specular_ior": 1.5, "specular_rotation": 0, "specular_roughness": 0.2, "specular_roughness_anisotropy": 0, "specular_weight": 1, "subsurface_color": [0.8, 0.8, 0.8], "subsurface_radius": [1, 1, 1], "subsurface_scatter_anisotropy": 0, "subsurface_weight": 0, "thin_film_ior": 1.4, "thin_film_thickness": 500, "thin_film_weight": 0, "transmission_color": [1, 1, 1], "transmission_depth": 0, "transmission_dispersion_abbe": 0, "transmission_scatter": [0, 0, 0], "transmission_scatter_anisotropy": 0, "transmission_tint": [1, 1, 1], "transmission_weight": 0, "type": "diffuse", "volume_density": 1, "volume_emission_color": [1, 1, 1], "volume_emission_strength": 0 }, "material": 1 }
        """
        return self._apply('material.set_graph', {'definition': definition, 'material': material})

    def set_type(self, *, material=_UNSET, type=_UNSET):
        r"""`material.set_type`: A material's type (plastic, metal, glass, velvet, ...), keeping what carries over.

        { "command": "material.set_type", "material": 1, "type": "glass_solid" }
        """
        return self._apply('material.set_type', {'material': material, 'type': type})

    def set_texture(self, *, color_space=_UNSET, material=_UNSET, param=_UNSET, path=_UNSET, role=_UNSET):
        r"""`material.set_texture`: Drive a parameter from a texture, or stop driving it. This is §4.2's "enabling the texture toggle next to a parameter inserts a Texture Map node connected to that input" — one command, because from the user's side it is one action and has to undo as one.

        { "color_space": "srgb", "command": "material.set_texture", "material": 1, "param": "base_color", "path": "maps/panel.png", "role": "color" }
        """
        return self._apply('material.set_texture', {'color_space': color_space, 'material': material, 'param': param, 'path': path, 'role': role})

    def set_procedural(self, *, material=_UNSET, param=_UNSET, procedural=_UNSET, seed=_UNSET):
        r"""`material.set_procedural`: Drive a parameter from a procedural pattern rather than from a file. `None` detaches, leaving the literal, exactly as a texture does.

        { "command": "material.set_procedural", "material": 1, "param": "base_color", "procedural": { "basis": "perlin", "frequency": 4, "gain": 0.5, "kind": "noise", "lacunarity": 2, "octaves": 4 }, "seed": 7 }
        """
        return self._apply('material.set_procedural', {'material': material, 'param': param, 'procedural': procedural, 'seed': seed})

    def set_gradient(self, *, gradient=_UNSET, material=_UNSET, param=_UNSET):
        r"""`material.set_gradient`: The colour ramp a parameter's procedural is read through.

        { "command": "material.set_gradient", "gradient": [ { "at": 0, "color": [0.05, 0.05, 0.05] }, { "at": 1, "color": [0.8, 0.8, 0.8] } ], "material": 1, "param": "base_color" }
        """
        return self._apply('material.set_gradient', {'gradient': gradient, 'material': material, 'param': param})

    def set_mapping(self, *, axis=_UNSET, blend_sharpness=_UNSET, flip_u=_UNSET, flip_v=_UNSET, material=_UNSET, offset=_UNSET, param=_UNSET, pivot=_UNSET, projection=_UNSET, real_world_units=_UNSET, rotation=_UNSET, scale=_UNSET, uv_set=_UNSET, wrap_u=_UNSET, wrap_v=_UNSET):
        r"""`material.set_mapping`: How a parameter's texture is laid onto the surface: the projection, the scale, the rotation and the rest of §5.3's shared controls.

        { "axis": "z", "blend_sharpness": 6, "command": "material.set_mapping", "flip_u": true, "flip_v": false, "material": 1, "offset": [0, 0], "param": "base_color", "pivot": [0, 0], "projection": "uv", "real_world_units": false, "rotation": 0.5, "scale": [1, 1, 1], "uv_set": 1, "wrap_u": "repeat", "wrap_v": "repeat" }
        """
        return self._apply('material.set_mapping', {'axis': axis, 'blend_sharpness': blend_sharpness, 'flip_u': flip_u, 'flip_v': flip_v, 'material': material, 'offset': offset, 'param': param, 'pivot': pivot, 'projection': projection, 'real_world_units': real_world_units, 'rotation': rotation, 'scale': scale, 'uv_set': uv_set, 'wrap_u': wrap_u, 'wrap_v': wrap_v})

    def set_multi_materials(self, *, multi_materials=_UNSET):
        r"""`material.set_multi_materials`: The whole list, and the active variant of each assigned to its parts.

        { "command": "material.set_multi_materials", "multi_materials": [ { "active": 0, "id": 7, "name": "Colourways", "parts": [2], "variants": [ "mat_00000001" ] } ] }
        """
        return self._apply('material.set_multi_materials', {'multi_materials': multi_materials})

    def set_templates(self, *, templates=_UNSET):
        r"""`material.set_templates`: The list of Material Templates. Applying one is ordinary assignments, grouped.

        { "command": "material.set_templates", "templates": [ { "name": "Fasteners", "rules": [ { "match": "name", "material": "mat_00000001", "pattern": "bolt*" }, { "color": [0.25, 0.25, 0.25], "match": "colour", "material": "mat_00000001", "tolerance": 0.05 }, { "key": "layer", "match": "metadata", "material": "mat_00000001", "pattern": "HARDWARE" }, { "match": "material", "material": "mat_00000001", "pattern": "Steel*" } ] } ] }
        """
        return self._apply('material.set_templates', {'templates': templates})


class SceneCommands(_Commands):
    """The `scene.*` commands."""

    def import_(self, *, payload=_UNSET):
        r"""`scene.import`: A model read from a file, its nodes, meshes and materials added as one step.

        { "command": "scene.import", "payload": { "materials": [ { "base_color": [0.5, 0.5, 0.5], "base_diffuse_roughness": 0, "base_metalness": 0, "base_weight": 1, "coat_color": [1, 1, 1], "coat_darkening": 1, "coat_ior": 1.5, "coat_roughness": 0.02, "coat_weight": 0, "displacement_detail": 0.0005, "edge_radius": 0, "edge_samples": 8, "emission_color": [1, 1, 1], "emission_luminance": 0, "flake_color": [0.9, 0.9, 0.9], "flake_roughness": 0.3, "flake_size": 0.00005, "flake_spread": 0.3, "flake_weight": 0, "fuzz_color": [1, 1, 1], "fuzz_roughness": 0.3, "fuzz_weight": 0, "geometry_occlusion": 1, "geometry_opacity": 1, "geometry_thin_walled": false, "id": "mat_00000001", "medium_priority": 0, "metal_ior": { "k": [7.475, 6.4, 5.303], "n": [1.345, 0.965, 0.617] }, "name": "Shell", "specular_color": [1, 1, 1], "specular_ior": 1.5, "specular_rotation": 0, "specular_roughness": 0.2, "specular_roughness_anisotropy": 0, "specular_weight": 1, "subsurface_color": [0.8, 0.8, 0.8], "subsurface_radius": [1, 1, 1], "subsurface_scatter_anisotropy": 0, "subsurface_weight": 0, "thin_film_ior": 1.4, "thin_film_thickness": 500, "thin_film_weight": 0, "transmission_color": [1, 1, 1], "transmission_depth": 0, "transmission_dispersion_abbe": 0, "transmission_scatter": [0, 0, 0], "transmission_scatter_anisotropy": 0, "transmission_tint": [1, 1, 1], "transmission_weight": 0, "type": "plastic", "volume_density": 1, "volume_emission_color": [1, 1, 1], "volume_emission_strength": 0 } ], "meshes": [ { "data": "RlNNRVNIAAABAAAAAwAAAAMAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgD8AAIA/AAAAAAAAAAAAAAAAAAAAAAAAgD8AAAAAAAAAAAAAAAAAAIA/AAAAAAAAAAABAAAAAgAAAAAAAAA=", "key": 1 } ], "nodes": [ { "children": [], "id": 50, "kind": "Group", "locked": false, "name": "X", "transform": [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1], "visible": true } ], "roots": [50], "textures": [ { "id": 7, "path": "maps/Leather037_Color.jpg", "role": "color" } ] } }
        """
        return self._apply('scene.import', {'payload': payload})

    def reimport(self, *, payload=_UNSET):
        r"""`scene.reimport`: An updated version of an imported model, matched to it part by part (`crate::reimport`).

        { "command": "scene.reimport", "payload": { "added": { "materials": [], "meshes": [ { "data": "RlNNRVNIAAABAAAAAwAAAAMAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgD8AAIA/AAAAAAAAAAAAAAAAAAAAAAAAgD8AAAAAAAAAAAAAAAAAAIA/AAAAAAAAAAABAAAAAgAAAAAAAAA=", "key": 1 } ], "nodes": [ { "children": [], "id": 51, "kind": "Part", "locked": false, "mesh": 1, "name": "Lid", "parent": 1, "path_key": "Bracket#1/Lid#1", "transform": [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1], "visible": true } ], "roots": [51] }, "hidden": [3], "updates": [ { "material": 1, "mesh": 1, "metadata": { "Revision": "B" }, "node": 2, "path_key": "Bracket#1/Housing#1", "transform": [1, 0, 0, 0, 0, 1, 0, 0.5, 0, 0, 1, 0, 0, 0, 0, 1] }, { "metadata": {}, "node": 1, "path_key": "Bracket#1" } ] } }
        """
        return self._apply('scene.reimport', {'payload': payload})

    def add_group(self, *, name=_UNSET, parent=_UNSET):
        r"""`scene.add_group`: An empty group, under `parent` or at the top.

        { "command": "scene.add_group", "name": "Sub", "parent": 1 }
        """
        return self._apply('scene.add_group', {'name': name, 'parent': parent})

    def delete_nodes(self, *, nodes=_UNSET):
        r"""`scene.delete_nodes`: Nodes and everything under them, removed.

        { "command": "scene.delete_nodes", "nodes": [2, 3] }
        """
        return self._apply('scene.delete_nodes', {'nodes': nodes})

    def split_parts(self, *, node=_UNSET):
        r"""`scene.split_parts`: Turn one part into a group of parts, one per connected shell of its mesh. For the models that arrive as a single lump because the file had no way to say otherwise — an STL, an OBJ with no groups — which is the only way to get at the pieces afterwards.

        { "command": "scene.split_parts", "node": 2 }
        """
        return self._apply('scene.split_parts', {'node': node})

    def rename(self, *, name=_UNSET, node=_UNSET):
        r"""`scene.rename`: A node's name.

        { "command": "scene.rename", "name": "Renamed", "node": 2 }
        """
        return self._apply('scene.rename', {'name': name, 'node': node})

    def set_transform(self, *, transforms=_UNSET):
        r"""`scene.set_transform`: Takes a transform per node rather than one node, because moving a selection of six parts is one action and has to be one undo step. A shared transform would be meaningless — each part has its own place — so the pairs are explicit.

        { "command": "scene.set_transform", "transforms": [ { "matrix": [1, 0, 0, 1.5, 0, 1, 0, -2.25, 0, 0, 1, 0.125, 0, 0, 0, 1], "node": 2 } ] }
        """
        return self._apply('scene.set_transform', {'transforms': transforms})

    def set_visibility(self, *, states=_UNSET):
        r"""`scene.set_visibility`: A flag per node rather than one flag for all, so that "Isolate" — show these, hide everything else — is a single command and therefore a single undo step. Two commands would leave the user pressing Ctrl+Z twice for one menu item, with the first press appearing to do nothing.

        { "command": "scene.set_visibility", "states": [ { "node": 2, "value": false } ] }
        """
        return self._apply('scene.set_visibility', {'states': states})

    def set_locked(self, *, states=_UNSET):
        r"""`scene.set_locked`: A lock per node: a locked node, and everything under it, refuses edits until unlocked.

        { "command": "scene.set_locked", "states": [ { "node": 2, "value": true } ] }
        """
        return self._apply('scene.set_locked', {'states': states})

    def set_pivot(self, *, pivots=_UNSET):
        r"""`scene.set_pivot`: The point a node rotates and scales about.

        { "command": "scene.set_pivot", "pivots": [ { "at": [0.5, -1.25, 2], "node": 2 } ] }
        """
        return self._apply('scene.set_pivot', {'pivots': pivots})

    def reparent(self, *, index=_UNSET, node=_UNSET, parent=_UNSET):
        r"""`scene.reparent`: A node moved under another parent (or to the top), at `index` among its siblings or last.

        { "command": "scene.reparent", "index": 3, "node": 2, "parent": 1 }
        """
        return self._apply('scene.reparent', {'index': index, 'node': node, 'parent': parent})

    def group(self, *, name=_UNSET, nodes=_UNSET):
        r"""`scene.group`: Collect nodes under a new group.

        { "command": "scene.group", "name": "Fasteners", "nodes": [2, 3] }
        """
        return self._apply('scene.group', {'name': name, 'nodes': nodes})

    def ungroup(self, *, group=_UNSET):
        r"""`scene.ungroup`: Dissolve a group, promoting its children.

        { "command": "scene.ungroup", "group": 1 }
        """
        return self._apply('scene.ungroup', {'group': group})

    def set_metadata(self, *, key=_UNSET, node=_UNSET, value=_UNSET):
        r"""`scene.set_metadata`: One metadata field on a node, set, or removed with no value.

        { "command": "scene.set_metadata", "key": "Mass_g", "node": 2, "value": "42.1" }
        """
        return self._apply('scene.set_metadata', {'key': key, 'node': node, 'value': value})

    def set_units(self, *, mode=_UNSET, unit=_UNSET):
        r"""`scene.set_units`: The unit lengths are shown in. They are stored in metres either way; `mode` says whether the scene keeps its size (`KeepSceneSize`) or keeps its numbers and is rescaled to them (`ScaleScene`), with its lights and cameras.

        { "command": "scene.set_units", "mode": "ScaleScene", "unit": "Inch" }
        """
        return self._apply('scene.set_units', {'mode': mode, 'unit': unit})

    def add_light(self, *, light=_UNSET, name=_UNSET):
        r"""`scene.add_light`: A light, under `parent` or at the top.

        { "command": "scene.add_light", "light": { "affects_diffuse": true, "affects_specular": true, "casts_shadows": true, "color": [1, 1, 1], "enabled": true, "height": 1, "intensity": 1500, "intensity_unit": "lm", "layer": 0, "shape": "Area", "two_sided": false, "visible_to_camera": true, "width": 1 }, "name": "Key" }
        """
        return self._apply('scene.add_light', {'light': light, 'name': name})

    def set_light(self, *, light=_UNSET, node=_UNSET):
        r"""`scene.set_light`: Every field of a light but its placement, which is the node's transform and goes through `scene.set_transform` like any other node's.

        { "command": "scene.set_light", "light": { "affects_diffuse": true, "affects_specular": true, "casts_shadows": true, "color": [1, 1, 1], "enabled": true, "falloff": 2, "inner_degrees": 12, "intensity": 400, "intensity_unit": "cd", "layer": 3, "outer_degrees": 30, "shape": "Spot", "temperature": 3200, "visible_to_camera": true }, "node": 2 }
        """
        return self._apply('scene.set_light', {'light': light, 'node': node})

    def add_cutaway(self, *, cutaway=_UNSET, name=_UNSET, parent=_UNSET, transform=_UNSET):
        r"""`scene.add_cutaway`: A cutaway volume, placed. The transform comes with it, because a cutter is sized to what it is meant to cut and a separate move would be a second undo step for one action.

        { "command": "scene.add_cutaway", "cutaway": { "cap": "color", "cap_color": [0.9, 0.2, 0.1], "enabled": false, "shape": "cylinder" }, "name": "Section", "parent": 1, "transform": [1, 0, 0, 0.5, 0, 1, 0, -1, 0, 0, 1, 2, 0, 0, 0, 1] }
        """
        return self._apply('scene.add_cutaway', {'cutaway': cutaway, 'name': name, 'parent': parent, 'transform': transform})

    def set_cutaway(self, *, cutaway=_UNSET, node=_UNSET):
        r"""`scene.set_cutaway`: Every field of a cutaway but its placement.

        { "command": "scene.set_cutaway", "cutaway": { "cap": "own", "enabled": true, "shape": "plane" }, "node": 2 }
        """
        return self._apply('scene.set_cutaway', {'cutaway': cutaway, 'node': node})

    def set_measurements(self, *, measurements=_UNSET):
        r"""`scene.set_measurements`: The whole list, as pins and decals are edited: adding, removing and hiding are all edits to the list.

        { "command": "scene.set_measurements", "measurements": [ { "a": { "node": 2, "point": [0.25, -1.5, 3] }, "b": { "point": [-2, 0.5, 0.125] }, "visible": false } ] }
        """
        return self._apply('scene.set_measurements', {'measurements': measurements})

    def set_model_sets(self, *, model_sets=_UNSET):
        r"""`scene.set_model_sets`: The sets, and which is shown.

        { "command": "scene.set_model_sets", "model_sets": { "active": 0, "sets": [ { "always_visible": true, "name": "Chassis A", "nodes": [1] } ] } }
        """
        return self._apply('scene.set_model_sets', {'model_sets': model_sets})


class LightingCommands(_Commands):
    """The `lighting.*` commands."""

    def set_preset(self, *, preset=_UNSET):
        r"""`lighting.set_preset`: The lighting preset: performance, basic, product, interior or jewelry.

        { "command": "lighting.set_preset", "preset": "Interior" }
        """
        return self._apply('lighting.set_preset', {'preset': preset})

    def set_custom(self, *, config=_UNSET):
        r"""`lighting.set_custom`: The Custom panel's integrator configuration.

        { "command": "lighting.set_custom", "config": { "adaptive_sampling_threshold": 0.005, "caustics": true, "direct_lighting": true, "global_illumination": true, "ground_illumination": true, "interior_mode": true, "light_source_materials": true, "max_bounces": 12, "max_transmission_bounces": 16, "regularization_roughness": 0.09, "regularization_start_bounce": 3, "russian_roulette_start_bounce": 3, "shadows": true } }
        """
        return self._apply('lighting.set_custom', {'config': config})


class EnvironmentCommands(_Commands):
    """The `environment.*` commands."""

    def set_parameter(self, *, parameter=_UNSET, value=_UNSET):
        r"""`environment.set_parameter`: One parameter of the environment: brightness, rotation, background, ground and the like.

        { "command": "environment.set_parameter", "parameter": "rotation", "value": 0.75 }
        """
        return self._apply('environment.set_parameter', {'parameter': parameter, 'value': value})

    def set_pins(self, *, pins=_UNSET):
        r"""`environment.set_pins`: The whole stack at once.

        { "command": "environment.set_pins", "pins": [ { "angular_size": [0.18, 1.5], "blend": "add", "color": [1, 1, 1], "direction": [0.24999999, 0.86602545, -0.43301266], "enabled": true, "intensity": 4, "kind": "light", "lighting": true, "name": "Top strip", "roll": 0, "shape": "rectangle", "show_in_background": false, "softness": 0.35 }, { "angular_size": [0.6, 0.45000002], "blend": "over", "color": [1, 1, 1], "direction": [0.46986896, 0.19866933, -0.86008936], "enabled": true, "image": "window.png", "intensity": 1, "kind": "image", "lighting": true, "name": "Window", "roll": 0, "shape": "rectangle", "show_in_background": true, "softness": 0 }, { "angular_size": [6.2831855, 6.2831855], "blend": "over", "color": [1, 1, 1], "direction": [0, -1, 0], "enabled": true, "intensity": 1, "kind": "gradient", "lighting": false, "name": "Backdrop", "radial": true, "roll": 0, "shape": "rectangle", "show_in_background": true, "softness": 0, "stops": [ { "color": [0.1, 0.1, 0.1], "t": 0 }, { "color": [0.7, 0.7, 0.7], "t": 1 } ] } ] }
        """
        return self._apply('environment.set_pins', {'pins': pins})

    def set_sky(self, *, sky=_UNSET):
        r"""`environment.set_sky`: A physical sky, or `None` for an image.

        { "command": "environment.set_sky", "sky": { "ground_albedo": 0.2, "intensity": 1, "sun": [0.45, 0.574, -0.685], "sun_disc": true, "sun_intensity": 4, "turbidity": 2 } }
        """
        return self._apply('environment.set_sky', {'sky': sky})


class StudioCommands(_Commands):
    """The `studio.*` commands."""

    def set(self, *, studios=_UNSET):
        r"""`studio.set`: The list of Studios. Editing a Studio does not apply it.

        { "command": "studio.set", "studios": [ { "environment": { "background": { "color": [0.05, 0.05, 0.05], "image": null, "mode": "Environment" }, "brightness": 1, "contrast": 1, "ground": { "bounce": false, "bounce_color": [1, 1, 1], "bounce_contact_falloff": 0, "bounce_strength": 1, "enabled": true, "flatten": false, "occlusion_shadows": false, "reflection_alpha": "coverage", "reflection_blur": 0, "reflection_fade_distance": 0, "reflection_fresnel": false, "reflection_length": 1, "reflections": 0, "shadow_color": [0, 0, 0], "shadows": true, "size": 0 }, "height": 0, "image": null, "rotation": 1.25, "size": 0 }, "model_set": null, "name": "Hero", "variants": [ { "active": 0, "multi_material": 7 } ] } ] }
        """
        return self._apply('studio.set', {'studios': studios})

    def apply(self, *, index=_UNSET):
        r"""`studio.apply`: Make the scene what a Studio says, in one step.

        { "command": "studio.apply", "index": 0 }
        """
        return self._apply('studio.apply', {'index': index})


class ConfiguratorCommands(_Commands):
    """The `configurator.*` commands."""

    def set(self, *, configurator=_UNSET):
        r"""`configurator.set`: The configurator: its options, rules and layout. Setting it does not apply a configuration.

        { "command": "configurator.set", "configurator": { "current": [0, 0], "options": [ { "choices": [ { "label": "Red", "swatch": [0.5, 0.05, 0.04], "target": 0 }, { "label": "Cream", "target": 1 } ], "drives": "material", "id": 1, "multi_material": 7, "name": "Body", "style": "swatches" }, { "choices": [ { "label": "Hero", "target": 0 } ], "drives": "studio", "id": 2, "name": "View", "style": "buttons" } ], "placement": "bottom", "rules": [ { "choice": 1, "kind": "excludes", "option": 1, "other": 2, "other_choice": 0 } ], "subtitle": "Pick a finish", "title": "Kettle" } }
        """
        return self._apply('configurator.set', {'configurator': configurator})

    def choose(self, *, choices=_UNSET):
        r"""`configurator.choose`: Show a configuration: every option's choice applied, in one step. Refused when it breaks a rule.

        { "choices": [0, 0], "command": "configurator.choose" }
        """
        return self._apply('configurator.choose', {'choices': choices})


class CameraCommands(_Commands):
    """The `camera.*` commands."""

    def add(self, *, camera=_UNSET):
        r"""`camera.add`: Save the current view as a new camera.

        { "camera": { "basis": [0.83205026, 0, -0.5547002, -0.13433959, 0.9702303, -0.20150937, 0.53818697, 0.24218413, 0.8072804], "dof": { "aperture": "Circular", "enabled": false, "f_number": 8, "focus_distance": 743.23615, "keep_focus": false }, "focal_length_mm": 50, "iso": 100, "name": "Hero", "position": [400, 220, 600], "sensor_width_mm": 36, "shutter_speed_seconds": 0.01, "target": [0, 40, 0], "type": "Perspective", "up": [-0.13433959, 0.9702303, -0.20150937] }, "command": "camera.add" }
        """
        return self._apply('camera.add', {'camera': camera})

    def remove(self, *, index=_UNSET):
        r"""`camera.remove`: A saved camera, removed.

        { "command": "camera.remove", "index": 1 }
        """
        return self._apply('camera.remove', {'index': index})

    def select(self, *, index=_UNSET):
        r"""`camera.select`: Which saved camera is the active one.

        { "command": "camera.select", "index": 0 }
        """
        return self._apply('camera.select', {'index': index})

    def set(self, *, camera=_UNSET, index=_UNSET):
        r"""`camera.set`: Replace a saved camera, which is what every lens control and every orbit of the viewport amounts to.

        { "camera": { "basis": [0.9486834, 0, -0.3162278, -0.16903086, 0.8451543, -0.5070926, 0.26726124, 0.5345225, 0.8017837], "dof": { "aperture": "Circular", "enabled": false, "f_number": 8, "focus_distance": 3.7416575, "keep_focus": false }, "focal_length_mm": 50, "iso": 100, "name": "Hero", "position": [1, 2, 3], "sensor_width_mm": 36, "shutter_speed_seconds": 0.01, "target": [0, 0, 0], "type": "Perspective", "up": [-0.16903086, 0.8451543, -0.5070926] }, "command": "camera.set", "index": 0 }
        """
        return self._apply('camera.set', {'camera': camera, 'index': index})

    def rename(self, *, index=_UNSET, name=_UNSET):
        r"""`camera.rename`: A saved camera's name.

        { "command": "camera.rename", "index": 0, "name": "Three quarter" }
        """
        return self._apply('camera.rename', {'index': index, 'name': name})


class ImageCommands(_Commands):
    """The `image.*` commands."""

    def set_style(self, *, style=_UNSET):
        r"""`image.set_style`: One command for the whole style rather than one per control: §9.3 classifies every one of them as `Display`, so there is nothing to be gained by telling them apart, and a style is small enough to carry whole.

        { "command": "image.set_style", "style": { "blacks": 0, "bloom": 0, "bloom_radius": 0.03, "chromatic_aberration": 0, "contrast": 1, "display_transform": "PBR Neutral", "exposure": 0.75, "highlights": -0.4, "saturation": 1, "shadows": 0, "sharpening": 0, "temperature": 5200, "tint": 0, "type": "Photographic", "vignette": 0.3, "whites": 0 } }
        """
        return self._apply('image.set_style', {'style': style})

    def set_color_management(self, *, color_management=_UNSET):
        r"""`image.set_color_management`: Choose an OCIO config's display and view, or `None` to go back to the built-in transforms.

        { "color_management": { "config": "studio/config.ocio", "display": "sRGB", "view": "ACES 1.0 - SDR Video" }, "command": "image.set_color_management" }
        """
        return self._apply('image.set_color_management', {'color_management': color_management})


class RenderCommands(_Commands):
    """The `render.*` commands."""

    def set_settings(self, *, render=_UNSET):
        r"""`render.set_settings`: One command for the whole block rather than one per control, for the same reason the image style is one: none of it reaches the renderer until Render is pressed, so there is nothing to be gained by telling the settings apart.

        { "command": "render.set_settings", "render": { "adaptive": true, "adaptive_threshold": null, "alpha_mode": "none", "clamp_direct": null, "clamp_indirect": 12, "denoise": true, "denoise_cadence": 4, "denoise_strength": 1, "depth": "32", "dpi": 300, "engine": "cycles-gpu", "filename_template": "{project}_{camera}_{date}", "format": "exr", "jpeg_quality": 92, "layers": [], "max_samples": 512, "max_seconds": 600, "output_directory": "renders", "pixel_filter": null, "prefilter_guides": true, "region": [10, 20, 320, 240], "reject_fireflies": true, "relight": false, "resolution": [3840, 2160], "write_passes": true, "write_text_metadata": false, "write_xmp": false } }
        """
        return self._apply('render.set_settings', {'render': render})


class Commands:
    """The whole catalog, by group: `commands.material.set_type(...)`."""

    def __init__(self, client):
        self.material = MaterialCommands(client)
        self.scene = SceneCommands(client)
        self.lighting = LightingCommands(client)
        self.environment = EnvironmentCommands(client)
        self.studio = StudioCommands(client)
        self.configurator = ConfiguratorCommands(client)
        self.camera = CameraCommands(client)
        self.image = ImageCommands(client)
        self.render = RenderCommands(client)
