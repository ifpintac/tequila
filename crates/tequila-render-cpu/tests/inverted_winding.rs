//! Glass whose mesh is wound the wrong way round.
//!
//! A closed mesh can arrive from a CAD exporter or a glTF file wound
//! clockwise, and once it has there is nothing in the file to say so: the
//! geometry is the same solid, described inside out. Two separate things then
//! go wrong, and they look nothing alike.
//!
//! **The winding contradicts the vertex normals.** This is what an exporter
//! that reverses faces without touching the `NORMAL` attribute produces. The
//! geometric normal points in, the shading normal still points out, and
//! `facing_forward` — which turns both by the geometric one — hands the
//! integrator a shading frame facing away from the ray. The path is then
//! killed by the guard meant for grazing angles and **the object renders pure
//! black**, at full coverage, with nothing logged. This is the failure that
//! gets reported, and it is not about transmission at all: an opaque diffuse
//! sphere built the same way is just as black.
//!
//! **The winding and the normals are both reversed.** This is a genuinely
//! inside-out solid, and what recomputing normals from a reversed winding
//! leaves behind. Everything is self-consistent, so nothing is black — but
//! every face reports "inside", the relative IOR is inverted at every
//! crossing, and a glass ball renders as the air bubble that would be left if
//! you took it away. Roughly half again as bright, which reads as a lighting
//! choice rather than as a bug.
//!
//! Both are measured here against the same mesh wound correctly, which is the
//! only reference that separates "renders something" from "renders the right
//! thing" — asserting merely that the pixels are not black would pass on the
//! bubble.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Transform, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::MaterialType;
use tequila_materials::openpbr::Material;
use tequila_render_core::environment::Environment;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::scene::{Instance, Mesh, SceneSnapshot};
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use std::sync::Arc;

/// The sphere every case is built from: closed, and wound correctly.
fn sphere() -> Mesh {
    tequila_scene::demo::uv_sphere(1.0, 64, 32)
}

/// The same solid, described inside out.
///
/// `flip_normals` chooses between the two failures above: left alone, the
/// stored normals contradict the reversed winding; reversed with it, the mesh
/// is consistently inside out.
fn reversed(flip_normals: bool) -> Mesh {
    let source = sphere();
    let mut indices = source.indices.clone();
    for triangle in indices.chunks_exact_mut(3) {
        // Swapping any two of the three reverses the winding, and so the sign
        // of `cross(v1 - v0, v2 - v0)` that the geometric normal comes from.
        triangle.swap(1, 2);
    }
    let normals = if flip_normals {
        source.normals.iter().map(|n| -*n).collect()
    } else {
        source.normals.clone()
    };
    Mesh::new(source.positions.clone(), indices)
        .with_normals(normals)
        .with_uvs(source.uvs.clone())
}

/// Solid glass: not thin-walled, so it has two sides that have to be told
/// apart. A thin-walled pane would pass this test however it were wound,
/// because both of its sides are the same side.
fn solid_glass() -> Material {
    Material {
        transmission_weight: 1.0,
        transmission_color: Spectrum::ONE,
        transmission_depth: 0.0,
        geometry_thin_walled: false,
        specular_roughness: 0.0,
        specular_ior: 1.5,
        ..MaterialType::GlassSolid.defaults()
    }
}

/// `camera_z` inside the sphere puts the camera in the glass, which is the
/// case that has to keep working: it is the one place the winding is the only
/// evidence of which side a path is on.
fn render(mesh: Mesh, camera_z: f32) -> Framebuffer {
    let studio = tequila_scene::demo::studio_environment();
    let scene = SceneSnapshot::new(
        vec![Arc::new(mesh)],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))
            .with_transform(Transform::IDENTITY)],
        vec![solid_glass()],
        Arc::new(Environment::new(studio.image().clone(), 1.0)),
    );
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut framebuffer = Framebuffer::new(64, 64);
    let context = RenderContext {
        camera: tequila_render_core::camera::Camera::look_at(
            vec3(0.0, 0.0, camera_z),
            vec3(0.0, 0.0, -1.0),
            Vec3::Y,
        ),
        ..RenderContext::default()
    };
    render_pass(
        &intersector,
        &mut framebuffer,
        &context,
        &ProgressiveSchedule::default(),
        128,
    );
    framebuffer
}

