//! KeyShot scenes (`.bip`, `.ksp`): a scene written here, token by token as
//! the format's grammar has them, and read back.
//!
//! The scene is small and made up: a trimmed square with a hole, a quarter
//! cylinder, a triangle, groups with a hidden variant, every kind of
//! material the reader maps, an HDRI-editor environment with two pins,
//! cameras with depth of field and an orthographic one, an image style,
//! the compressed chunk, and a package with a picture.

use std::io::Write;
use tequila_core::math::{vec3, Vec3};
use tequila_import::{read_format, ImportOptions};
use tequila_materials::graph::{NodeKind, Socket};
use tequila_materials::material_type::{MaterialType, Param};
use tequila_render_core::camera::Projection;
use tequila_scene::document::BackgroundMode;
use tequila_scene::{CommandBus, SceneDocument};

// ---- the writer -------------------------------------------------------------------

/// Tokens, written as the grammar says: a tag (the type doubled, plus one
/// for a one-byte size), the size, the payload.
#[derive(Default)]
struct Bip(Vec<u8>);

impl Bip {
    fn token(&mut self, id: u8, payload: &[u8]) -> &mut Self {
        if payload.len() < 256 {
            self.0.push(id * 2 + 1);
            self.0.push(payload.len() as u8);
        } else {
            self.0.push(id * 2);
            self.0.extend((payload.len() as u32).to_le_bytes());
        }
        self.0.extend_from_slice(payload);
        self
    }
    fn keyed(&mut self, id: u8, value: &[u8], key: &str) -> &mut Self {
        let mut p = value.to_vec();
        p.extend(key.as_bytes());
        self.token(id, &p)
    }
    fn f32(&mut self, key: &str, v: f32) -> &mut Self {
        self.keyed(0x0b, &v.to_le_bytes(), key)
    }
    fn f64(&mut self, key: &str, v: f64) -> &mut Self {
        self.keyed(0x1a, &v.to_le_bytes(), key)
    }
    fn i32(&mut self, key: &str, v: i32) -> &mut Self {
        self.keyed(0x0e, &v.to_le_bytes(), key)
    }
    fn bool(&mut self, key: &str, v: bool) -> &mut Self {
        self.keyed(0x12, &(v as u32).to_le_bytes(), key)
    }
    fn vec3f(&mut self, key: &str, v: [f32; 3]) -> &mut Self {
        let b: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
        self.keyed(0x13, &b, key)
    }
    fn vec4f(&mut self, key: &str, v: [f32; 4]) -> &mut Self {
        let b: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
        self.keyed(0x0d, &b, key)
    }
    fn vec3d(&mut self, key: &str, v: [f64; 3]) -> &mut Self {
        let b: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
        self.keyed(0x15, &b, key)
    }
    fn mat4d(&mut self, key: &str, m: [f64; 16]) -> &mut Self {
        let b: Vec<u8> = m.iter().flat_map(|x| x.to_le_bytes()).collect();
        self.keyed(0x16, &b, key)
    }
    fn text(&mut self, id: u8, key: &str, v: &str) -> &mut Self {
        let mut p = (v.len() as u32).to_le_bytes().to_vec();
        p.extend(v.as_bytes());
        p.extend(key.as_bytes());
        self.token(id, &p)
    }
    fn str(&mut self, key: &str, v: &str) -> &mut Self {
        self.text(0x17, key, v)
    }
    fn attr(&mut self, key: &str, v: &str) -> &mut Self {
        self.text(0x1e, key, v)
    }
    fn link(&mut self, key: &str, to: u32) -> &mut Self {
        self.keyed(0x18, &to.to_le_bytes(), key)
    }
    fn param_link(&mut self, key: &str, to: u32) -> &mut Self {
        self.keyed(0x19, &to.to_le_bytes(), key)
    }
    fn obj_link(&mut self, key: &str, to: u32) -> &mut Self {
        self.keyed(0x47, &to.to_le_bytes(), key)
    }
    fn array(&mut self, id: u8, key: &str, data: &[u8]) -> &mut Self {
        let mut p = (key.len() as u32).to_le_bytes().to_vec();
        p.extend(key.as_bytes());
        p.extend(data);
        self.token(id, &p)
    }
    fn f64s(&mut self, key: &str, v: &[f64]) -> &mut Self {
        self.array(0x3d, key, &v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>())
    }
    fn f32s(&mut self, key: &str, v: &[f32]) -> &mut Self {
        self.array(0x3a, key, &v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>())
    }
    fn pairs(&mut self, key: &str, v: &[[f32; 2]]) -> &mut Self {
        self.array(0x3c, key, &v.iter().flatten().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>())
    }
    fn i32s(&mut self, key: &str, v: &[i32]) -> &mut Self {
        self.array(0x3f, key, &v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>())
    }
    fn bools(&mut self, key: &str, v: &[bool]) -> &mut Self {
        self.array(0x3e, key, &v.iter().flat_map(|x| (*x as u32).to_le_bytes()).collect::<Vec<u8>>())
    }
    fn raw(&mut self, id: u8, v: u32) -> &mut Self {
        self.token(id, &v.to_le_bytes())
    }
    fn id(&mut self, v: u32) -> &mut Self {
        self.raw(0x1c, v)
    }
    fn flags(&mut self, v: u32) -> &mut Self {
        self.raw(0x1d, v)
    }
    fn begin(&mut self, id: u8, name: &str) -> &mut Self {
        self.token(id, name.as_bytes())
    }
    fn node(&mut self, number: u32, kind: &str) -> &mut Self {
        let mut p = number.to_le_bytes().to_vec();
        p.extend(kind.as_bytes());
        self.token(0x03, &p)
    }
    fn object(&mut self, number: u32, class: &str, name: &str) -> &mut Self {
        let mut p = number.to_le_bytes().to_vec();
        p.extend((class.len() as u32).to_le_bytes());
        p.extend(class.as_bytes());
        p.extend(name.as_bytes());
        self.token(0x1f, &p)
    }
    fn end(&mut self, id: u8) -> &mut Self {
        self.token(id, &[])
    }
    /// A plain chunk around `body`.
    fn chunk(&mut self, kind: u16, body: &Bip) -> &mut Self {
        self.token(0x43, &chunk_header(body.0.len() as u64, 0, kind));
        self.0.extend(&body.0);
        self.end(0x44)
    }
    /// A compressed chunk: the token's size covers only its header, and the
    /// zlib stream follows it.
    fn compressed(&mut self, kind: u16, body: &Bip) -> &mut Self {
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(&body.0).unwrap();
        let packed = z.finish().unwrap();
        self.token(0x43, &chunk_header(packed.len() as u64, 1, kind));
        self.0.extend(&packed);
        self.end(0x44)
    }
    fn file(&self) -> Vec<u8> {
        let mut out = b"//---luxion:dal:020---\n".to_vec();
        out.resize(0x80, 0);
        out[0x28..0x2a].copy_from_slice(&11u16.to_le_bytes());
        out[0x2a..0x2c].copy_from_slice(&3u16.to_le_bytes());
        out.extend(&self.0);
        let length = out.len() as u32;
        out[0x20..0x24].copy_from_slice(&length.to_le_bytes());
        out
    }
}

