//! Every record that crosses to the card has one layout on both sides
//! (`docs/07` R2.2): naga reads the WGSL, and each struct's size and every
//! member's offset are held to Rust's `#[repr(C)]` twin. A field added on
//! one side only, or a vec3 that WGSL pads to sixteen bytes where Rust does
//! not, fails here rather than as a picture that is subtly wrong.

use std::mem::{offset_of, size_of};
use tequila_render_gpu::emitters::GpuEmitterNode;
use tequila_render_gpu::lights::{GpuLight, GpuLightNode};
use tequila_render_gpu::material::GpuMaterial;
use tequila_render_gpu::program::GpuProgram;
use tequila_render_gpu::scene::{GpuCatcher, GpuInstance, GpuMesh};
use tequila_render_gpu::transport::{FrameParams, PixelSums};
use tequila_render_gpu::shader;

/// `name`'s size and its members' (name, offset), as naga lays it out.
fn wgsl_struct(module: &naga::Module, name: &str) -> (u32, Vec<(String, u32)>) {
    for (_, ty) in module.types.iter() {
        if ty.name.as_deref() == Some(name) {
            if let naga::TypeInner::Struct { members, span } = &ty.inner {
                return (*span, members.iter().map(|m| (m.name.clone().unwrap_or_default(), m.offset)).collect());
            }
        }
    }
    panic!("no struct {name} in the WGSL");
}

fn check(module: &naga::Module, name: &str, size: usize, fields: &[(&str, usize)]) {
    let (span, members) = wgsl_struct(module, name);
    let wgsl: Vec<(&str, u32)> = members.iter().map(|(n, o)| (n.as_str(), *o)).collect();
    let rust: Vec<(&str, u32)> = fields.iter().map(|(n, o)| (*n, *o as u32)).collect();
    assert_eq!(wgsl, rust, "{name}'s members differ between WGSL and Rust");
    assert_eq!(span as usize, size, "{name} is {span} bytes in WGSL and {size} in Rust");
}

macro_rules! layout {
    ($module:expr, $ty:ident { $($field:ident),* $(,)? }) => {
        check($module, stringify!($ty), size_of::<$ty>(), &[$((stringify!($field), offset_of!($ty, $field))),*])
    };
}

#[test]
fn records_have_one_layout_on_both_sides() {
    let source = shader::source("scene");
    let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
    layout!(&module, GpuMesh { first_index, base_vertex, has_normals, vertex_count, uv_base, tangent_base, attribute_base, attribute_flags, bounds_min, bounds_max });
    layout!(
        &module,
        GpuInstance {
            object_to_world, world_to_object, mesh, material, instance, catcher, light, flags, solid, emitter, object_color, inv_length_scale,
            scale, object_random, object_number,
        }
    );
    layout!(
        &module,
        GpuCatcher {
            shadow_color, bounce, reflections, reflection_roughness, reflection_fade, fade, reflection_fresnel,
            occlusion_only, occlusion_radius, shadows, bounce_falloff, _pad,
        }
    );
    let source = shader::source("frame");
    let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
    layout!(
        &module,
        FrameParams {
            camera_to_world, half_height, half_width, width, height, sample_base, samples, scene_seed, max_bounces,
            rr_start, clamp_direct, clamp_indirect, shadow_far, env_rotation, env_inverse_rotation, env_width,
            env_height, env_tiles_x, env_tiles_y, env_intensity, env_weighted_sum, env_infinite, mode,
            off_luminance, off_cdf, off_power, off_rows, off_choice, off_tile_column, off_tile_row, off_row_start,
            regularization_start, regularization_roughness, off_marginal, off_upward, env_upward_sum, shadows,
            ground_illumination, light_count, light_hierarchy, lamp_flags, pixel_spread, max_transmission_bounces, shadow_glass,
            glass_flags, row_base, row_stride, background, plate_offset, plate_width, plate_height, camera_flags,
            lens_radius, focus_distance, aperture_blades, aperture_rotation, _pad2,
        }
    );
    layout!(&module, PixelSums { sum, background, squares_high, squares_low, samples, _pad });
    let source = shader::source("openpbr");
    let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
    layout!(
        &module,
        GpuMaterial {
            base_color, metal_n, metal_k, specular_color, coat_color, fuzz_color, base_weight, base_metalness,
            base_diffuse_roughness, specular_weight, specular_roughness, specular_roughness_anisotropy, specular_ior,
            coat_weight, coat_roughness, coat_ior, coat_darkening, fuzz_weight, fuzz_roughness, geometry_occlusion, specular_rotation, geometry_opacity,
            emission_color, transmission_color, transmission_tint, volume_density, medium_priority, base_ior, glass, pane_weight,
            pane_opacity, pane_ior, pane_coat, pane_coat_ior, pane_depth, edge_radius, edge_samples, volume_glow,
        }
    );
    let source = shader::source("lights");
    let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
    layout!(
        &module,
        GpuLight {
            position, half_u, half_v, normal, radiance, rect_x, rect_y, rect_z, rect_origin, kind, flags, tan_half,
            spread_norm, falloff, fade, cos_a, cos_b, exponent, probability, alias_p, alias_alt, leaf, links, link_count,
            _pad,
        }
    );
    layout!(&module, GpuLightNode { centre, axis, cos_o, sin_o, cos_e, distant, left, right, parent, light });
    let source = shader::source("emitters");
    let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
    layout!(&module, GpuEmitterNode { centre, axis, a, b, c, normal, radiance, power, parent, left, right, instance, primitive, _pad });
    let source = shader::source("program");
    let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
    layout!(
        &module,
        GpuProgram { main_start, main_end, bump_start, bump_end, constants, writes_start, writes_count, bump_here, bump_height, tangent, flags, _pad }
    );
}

/// The processor's numbers the transport repeats as constants: a shadow
/// ray's footprint for a program at a crossing.
#[test]
fn constants_the_kernel_repeats_are_the_processors() {
    let source = shader::source("transport");
    let spread = tequila_render_core::settings::DEFAULT_PIXEL_SPREAD;
    let line = source.lines().find(|l| l.starts_with("const DEFAULT_PIXEL_SPREAD: f32 = ")).expect("the transport has it");
    let value: f32 = line.trim_start_matches("const DEFAULT_PIXEL_SPREAD: f32 = ").trim_end_matches(';').parse().expect("a number");
    assert_eq!(value, spread, "the transport's DEFAULT_PIXEL_SPREAD is {value}, settings.rs's {spread}");
}
