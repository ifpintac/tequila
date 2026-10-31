//! The material's textures that are laid on by the UV layout - its base
//! colour, roughness, bump and the rest - as layers under the labels: each
//! selected is placed by hand, its offset, repeats and turn, with handles on
//! the outline of one of its tiles.

use tequila_materials::graph::{MaterialGraph, TextureId};
use tequila_materials::material_type::Param;
use tequila_texture::projection::{Mapping, Projection};

/// One texture on the layout.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub param: Param,
    /// The picture, where an image drives it; a pattern has none.
    pub texture: Option<TextureId>,
    /// The file's name, or what drives it.
    pub name: String,
    pub mapping: Mapping,
}

/// Every parameter of `graph` driven through a mapping by the UV layout,
/// in the order the material's parameters are listed.
pub fn layers(graph: &MaterialGraph, name_of: impl Fn(TextureId) -> Option<String>) -> Vec<Layer> {
    Param::ALL
        .iter()
        .copied()
        .filter_map(|param| {
            let driver = graph.driver_of(param)?;
            let mapping = tequila_scene::command::mapping_of(graph, param)?;
            if mapping.projection != Projection::Uv {
                return None;
            }
            let texture = crate::material_tab::texture_behind(graph, driver);
            let name = texture.and_then(&name_of).unwrap_or_else(|| "Pattern".to_string());
            Some(Layer { param, texture, name, mapping })
        })
        .collect()
}