fn chunk_header(length: u64, compression: u16, kind: u16) -> Vec<u8> {
    let mut p = length.to_le_bytes().to_vec();
    p.extend(compression.to_le_bytes());
    p.extend(kind.to_le_bytes());
    p.extend(0u32.to_le_bytes());
    p
}

/// Turn KeyShot's way: about X by -90 degrees (a CAD model drawn Z up,
/// stood Y up), lifted 300 mm. Points multiplied on the left.
const STAND_UP: [f64; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 300.0, 0.0, 1.0];
const IDENTITY: [f64; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

/// A flat square face, `side` mm, at CAD height `z`: a bilinear patch over
/// its whole domain, trimmed by its edge and, when `hole`, by a square hole
/// a fifth of its side across in the middle.
fn square(b: &mut Bip, id: u32, side: f64, z: f64, hole: bool) {
    b.begin(0x26, "Square").id(id).begin(0x2d, "nurbs");
    b.i32("uorder", 2).f64s("uknot", &[0.0, 0.0, 1.0, 1.0]).f64("umin", 0.0).f64("umax", 1.0);
    b.i32("vorder", 2).f64s("vknot", &[0.0, 0.0, 1.0, 1.0]).f64("vmin", 0.0).f64("vmax", 1.0);
    b.f64s("Pw", &[0.0, 0.0, z, 1.0, side, 0.0, z, 1.0, 0.0, side, z, 1.0, side, side, z, 1.0]);
    let outer = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let inner = [[0.4, 0.4], [0.4, 0.6], [0.6, 0.6], [0.6, 0.4]];
    let mut loops = vec![outer];
    if hole {
        loops.push(inner);
    }
    let edges = loops.len() * 4;
    b.i32s("trim_nedges", &vec![4; loops.len()]).bools("trim_inside", &loops.iter().enumerate().map(|(i, _)| i == 0).collect::<Vec<_>>());
    b.i32s("edge_order", &vec![2; edges]).i32s("edge_nknots", &vec![4; edges]);
    b.f64s("edge_min", &vec![0.0; edges]).f64s("edge_max", &vec![1.0; edges]);
    b.f64s("edge_knot", &[0.0, 0.0, 1.0, 1.0].repeat(edges));
    let mut uvw = Vec::new();
    for l in &loops {
        for k in 0..4 {
            let (a, c) = (l[k], l[(k + 1) % 4]);
            uvw.extend([a[0], a[1], 1.0, c[0], c[1], 1.0]);
        }
    }
    b.f64s("edge_uvw", &uvw);
    b.end(0x2e).end(0x27);
}

/// A quarter of a cylinder of radius `r` about CAD Y, 50 mm long: a
/// rational quadratic arc, its middle weight √2/2, stored multiplied in.
fn quarter_cylinder(b: &mut Bip, id: u32, r: f64) {
    let w = std::f64::consts::FRAC_1_SQRT_2;
    b.begin(0x26, "Cylinder").id(id).begin(0x2d, "nurbs");
    b.i32("uorder", 3).f64s("uknot", &[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]).f64("umin", 0.0).f64("umax", 1.0);
    b.i32("vorder", 2).f64s("vknot", &[0.0, 0.0, 1.0, 1.0]).f64("vmin", 0.0).f64("vmax", 1.0);
    let mut pw = Vec::new();
    for y in [0.0, 50.0] {
        pw.extend([r, y, 0.0, 1.0]);
        pw.extend([r * w, y * w, r * w, w]);
        pw.extend([0.0, y, r, 1.0]);
    }
    b.f64s("Pw", &pw);
    b.i32s("trim_nedges", &[]).bools("trim_inside", &[]).i32s("edge_order", &[]).i32s("edge_nknots", &[]);
    b.f64s("edge_min", &[]).f64s("edge_max", &[]).f64s("edge_knot", &[]).f64s("edge_uvw", &[]);
    b.end(0x2e).end(0x27);
}

fn triangle(b: &mut Bip, id: u32) {
    mesh(b, id, &[[0.0, 0.0, 10.0], [20.0, 0.0, 10.0], [0.0, 20.0, 10.0]], &[[0, 1, 2]], [0.0, 0.0, 1.0]);
}

/// A triangle mesh, every vertex given the normal `n`.
fn mesh(b: &mut Bip, id: u32, points: &[[f32; 3]], triangles: &[[i32; 3]], n: [f32; 3]) {
    b.begin(0x26, "Mesh").id(id).begin(0x2d, "triangles");
    b.f32("rounded_edge_angle", 30.0);
    b.f32s("vertices", &points.iter().flatten().copied().collect::<Vec<_>>());
    b.i32s("triangles", &triangles.iter().flatten().copied().collect::<Vec<_>>());
    b.f32s("normals", &points.iter().flat_map(|_| n).collect::<Vec<_>>());
    b.pairs("uv_tex", &points.iter().map(|p| [p[0] / 100.0, p[1] / 100.0]).collect::<Vec<_>>());
    b.end(0x2e).end(0x27);
}

fn part(b: &mut Bip, name: &str, id: u32, shape: u32, material: u32, visible: bool) {
    b.begin(0x25, name);
    if visible {
        b.flags(0x4000);
    }
    b.id(id).raw(0x38, shape).raw(0x37, material).attr("original_material_name", "Layer:0:0:0").end(0x27);
}

/// The label's frame: object x (in metres) across the picture, object y up
/// it, looking down object z; a 100 mm picture with its corner at the
/// origin.
const LABEL_FRAME: [f64; 16] = [0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, -0.05, 0.0, -0.05, 1.0];

fn texture(b: &mut Bip, number: u32, kind: &str, file: &str, flags: i32) {
    b.node(number, kind).i32("texture_space", 1).i32("uv_projection", 0);
    b.f32("uv_scale_u", 0.1).f32("uv_scale_v", 0.1).f32("uv_shift_u", 0.0).f32("uv_shift_v", 0.0).f32("uv_angle", 0.0);
    b.i32("uv_flags", flags).mat4d("transform_obj_to_uv", LABEL_FRAME).str("texture", file);
    b.f32("brightness", 1.0).f32("contrast", 1.0).f32("bump_height", 0.5).i32("bump_mode", 0).vec4f("default_color", [0.0; 4]).end(0x04);
}

fn plastic(b: &mut Bip, number: u32, diffuse: [f32; 3], roughness: f32) {
    b.node(number, "lux_plastic_simple").vec3f("diffuse", diffuse).f32("alpha", 1.0).vec3f("specular", [1.0; 3]).f32("roughness", roughness).f32("ior", 1.5);
    b.str("attribute", "--MATMETA--AAAAAQAAAAA=").end(0x04);
}

fn material(b: &mut Bip, name: &str, id: u32, root: u32) {
    let mut p = 1u32.to_le_bytes().to_vec();
    p.extend(root.to_le_bytes());
    b.begin(0x05, name).flags(0x1000).token(0x08, &p).id(id).end(0x07);
}

/// The whole scene.
fn scene() -> Vec<u8> {
    let mut b = Bip::default();

    let mut geometry = Bip::default();
    square(&mut geometry, 100, 100.0, 0.0, true);
    quarter_cylinder(&mut geometry, 101, 40.0);
    square(&mut geometry, 102, 100.0, 5.0, false);
    square(&mut geometry, 103, 80.0, -20.0, false);
    triangle(&mut geometry, 104);
    square(&mut geometry, 105, 30.0, 60.0, false);
    // KeyShot's own mesh of the first object: its square, looking down.
    // The face itself, as stored, looks up; the mesh says which way.
    mesh(&mut geometry, 106, &[[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [100.0, 100.0, 0.0], [0.0, 100.0, 0.0]], &[[0, 2, 1], [0, 3, 2]], [0.0, 0.0, -1.0]);
    geometry.token(0x48, b"Square\0\0\x80\x3f");
    b.chunk(1, &geometry);

    let mut settings = Bip::default();
    settings.begin(0x35, "luxrtopt.so").i32("render_width", 1280).i32("render_height", 720).i32("render_samples", 64).f32("meter_scale", 1000.0);
    settings.str("active_scene_set", "Default").i32("output_style_id", 900).end(0x36);
    settings.begin(0x35, "luxdenoise.so").end(0x36);
    settings.token(0x1b, b"0\x89PNG not really");
    b.chunk(4, &settings);

    b.begin(0x09, "").i32("width", 2490).f32("gain", 1.0).end(0x0a);
    b.begin(0x23, "").i32("trace level", 14).end(0x24);

    // Project objects, compressed: two environments, the first in use with
    // a rectangular pin and a round one over a grey background; the image
    // style.
    let mut objects = Bip::default();
    let environment = |o: &mut Bip, number: u32, name: &str, id: u32, active: bool, map: u32| {
        o.object(number, "lux.env.hdri", name).id(id).flags(0);
        o.f32("map.gain", 1.5).f32("map.gamma", 1.0).f32("map.size", 2.0).f32("map.height", 0.0).f32("map.rotation", 90.0);
        o.bool("ground.shadows", true).bool("ground.occlusionShadows", false).bool("ground.reflection", false).bool("ground.flatten", false);
        o.f32("ground.size", 1.0).vec3f("ground.shadowColor", [0.0; 3]).i32("background.type", 0).vec3f("background.color", [1.0; 3]);
        o.bool("background.adjust", false).f32s("sample.dir", &[]).bool("active", active).obj_link("map", map).end(0x20);
    };
    let brush = |o: &mut Bip, number: u32, colour: [f32; 3], gain: f32| {
        o.object(number, "lux.env.brush.color", "").flags(0).i32("blend_mode", 0).f32("color.gain", gain).f32("color.gamma", 1.0);
        o.f32("color.hue", 0.0).f32("color.saturation", 0.0).vec3f("color.colorize", [1.0; 3]).vec3f("color", colour).end(0x20);
    };
    brush(&mut objects, 3, [0.2, 0.2, 0.2], 1.0);
    brush(&mut objects, 4, [1.0; 3], 12.0);
    objects.object(5, "lux.env.brush.gradient", "").flags(0).i32("blend_mode", 0).f32("color.gain", 3.0).f32("color.gamma", 1.0).vec3f("color.colorize", [1.0; 3]);
    objects.pairs("gradient.color", &[[0.0, 1.0], [1.0, 1.0]]).pairs("gradient.alpha", &[[0.0, 1.0], [1.0, 0.0], [1.0, 1.0]]).f32("angle", 30.0).end(0x20);
    let pin = |o: &mut Bip, number: u32, name: &str, brush: u32, circle: bool, azimuth: f32, inclination: f32| {
        o.object(number, "lux.env.pin", name).flags(0).bool("enabled", true).obj_link("brush", brush).i32("shape", circle as i32);
        o.f32("radius", 15.0).f32("width", 20.0).f32("height", 40.0).f32("corner_radius", 0.0).f32("rotation", 10.0);
        o.i32("falloff_type", if circle { 19 } else { 32 }).f32("falloff", 0.5).f32("azimuth", azimuth).f32("inclination", inclination).bool("half", false).end(0x20);
    };
    pin(&mut objects, 6, "Softbox", 4, false, 45.0, 30.0);
    pin(&mut objects, 7, "Strip", 5, true, -90.0, 10.0);
    objects.object(8, "lux.env.group", "").flags(0).bool("enabled", true).i32s("children", &[6, 7]).end(0x20);
    objects.object(9, "lux.env.image_map", "").flags(0).i32("size.width", 4000).i32("size.height", 2000).obj_link("background", 3).obj_link("pins", 8).end(0x20);
    environment(&mut objects, 10, "Front", 901, true, 9);
    brush(&mut objects, 11, [0.5; 3], 1.0);
    objects.object(12, "lux.env.group", "").flags(0).bool("enabled", true).i32s("children", &[]).end(0x20);
    objects.object(13, "lux.env.image_map", "").flags(0).obj_link("background", 11).obj_link("pins", 12).end(0x20);
    environment(&mut objects, 14, "Top", 902, false, 13);
    objects.object(15, "lux.style.filmic", "Look").id(900).flags(0).bool("active", true).f32("exposure", 0.5).f32("contrast", 0.1);
    objects.i32("tonemapping", 2).bool("use_background_color", true).vec3f("background_color", [1.0; 3]).bool("denoise", false).end(0x20);
    objects.object(16, "lux.studios.studiobasenode", "studio_node").id(1008).flags(0).end(0x20);
    b.compressed(0, &objects);

    // The material library: every kind the reader maps.
    plastic(&mut b, 1, [0.5, 0.5, 0.5], 0.09);
    material(&mut b, "Grey", 1, 1);
    b.node(2, "metallic_paint").vec3f("base", [0.2, 0.0, 0.0]).vec3f("clearcoat", [1.0; 3]).f32("thickness multiplier", 1.0).vec3f("metal", [0.8; 3]);
    b.f32("roughness", 0.04).f32("metal_roughness", 0.16).f32("metal_coverage", 0.5).f32("ior", 1.5).f32("metal_flake_size", 0.0).f32("metal_flake_visibility", 0.8).end(0x04);
    material(&mut b, "Paint", 2, 2);
    b.node(3, "lux_metal_coated").i32("preset", 3).bool("coated", false).f32("film_ior", 2.0).vec3f("color", [0.0; 3]).f32("roughness", 0.0).end(0x04);
    material(&mut b, "Chrome", 3, 3);
    b.node(4, "lux_glass_simple").vec3f("color", [0.9; 3]).f32("ior", 1.5).f32("transparency", 0.5).f32("roughness", 0.0).end(0x04);
    material(&mut b, "Glass", 4, 4);
    texture(&mut b, 5, "texture2_rgba", ".\\screen.png", 0x430);
    b.node(6, "lux_emissive").vec3f("color", [0.0; 3]).param_link("color", 5).f32("alpha", 1.0).f32("intensity", 5.0).bool("twosided", true);
    b.bool("visible to the eye", true).bool("visible in reflections", true).bool("visible in shadows", false).end(0x04);
    b.node(7, "lux_label").link("shader", 6).end(0x04);
    plastic(&mut b, 8, [0.0; 3], 0.0);
    b.node(9, "lux_composite").i32s("shaders", &[7, 8]).bool("opacity_map", false).end(0x04);
    material(&mut b, "Screen", 5, 9);
    b.node(10, "lux_fractal_noise").vec3f("color1", [0.0; 3]).vec3f("color2", [0.8; 3]).i32("levels", 5).f32("fall-off", 1.2).f32("scale", 0.001);
    b.f32("bump_height", 0.1).i32("texture_space", 1).mat4d("transform_obj_to_uv", IDENTITY).end(0x04);
    plastic(&mut b, 11, [0.3; 3], 0.04);
    b.node(12, "lux_bumpmap").link("bumpmap", 10).link("shader", 11).f32("bump_scale", 1.0).end(0x04);
    material(&mut b, "Grained", 6, 12);
    texture(&mut b, 13, "texture2_rgb", "screen.png", 0x430);
    b.node(14, "lux_plastic_simple").vec3f("diffuse", [1.0; 3]).param_link("diffuse", 13).f32("roughness", 0.25).end(0x04);
    b.node(15, "lux_label").link("shader", 14).end(0x04);
    plastic(&mut b, 16, [0.9; 3], 0.04);
    b.node(17, "lux_composite").i32s("shaders", &[15, 16]).bool("opacity_map", false).end(0x04);
    material(&mut b, "Printed", 7, 17);
    // A multi-material of the grey and the paint, the grey shown.
    let mut root = 1u32.to_le_bytes().to_vec();
    root.extend(1u32.to_le_bytes());
    b.begin(0x05, "Colourway").i32s("materials", &[1, 2]).flags(0x1800).token(0x08, &root).id(8).end(0x07);

    // The tree: a model set, a model stood up and lifted, a layer, two
    // objects, and a hidden variant.
    b.begin(0x2a, "Default").id(1005).attr("model set", "Default").flags(0x0086_000a);
    b.begin(0x2a, "Model").id(2).attr("source file", "C:/cad/model.3dm").attr("center", "0,0,0").flags(0x0004_c009);
    b.begin(0x2a, "Layer").id(3).flags(0x0004_c009);
    b.begin(0x2a, "Body(1)").id(4).flags(0x0005_4408);
    part(&mut b, "Face", 5, 100, 8, true);
    part(&mut b, "Face", 6, 101, 8, true);
    b.token(0x48, b"Face\0\0\0\x3f");
    part(&mut b, "Face", 7, 105, 6, true);
    part(&mut b, "Face", 13, 106, 8, true);
    b.end(0x2c);
    b.begin(0x2a, "Mesh(4)").id(14).flags(0x0005_4408);
    part(&mut b, "Face", 15, 104, 6, true);
    b.end(0x2c);
    b.begin(0x2a, "Screen(2)").id(8).flags(0x0005_4408);
    part(&mut b, "Face", 9, 102, 5, true);
    b.end(0x2c);
    // An object KeyShot has replaced with a mesh of its own, split to wear
    // two materials: its visibility bit still set, and bit 22 with it.
    b.begin(0x2a, "Panel(5)").id(16).flags(0x0045_4408);
    part(&mut b, "Face", 17, 104, 6, true);
    b.end(0x2c);
    b.end(0x2c);
    b.token(0x2f, b"group_local_transform");
    b.token(0x30, &STAND_UP.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>());
    b.end(0x2c);
    b.begin(0x2a, "Variant").id(10).attr("source file", "C:/cad/variant.3dm").flags(0x0004_8009);
    b.begin(0x2a, "Other(3)").id(11).flags(0x0005_0408);
    part(&mut b, "Face", 12, 103, 3, false);
    b.end(0x2c);
    b.token(0x2f, b"group_local_transform");
    b.token(0x30, &STAND_UP.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>());
    b.end(0x2c);
    b.end(0x2c);

    // Cameras, each after its own graph: a 50 mm one named after the
    // environment in use, one with depth of field, an orthographic one,
    // and the view the scene was saved in.
    let envmap = |b: &mut Bip| {
        b.node(1, "lux_envmap").str("texture", "startup.hdr").f32("multiplier", 1.5).f32("radius", 2000.0).mat4d("user_transform", IDENTITY).end(0x04);
    };
    let camera = |b: &mut Bip, name: &str, id: Option<u32>, fov: f32| {
        b.begin(0x01, name).vec3d("position", [0.0, 300.0, 600.0]).vec3d("direction", [0.0, 0.0, -1.0]).vec3d("up", [0.0, 1.0, 0.0]);
        b.f32("fov", fov).i32("flags", 0x41a0);
        if let Some(id) = id {
            b.id(id);
        }
        b.vec3d("pivot", [0.0, 300.0, 0.0]).bool("has_pivot", true).link("reflection map", 1);
    };
    envmap(&mut b);
    camera(&mut b, "Front", Some(50), 39.597_755);
    b.token(0x39, &IDENTITY.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>()).end(0x02);
    envmap(&mut b);
    b.node(2, "rtrt_dof_effect").f32("fstop", 8.0).f32("focal_dist", 400.0).f32("focal_length", 0.05).i32("blades", 0).end(0x04);
    camera(&mut b, "Detail", Some(51), 39.597_755);
    b.link("lens", 2).end(0x02);
    envmap(&mut b);
    camera(&mut b, "Side", Some(52), 39.597_755);
    b.str("projection", "orthographic").vec4f("window", [-100.0, 100.0, -56.25, 56.25]).end(0x02);
    envmap(&mut b);
    camera(&mut b, "last_active", None, 25.360_77);
    b.end(0x02);
    b.file()
}

/// A tiny RGBA picture, as PNG.
fn png() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, 4, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[200u8; 32]).unwrap();
    }
    out
}