/// The middle of the sphere, where the path through the glass is longest and
/// the side of each interface matters most.
fn centre(image: &Framebuffer) -> Spectrum {
    let mut total = Spectrum::ZERO;
    let mut n = 0.0;
    for y in 26..38 {
        for x in 26..38 {
            total = total + image.pixel(x, y).mean();
            n += 1.0;
        }
    }
    total * (1.0 / n)
}

/// A reversed winding that contradicts the vertex normals must not render
/// black.
///
/// The assertion worth having is the first one. A black render is not a
/// slightly wrong image that someone will notice and tune; it is an empty
/// frame with a perfect silhouette, which reads as "the material is broken"
/// and sends the search somewhere else entirely.
#[test]
fn a_reversed_winding_does_not_render_black() {
    let control = centre(&render(sphere(), 4.0));
    let reversed_winding = centre(&render(reversed(false), 4.0));

    assert!(
        control.luminance() > 1e-4,
        "the control sphere is black ({control:?}), so there is nothing to \
         compare against"
    );
    assert!(
        reversed_winding.luminance() > 1e-4,
        "a closed glass sphere wound the wrong way rendered black \
         ({reversed_winding:?}) — the shading frame was turned away from the \
         ray by a geometric normal that contradicts the vertex normals, and \
         every path died at its first vertex"
    );
}

/// And it renders the *same* glass, not merely something.
///
/// Held to a few percent rather than to equality: resolving the contradiction
/// towards the geometric normal costs the interpolated normal across each
/// triangle, so the reversed sphere shades as a faceted one. That is a
/// tessellation difference, not a transport one.
#[test]
fn a_reversed_winding_renders_the_same_glass_as_a_correct_one() {
    let control = centre(&render(sphere(), 4.0));
    let reversed_winding = centre(&render(reversed(false), 4.0));

    let ratio = reversed_winding.luminance() / control.luminance();
    assert!(
        (0.95..1.05).contains(&ratio),
        "a reversed winding transmitted {:.5} against the correctly wound \
         {:.5} ({ratio:.3}x)",
        reversed_winding.luminance(),
        control.luminance()
    );
}

/// A consistently inside-out solid is glass, not a bubble.
///
/// Nothing here is black, which is what makes this the dangerous one: reading
/// the side off the winding inverts the relative IOR at every crossing and
/// produces a plausible, brighter image. The ratio is what catches it — it
/// was 1.47 when the side came from the geometric normal.
#[test]
fn an_inside_out_solid_is_not_rendered_as_a_bubble() {
    let control = centre(&render(sphere(), 4.0));
    let inside_out = centre(&render(reversed(true), 4.0));

    let ratio = inside_out.luminance() / control.luminance();
    assert!(
        (0.95..1.05).contains(&ratio),
        "an inside-out glass sphere transmitted {:.5} against the correctly \
         wound {:.5} ({ratio:.3}x) — the relative IOR is being inverted at \
         every crossing, which turns a glass ball into the bubble its absence \
         would leave",
        inside_out.luminance(),
        control.luminance()
    );
}

/// A camera that starts inside the glass still sees the way out.
///
/// The guard on the fix rather than on the bug. Deciding the side from the
/// crossings the path has made is only right if the path knows where it
/// started, and a camera inside a glass object has made no crossings at all —
/// assuming it began in air would put every interface on the way out the
/// wrong way round, which is a regression this case would be the only one to
/// catch.
#[test]
fn a_camera_inside_the_glass_is_still_inside_it() {
    let inside = centre(&render(sphere(), 0.0));

    assert!(
        inside.luminance() > 0.03,
        "a camera inside a glass sphere rendered {:.5}, far darker than the \
         {:.3} a path leaving glass carries — it is being treated as entering \
         the glass from air at every interface on the way out",
        inside.luminance(),
        0.0559
    );
}
