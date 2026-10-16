//! A label reaches the render on a part that has no UV coordinates.
//!
//! This is the claim the Labels section could not make. Every piece of the path
//! has its own test — the projection projects, the alpha survives the MIP
//! pyramid, the decal blends, the command fits the mapping — and all of them
//! passed while the feature did nothing on an STL, because none of them render
//! anything. The mesh here deliberately carries no UV set, which is what an STL
//! gives you and what most exported OBJ meshes give you.
//!
//! Two claims, and the second matters as much as the first: the artwork appears,
//! and it appears *only where it was placed*. A label that covers the whole part
//! is as wrong as one that never shows up, and a test that checks only for a
//! difference would pass for both.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Transform, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::decal::Decal;
use tequila_materials::graph::{MaterialGraph, TextureId};
use tequila_materials::material_type::MaterialType;
use tequila_render_core::environment::Environment;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::scene::{Instance, SceneSnapshot};
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_core::shade::TextureTable;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use tequila_texture::image::{ColorSpace, Image};
use tequila_texture::projection::Mapping;
use tequila_texture::texture::{MapRole, Texture};
use std::sync::Arc;

/// Artwork that is opaque red across its middle and transparent at its edges,
/// so where it lands and where it stops are both visible in the image.
fn artwork() -> Arc<Texture> {
    let size = 64;
    let mut data = vec![0.0f32; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let at = (y * size + x) * 4;
            let inside = (16..48).contains(&x) && (16..48).contains(&y);
            data[at] = 1.0;
            data[at + 3] = if inside { 1.0 } else { 0.0 };
        }
    }
    Arc::new(Texture::from_linear(
        Image {
            width: size,
            height: size,
            channels: 4,
            data,
            color_space: ColorSpace::Raw,
        },
        MapRole::Color,
        Mapping::default(),
    ))
}

/// A two-unit quad facing the camera, carrying **no UV coordinates** —
/// `with_uvs` is deliberately not called.
fn bare_quad() -> tequila_render_core::scene::Mesh {
    tequila_render_core::scene::Mesh::new(
        vec![
            vec3(-1.0, -1.0, 0.0),
            vec3(1.0, -1.0, 0.0),
            vec3(1.0, 1.0, 0.0),
            vec3(-1.0, 1.0, 0.0),
        ],
        vec![0, 1, 2, 0, 2, 3],
    )
    .with_normals(vec![Vec3::Z; 4])
}

fn render(label: Option<Decal>) -> Framebuffer {
    let mut graph = MaterialGraph::from_material(MaterialType::Diffuse.defaults());
    if let Some(label) = label {
        graph.decals.push(label);
    }

    let mut textures = TextureTable::new();
    textures.insert(TextureId(1), artwork());
    let scene = SceneSnapshot::new(
        vec![Arc::new(bare_quad())],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))
            .with_transform(Transform::IDENTITY)],
        vec![MaterialType::Diffuse.defaults()],
        Arc::new(Environment::uniform(Spectrum::splat(1.0))),
    )
    .with_programs(
        vec![Some(Arc::new(graph.compile().expect("compiles")))],
        Arc::new(textures),
    );

    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");

    let mut framebuffer = Framebuffer::new(64, 64);
    let context = RenderContext {
        camera: tequila_render_core::camera::Camera::look_at(
            vec3(0.0, 0.0, 3.2),
            Vec3::ZERO,
            Vec3::Y,
        ),
        ..RenderContext::default()
    };
    render_pass(
        &intersector,
        &mut framebuffer,
        &context,
        &ProgressiveSchedule::default(),
        32,
    );
    framebuffer
}

/// The label a person would get by dropping a PNG on this part: fitted by the
/// same code the Add Label command runs.
fn fitted_label() -> Decal {
    let bounds = tequila_core::math::Aabb {
        min: vec3(-1.0, -1.0, 0.0),
        max: vec3(1.0, 1.0, 0.0),
    };
    let mapping = tequila_scene::decal_fit::fit_to_bounds(bounds)
        .expect("a flat quad has a face to place a label on");
    // The quad is thinnest in Z, so the fit should have chosen to project
    // along Z. Stated here because the rest of the test depends on it.
    assert_eq!(mapping.axis, tequila_texture::projection::Axis::Z);

    let mut decal = Decal::new("Logo", TextureId(1));
    decal.mapping = mapping;
    decal
}

fn redness(image: &Framebuffer, x: usize, y: usize) -> f32 {
    let pixel = image.pixel(x, y).mean();
    pixel.r - (pixel.g + pixel.b) * 0.5
}

/// The artwork reaches the image, and stops where it was placed.
#[test]
fn a_fitted_label_lands_on_a_part_with_no_uvs() {
    let plain = render(None);
    let labelled = render(Some(fitted_label()));

    // The centre of the part is under the opaque middle of the artwork.
    let centre_before = redness(&plain, 32, 32);
    let centre_after = redness(&labelled, 32, 32);
    assert!(
        centre_after - centre_before > 0.25,
        "the middle of the part went from {centre_before} red to \
         {centre_after}; the label is not reaching the render"
    );

    // The corners of the part are outside the artwork entirely — the fit
    // covers 45% of a 2-unit face, so the artwork is 0.9 units across and
    // reaches nowhere near them. A label that tiled, or that ignored its
    // placement and covered the surface, would show here.
    for (x, y) in [(6usize, 6usize), (57, 6), (6, 57), (57, 57)] {
        let before = redness(&plain, x, y);
        let after = redness(&labelled, x, y);
        assert!(
            (after - before).abs() < 0.05,
            "the corner at ({x}, {y}) changed from {before} to {after}; the \
             artwork is not staying where it was placed"
        );
    }
}