/// A ZIP of stored members, as a `.ksp` is a ZIP.
fn zip(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut directory = Vec::new();
    for (name, data) in members {
        let offset = out.len() as u32;
        let header = |out: &mut Vec<u8>, signature: u32, central: bool| {
            out.extend(signature.to_le_bytes());
            if central {
                out.extend(20u16.to_le_bytes());
            }
            out.extend(20u16.to_le_bytes());
            out.extend(0u16.to_le_bytes()); // flags
            out.extend(0u16.to_le_bytes()); // stored
            out.extend(0u32.to_le_bytes()); // time and date
            out.extend(0u32.to_le_bytes()); // CRC, not checked by the reader
            out.extend((data.len() as u32).to_le_bytes());
            out.extend((data.len() as u32).to_le_bytes());
            out.extend((name.len() as u16).to_le_bytes());
            out.extend(0u16.to_le_bytes());
            if central {
                out.extend([0u8; 6]); // comment, disk, internal attributes
                out.extend(0u32.to_le_bytes());
                out.extend(offset.to_le_bytes());
            }
            out.extend(name.as_bytes());
        };
        header(&mut out, 0x0403_4b50, false);
        out.extend(*data);
        header(&mut directory, 0x0201_4b50, true);
    }
    let start = out.len() as u32;
    out.extend(&directory);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0u8; 4]);
    out.extend((members.len() as u16).to_le_bytes());
    out.extend((members.len() as u16).to_le_bytes());
    out.extend((directory.len() as u32).to_le_bytes());
    out.extend(start.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}

