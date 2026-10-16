//! The viewport's four shadings, as Blender's header offers them:
//! Wireframe, Solid, Material Preview and Rendered.
//!
//! The first three are drawn by the graphics card from the scene as it is
//! (`solid_gpu`), instantly, and the render thread stays idle; Rendered is
//! the path tracer. A machine with no usable OpenGL shows the CPU's Solid
//! for all three.

use crate::orchestrator::Shading;
use crate::solid_gpu::GpuMode;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewMode {
    Wireframe,
    #[default]
    Solid,
    MaterialPreview,
    Rendered,
}

impl ViewMode {
    pub const ALL: [ViewMode; 4] = [ViewMode::Wireframe, ViewMode::Solid, ViewMode::MaterialPreview, ViewMode::Rendered];

    pub fn label(self) -> &'static str {
        match self {
            ViewMode::Wireframe => "Wireframe",
            ViewMode::Solid => "Solid",
            ViewMode::MaterialPreview => "Material Preview",
            ViewMode::Rendered => "Rendered",
        }
    }

    /// Its name in scripts and in the scripting channel's `ui` request.
    pub fn key(self) -> &'static str {
        match self {
            ViewMode::Wireframe => "wireframe",
            ViewMode::Solid => "solid",
            ViewMode::MaterialPreview => "material",
            ViewMode::Rendered => "rendered",
        }
    }

    pub fn from_key(key: &str) -> Option<ViewMode> {
        match key {
            "wireframe" => Some(ViewMode::Wireframe),
            "solid" | "fast" => Some(ViewMode::Solid),
            "material" | "material_preview" | "lookdev" => Some(ViewMode::MaterialPreview),
            "rendered" => Some(ViewMode::Rendered),
            _ => None,
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            ViewMode::Wireframe => "Every edge of the model, the hidden ones faint · Z steps through the views",
            ViewMode::Solid => "Form and colour under a flat studio light, instantly · Z steps through the views",
            ViewMode::MaterialPreview => "Materials lit by the scene's environment on the graphics card: reflections, gloss and glass, instantly - a close guess at the render",
            ViewMode::Rendered => "Path traced: the image the render will produce · D",
        }
    }

    /// What the render thread does for it.
    pub fn shading(self) -> Shading {
        match self {
            ViewMode::Rendered => Shading::Rendered,
            _ => Shading::Fast,
        }
    }

    /// What the graphics card draws for it, if it draws it at all.
    pub fn gpu(self) -> Option<GpuMode> {
        match self {
            ViewMode::Wireframe => Some(GpuMode::Wireframe),
            ViewMode::Solid => Some(GpuMode::Solid),
            ViewMode::MaterialPreview => Some(GpuMode::MaterialPreview),
            ViewMode::Rendered => None,
        }
    }

    /// The next view along, as Z steps through them.
    pub fn next(self) -> ViewMode {
        match self {
            ViewMode::Wireframe => ViewMode::Solid,
            ViewMode::Solid => ViewMode::MaterialPreview,
            ViewMode::MaterialPreview => ViewMode::Rendered,
            ViewMode::Rendered => ViewMode::Wireframe,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_view_round_trips_by_key_and_cycles() {
        let mut mode = ViewMode::Solid;
        for _ in 0..4 {
            assert_eq!(ViewMode::from_key(mode.key()), Some(mode));
            mode = mode.next();
        }
        assert_eq!(mode, ViewMode::Solid, "four steps come back round");
        assert_eq!(ViewMode::Rendered.shading(), Shading::Rendered);
        assert!(ViewMode::ALL.iter().filter(|m| m.gpu().is_some()).count() == 3);
    }
}