/// The Labels pass holds the artwork alone: opaque where it was placed, and
/// empty everywhere else on the part, which a compositor needs so that
/// artwork can be swapped without rendering again.
#[test]
fn the_labels_pass_holds_the_artwork_alone() {
    use tequila_render_core::aov::AovSet;
    use tequila_render_core::framebuffer::AovBuffers;

    let mut graph = MaterialGraph::from_material(MaterialType::Diffuse.defaults());
    graph.decals.push(fitted_label());
    let mut textures = TextureTable::new();
    textures.insert(TextureId(1), artwork());
    let scene = SceneSnapshot::new(
        vec![Arc::new(bare_quad())],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))
            .with_transform(Transform::IDENTITY)],
        vec![MaterialType::Diffuse.defaults()],
        Arc::new(Environment::uniform(Spectrum::splat(1.0))),
    )
    .with_programs(
        vec![Some(Arc::new(graph.compile().expect("compiles")))],
        Arc::new(textures),
    );
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");

    let set = AovSet::standard();
    let mut framebuffer = Framebuffer::new(64, 64);
    let mut aovs = AovBuffers::new(64, 64, &set);
    let context = RenderContext {
        camera: tequila_render_core::camera::Camera::look_at(vec3(0.0, 0.0, 3.2), Vec3::ZERO, Vec3::Y),
        ..RenderContext::default()
    };
    let samples = 16;
    tequila_render_cpu::render_pass_with_aovs(
        &intersector,
        &mut framebuffer,
        Some((&mut aovs, &set)),
        &context,
        &ProgressiveSchedule::default(),
        samples,
    );
    let samples = framebuffer.min_samples();

    // The middle is under the opaque red of the artwork: red, and covered.
    let (colour, alpha) = aovs.mean_labels(32, 32, samples);
    let rgb = colour.to_linear_srgb();
    assert!(alpha > 0.95, "the label covers {alpha} of the middle");
    assert!(rgb.x > 0.9 && rgb.y < 0.05 && rgb.z < 0.05, "the middle's label colour is {rgb:?}");

    // The part's corners are outside the artwork: nothing there, and nothing
    // of the surface underneath leaks in.
    for (x, y) in [(6usize, 6usize), (57, 57)] {
        let (colour, alpha) = aovs.mean_labels(x, y, samples);
        assert!(alpha < 1e-4 && colour.to_linear_srgb().x.abs() < 1e-4, "({x}, {y}): {alpha}");
    }
}

/// Moving the part must take the label with it.
///
/// A projection anchored to the world would leave the artwork hanging in space
/// while the part slid out from under it — the same defect as a procedural
/// swimming, and more obvious, because a logo sliding off a product is not
/// something anyone has to squint at.
#[test]
fn a_projected_label_travels_with_its_part() {
    fn render_shifted(shift: Vec3) -> Framebuffer {
        let mut graph = MaterialGraph::from_material(MaterialType::Diffuse.defaults());
        graph.decals.push(fitted_label());
        let mut textures = TextureTable::new();
        textures.insert(TextureId(1), artwork());

        let transform = Transform::from_matrix(tequila_core::math::Mat4::translation(shift))
            .expect("a translation is invertible");
        let scene = SceneSnapshot::new(
            vec![Arc::new(bare_quad())],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))
                .with_transform(transform)],
            vec![MaterialType::Diffuse.defaults()],
            Arc::new(Environment::uniform(Spectrum::splat(1.0))),
        )
        .with_programs(
            vec![Some(Arc::new(graph.compile().expect("compiles")))],
            Arc::new(textures),
        );

        let device = Device::shared().expect("an Embree device");
        let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
        let mut framebuffer = Framebuffer::new(64, 64);
        let context = RenderContext {
            camera: tequila_render_core::camera::Camera::look_at(
                shift + vec3(0.0, 0.0, 3.2),
                shift,
                Vec3::Y,
            ),
            ..RenderContext::default()
        };
        render_pass(
            &intersector,
            &mut framebuffer,
            &context,
            &ProgressiveSchedule::default(),
            32,
        );
        framebuffer
    }

    let still = render_shifted(Vec3::ZERO);
    let moved = render_shifted(vec3(0.6, -0.35, 0.0));

    // The camera followed the part, so the label must occupy the same pixels.
    let mut worst = 0.0f32;
    for y in 0..64 {
        for x in 0..64 {
            worst = worst.max((redness(&still, x, y) - redness(&moved, x, y)).abs());
        }
    }
    assert!(
        worst < 0.08,
        "moving the part shifted the artwork by up to {worst} at a pixel; the \
         label is anchored to the world rather than to the part"
    );

    // The control: the label is actually in these images, so "identical"
    // above cannot be two pictures with no label in either.
    assert!(
        redness(&still, 32, 32) > 0.25,
        "no label in the render at all, so this test proves nothing"
    );
}