fn picture(name: &str) -> Option<Vec<u8>> {
    (name.ends_with("screen.png")).then(png)
}

// ---- reading it -------------------------------------------------------------------

fn read() -> tequila_import::raw::RawScene {
    read_format(&scene(), "bip", "Speaker", &picture).expect("the scene reads")
}

/// The file reads with nothing wrong with it, and its tree is KeyShot's:
/// the model set's group named after the file, the model stood up and
/// lifted, an object of two materials a group of two parts, an object of
/// one a part, and the variant hidden.
#[test]
fn the_tree_reads_with_names_placements_and_visibility() {
    let scene = read();
    for bad in ["unknown token", "cut short", "not closed", "closes nothing", "could not be divided"] {
        assert!(!scene.warnings.iter().any(|w| w.contains(bad)), "{bad}: {:#?}", scene.warnings);
    }
    let names: Vec<&str> = scene.nodes.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names[0], "Speaker");
    let index = |name: &str| names.iter().position(|n| *n == name).unwrap_or_else(|| panic!("no {name} in {names:?}"));
    let model = &scene.nodes[index("Model")];
    assert!(!model.hidden);
    // Turned and lifted: CAD +Z is up, 300 mm off the floor.
    let up = model.transform.vector(vec3(0.0, 0.0, 1.0));
    assert!((up - Vec3::Y).length() < 1e-5, "{up:?}");
    assert!((model.transform.point(Vec3::ZERO).y - 300.0).abs() < 1e-3);
    assert!(scene.nodes[index("Variant")].hidden, "the variant is shown");
    assert!(scene.nodes[index("Panel(5)")].hidden, "an object KeyShot replaced with its own mesh is shown as well");
    let body = index("Body(1)");
    assert!(scene.nodes[body].mesh.is_none(), "an object of two materials is a group");
    let parts: Vec<_> = scene.nodes.iter().filter(|n| n.parent == Some(body)).collect();
    assert_eq!(parts.len(), 2, "one part per material");
    assert!(scene.nodes[index("Screen(2)")].mesh.is_some(), "an object of one material is a part");
    assert_eq!(scene.source_unit, Some(tequila_core::units::LengthUnit::Millimetre));
}

