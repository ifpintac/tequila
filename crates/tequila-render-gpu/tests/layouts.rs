//! Every record that crosses to the card has one layout on both sides
//! (`docs/07` R2.2): naga reads the WGSL, and each struct's size and every
//! member's offset are held to Rust's `#[repr(C)]` twin. A field added on
//! one side only, or a vec3 that WGSL pads to sixteen bytes where Rust does
//! not, fails here rather than as a picture that is subtly wrong.

use std::mem::{offset_of, size_of};
use tequila_render_gpu::scene::{GpuInstance, GpuMesh};
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
    layout!(&module, GpuMesh { first_index, base_vertex, has_normals, vertex_count });
    layout!(&module, GpuInstance { object_to_world, world_to_object, mesh, material, instance, _pad });
    let source = shader::source("frame");
    let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
    layout!(
        &module,
        FrameParams {
            camera_to_world, half_height, half_width, width, height, sample_base, samples, scene_seed, max_bounces,
            rr_start, clamp_direct, clamp_indirect, shadow_far, env_rotation, env_inverse_rotation, env_width,
            env_height, env_tiles_x, env_tiles_y, env_intensity, env_weighted_sum, env_infinite, mode,
            off_luminance, off_cdf, off_power, off_rows, off_choice, off_tile_column, off_tile_row, off_row_start,
        }
    );
    layout!(&module, PixelSums { sum, background, squares_high, squares_low, samples, _pad });
}