/// The faces are divided as their surfaces say: the square's hole is cut
/// out, the quarter cylinder's points are on the cylinder (its weights were
/// read as already multiplied in), and the faces' boundaries are the
/// model's edges.
#[test]
fn trimmed_nurbs_faces_are_divided_with_their_edges() {
    let scene = read();
    let mesh = |name: &str| {
        let node = scene.nodes.iter().find(|n| n.name.starts_with(name) && n.mesh.is_some()).unwrap_or_else(|| panic!("no part {name}"));
        &scene.meshes[node.mesh.unwrap()]
    };
    let screen = mesh("Screen(2)");
    let area: f32 = screen.indices.chunks(3).map(|t| {
        let [a, b, c] = [t[0], t[1], t[2]].map(|i| screen.positions[i as usize]);
        (b - a).cross(c - a).length() * 0.5
    }).sum();
    assert!((area - 10_000.0).abs() < 1.0, "the screen's square is {area} mm²");
    assert!(!screen.edge_lines.is_empty(), "no model edges");
    assert_eq!(screen.uvs.len(), screen.positions.len(), "a face's own UVs");

    // The body with the square, its hole and the cylinder, of the
    // multi-material's shown variant.
    let body = scene.nodes.iter().position(|n| n.name == "Body(1)").unwrap();
    let meshes: Vec<_> = scene.nodes.iter().filter(|n| n.parent == Some(body)).map(|n| &scene.meshes[n.mesh.unwrap()]).collect();
    let faces = meshes.iter().find(|m| m.positions.len() > 10).expect("the square and cylinder");
    let (mut flat, mut curved) = (0.0f32, 0usize);
    for t in faces.indices.chunks(3) {
        let p = [t[0], t[1], t[2]].map(|i| faces.positions[i as usize]);
        if p.iter().all(|q| q.z.abs() < 1e-3) {
            let n = (p[1] - p[0]).cross(p[2] - p[0]);
            flat += n.length() * 0.5;
            // Turned to look the way KeyShot's own mesh of it looks.
            assert!(n.z < 0.0, "the square looks up, where KeyShot's mesh of it looks down");
            assert!(faces.normals[t[0] as usize].z < 0.0, "its normals were not turned with it");
        }
    }
    // KeyShot's own mesh of the object is not drawn as well: the square,
    // less its hole, once.
    assert!((flat - 9_600.0).abs() < 1.0, "the square less its hole is {flat} mm²");
    assert!(scene.warnings.iter().any(|w| w.contains("faces were turned")), "{:?}", scene.warnings);
    // An object that is only a mesh is its mesh.
    let alone = scene.nodes.iter().find(|n| n.name == "Mesh(4)").and_then(|n| n.mesh).expect("the mesh object is a part");
    assert_eq!(scene.meshes[alone].indices.len(), 3);
    assert_eq!(scene.meshes[alone].uvs.len(), 3, "its own UVs");
    for p in faces.positions.iter().filter(|p| p.z > 1e-3) {
        curved += 1;
        let r = (p.x * p.x + p.z * p.z).sqrt();
        assert!((r - 40.0).abs() < 0.2, "a point on the cylinder is {r} from its axis");
    }
    assert!(curved > 8, "the cylinder is {curved} points");
}

/// Every kind of material comes over as its Tequila counterpart, its
/// colours decoded from KeyShot's sRGB and its roughness as stored.
#[test]
fn every_material_kind_maps() {
    let scene = read();
    let find = |name: &str| scene.materials.iter().find(|m| m.name == name).unwrap_or_else(|| panic!("no material {name}"));
    let grey = find("Grey");
    assert_eq!(grey.material_type, MaterialType::Plastic);
    assert!((grey.material().base_color.r - 0.214).abs() < 0.002, "{:?}", grey.material().base_color);
    assert!((grey.material().specular_roughness - 0.09).abs() < 1e-4);

    let paint = find("Paint");
    assert_eq!(paint.material_type, MaterialType::MetallicPaint);
    assert!(paint.material().coat_weight > 0.9 && (paint.material().coat_roughness - 0.04).abs() < 1e-4);
    assert!((paint.material().flake_roughness - 0.16).abs() < 1e-4 && (paint.material().flake_weight - 0.5).abs() < 1e-4);

    let chrome = find("Chrome");
    assert_eq!(chrome.material_type, MaterialType::Metal);
    assert_eq!(chrome.material().metal_ior, tequila_materials::fresnel::metals::CHROMIUM);

    let glass = find("Glass");
    assert_eq!(glass.material_type, MaterialType::GlassSolid);
    assert!((glass.material().transmission_depth - 0.5).abs() < 1e-6, "the distance its colour is reached at, in metres");

    // The glowing label: the screen's material glows by its picture, at
    // KeyShot's intensity, the picture placed by its frame.
    let screen = find("Screen");
    assert!((screen.material().emission_luminance - 5.0).abs() < 1e-5);
    let graph = &screen.graph;
    let driver = graph.links().iter().find(|l| l.to == graph.output() && l.to_socket == Socket::Param(Param::EmissionColor)).expect("the glow is driven");
    assert!(matches!(graph.node(driver.from).map(|n| &n.kind), Some(NodeKind::ImageTexture { .. })));
    let mapping = graph
        .nodes()
        .find_map(|(_, n)| match &n.kind {
            NodeKind::Mapping2d { mapping } => Some(*mapping),
            _ => None,
        })
        .expect("a placement");
    assert_eq!(mapping.projection, tequila_texture::projection::Projection::Planar);
    // A point on the label, in the part's own metres, lands where KeyShot's
    // frame puts it: a quarter across, three quarters up - a quarter down
    // from the top, where Tequila's v starts.
    let point = tequila_texture::projection::SurfacePoint {
        position: vec3(0.025, 0.075, 0.0),
        normal: Vec3::Z,
        uv: tequila_core::math::Vec2::ZERO,
        footprint: 1e-5,
        scale: 1.0,
        uv_footprint: None,
        curvature: 0.0,
        occlusion: 0.0,
        vertex_color: tequila_core::spectrum::Spectrum::ONE,
        world: Default::default(),
    };
    let at = mapping.project(&point)[0].coords;
    assert!((at.u - 0.25).abs() < 1e-4 && (at.v - 0.25).abs() < 1e-4, "the label lands at {} {}", at.u, at.v);
    assert_eq!(mapping.wrap_u, tequila_texture::mip::Wrap::Clamp, "a label does not tile");

    // A printed label is one of Tequila's labels, placed the same way.
    let printed = find("Printed");
    assert_eq!(printed.graph.decals.len(), 1);
    assert_eq!(printed.graph.decals[0].mapping.projection, tequila_texture::projection::Projection::Planar);

    // Fractal noise as a bump over a plastic.
    let grained = find("Grained");
    let bump = grained.graph.links().iter().find(|l| l.to == grained.graph.output() && l.to_socket == Socket::Param(Param::Bump));
    assert!(bump.is_some(), "the noise's bump is lost");

    // The pictures, read beside the scene, kept with the model.
    assert!(scene.textures.iter().any(|t| t.uri == "screen.png" && t.embedded.is_some()), "{:?}", scene.textures.iter().map(|t| &t.uri).collect::<Vec<_>>());
}

/// The multi-material becomes Tequila's switchable variants, its shown
/// variant on its parts.
#[test]
fn a_multi_material_becomes_variants() {
    let scene = read();
    let setup = scene.setup.as_ref().expect("a setup");
    assert_eq!(setup.multi_materials.len(), 1);
    let multi = &setup.multi_materials[0];
    assert_eq!(multi.name, "Colourway");
    let names: Vec<&str> = multi.variants.iter().map(|&v| scene.materials[v].name.as_str()).collect();
    assert_eq!(names, ["Grey", "Paint"]);
    assert_eq!(multi.active, 0);
    assert!(!multi.parts.is_empty());
    for &p in &multi.parts {
        let mesh = &scene.meshes[scene.nodes[p].mesh.unwrap()];
        assert_eq!(mesh.material, Some(multi.variants[0]));
    }
}

/// Opened, the scene is a project of its own: in metres, where KeyShot had
/// it, looking through the view it was saved in, lit by its pins.
#[test]
fn opening_a_package_gives_its_cameras_lighting_and_look() {
    let directory = std::env::temp_dir().join(format!("tequila-ksp-test-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("Speaker.ksp");
    let bip = scene();
    std::fs::write(&path, zip(&[("Speaker.bip", &bip), ("screen.png", &png())])).unwrap();
    let (document, report) = tequila_import::open_scene(&path, tequila_import::step::Tessellation::Draft).expect("opens");
    document.validate().unwrap();
    assert!(report.setup.is_some());

    // The picture came out of the package, beside it.
    assert!(directory.join("Speaker.textures").join("screen.png").is_file());

    // Where KeyShot stood it: its CAD origin 300 mm above the floor.
    let shown = document.tree.iter().filter(|n| matches!(n.kind, tequila_scene::NodeKind::Part { .. }) && document.tree.is_effectively_visible(n.id)).count();
    assert!(shown >= 3);
    let model = document.tree.iter().find(|n| n.name == "Model").expect("the model");
    let origin = document.tree.world_transform(model.id).point(Vec3::ZERO);
    assert!((origin.y - 0.3).abs() < 1e-5, "{origin:?}");

    // The cameras: the saved view active, a 50 mm lens on 36 mm film, depth
    // of field, an orthographic frame, positions in metres.
    let names: Vec<&str> = document.cameras.iter().map(|(_, c)| c.name.as_str()).collect();
    assert_eq!(names, ["Front", "Detail", "Side", "Viewport"]);
    assert_eq!(document.cameras.active().name, "Viewport");
    let front = &document.cameras.get(0).unwrap().camera;
    let Projection::Perspective { focal_length_mm } = front.projection else { panic!("perspective") };
    assert!((focal_length_mm - 50.0).abs() < 0.01, "{focal_length_mm}");
    assert!((front.position() - vec3(0.0, 0.3, 0.6)).length() < 1e-4, "{:?}", front.position());
    assert!((front.forward() - -Vec3::Z).length() < 1e-5);
    let detail = &document.cameras.get(1).unwrap().camera;
    assert!((detail.f_number - 8.0).abs() < 1e-5 && (detail.focus_distance - 0.4).abs() < 1e-5);
    let Projection::Orthographic { scale } = document.cameras.get(2).unwrap().camera.projection else { panic!("orthographic") };
    assert!((scale - 0.1125).abs() < 1e-5);
    let Projection::Perspective { focal_length_mm } = document.cameras.active().camera.projection else { panic!("perspective") };
    assert!((focal_length_mm - 80.0).abs() < 0.01);

    // The environment in use: its brightness and turn, a dome of its size,
    // its background over the sphere and its two pins over that, the image
    // style's white behind the product.
    let e = &document.environment;
    assert!((e.brightness - 1.5).abs() < 1e-6 && (e.rotation - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
    assert!((e.size - 4.0).abs() < 1e-4 && (e.ground.size - 2.0).abs() < 1e-4);
    assert_eq!(e.pins.len(), 3);
    assert_eq!(e.background, BackgroundMode::Color);
    let softbox = &e.pins[1];
    assert_eq!((softbox.name.as_str(), softbox.shape), ("Softbox", tequila_render_core::pins::PinShape::Rectangle));
    let expected = tequila_render_core::pins::direction_from(45f32.to_radians(), 30f32.to_radians());
    assert!((softbox.direction - expected).length() < 1e-5);
    assert!((softbox.angular_size.x - 20f32.to_radians()).abs() < 1e-5 && (softbox.intensity - 12.0).abs() < 1e-5);
    let strip = &e.pins[2];
    assert!(matches!(strip.kind, tequila_render_core::pins::PinKind::Gradient { .. }));
    assert!((strip.angular_size.x - 30f32.to_radians()).abs() < 1e-5);

    // Each named camera a Studio, with the environment of its name; the
    // other environment a Studio of its own.
    let front = document.studios.iter().find(|s| s.name == "Front").expect("a Front studio");
    assert!(front.camera.is_some() && front.environment.as_ref().is_some_and(|e| e.pins.len() == 3));
    assert!(document.studios.iter().any(|s| s.name == "Top" && s.camera.is_none() && s.environment.is_some()));

    assert!((document.image_style.exposure - 0.5).abs() < 1e-6 && (document.image_style.contrast - 1.1).abs() < 1e-5);
    assert_eq!((document.render.width, document.render.height, document.render.max_samples), (1280, 720, 64));
    assert_eq!(document.model_sets.sets.len(), 1);
    assert_eq!(document.model_sets.sets[0].name, "Default");
    assert_eq!(document.multi_materials.len(), 1);
    assert_eq!(document.units.unit, tequila_core::units::LengthUnit::Millimetre);
    let _ = std::fs::remove_dir_all(&directory);
}

/// Imported into another scene, the model comes in on its floor with its
/// variants, model set and cameras, the cameras moved with it; the scene it
/// lands in keeps its own lighting.
#[test]
fn importing_into_a_scene_adds_the_model_and_its_cameras() {
    let mut document = SceneDocument::new();
    let mut bus = CommandBus::new();
    let before = document.environment.clone();
    let (command, report) = tequila_import::build(&document, None, read(), &ImportOptions::default(), None).unwrap();
    bus.apply(&mut document, command).unwrap();
    for command in report.setup.as_ref().unwrap().commands(&document) {
        bus.apply(&mut document, command).unwrap();
    }
    document.validate().unwrap();
    assert!(document.bounds().min.y.abs() < 0.03, "not on the floor: {:?}", document.bounds());
    assert_eq!(document.cameras.len(), 5, "the scene's own camera and the four");
    // Lowered with the model: the Front camera looked level at 300 mm.
    let front = document.cameras.find("Front").and_then(|i| document.cameras.get(i)).unwrap();
    assert!((front.camera.position().y - (0.3 + report.lifted)).abs() < 1e-4);
    assert_eq!(document.multi_materials.len(), 1);
    assert_eq!(document.model_sets.sets.len(), 1);
    assert_eq!(document.environment, before);
}

/// Cut anywhere or with bytes changed anywhere, a scene reads as far as it
/// goes or is refused; it never panics.
#[test]
fn truncated_and_corrupted_scenes_never_panic() {
    let bytes = scene();
    let step = (bytes.len() / 400).max(1);
    for cut in (0..bytes.len()).step_by(step) {
        let _ = read_format(&bytes[..cut], "bip", "Cut", &picture);
    }
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    for _ in 0..300 {
        let mut corrupt = bytes.clone();
        for _ in 0..4 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let at = 0x80 + (seed as usize) % (corrupt.len() - 0x80);
            corrupt[at] ^= (seed >> 32) as u8 | 1;
        }
        let _ = read_format(&corrupt, "bip", "Corrupt", &picture);
    }
}
