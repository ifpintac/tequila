//! Texture references the document owns (`docs/02-MATERIAL-SYSTEM.md` §5.2).
//!
//! The document stores *where a texture came from and what it is for*, not its
//! pixels. Pixels live in the renderer's [`TextureCache`][cache], which is
//! bounded and evicts; a document that owned them would hold a scene's worth of
//! decoded 4K images in memory whether or not anything was rendering, and would
//! write them into the save file.
//!
//! So the split is: this store maps an id to a path and a role, and the renderer
//! resolves that id to pixels through its cache. Two materials pointing at the
//! same file share one id here, and therefore one decoded copy and one cache
//! entry there.
//!
//! ## Why the role is stored rather than inferred
//!
//! §5.2 assigns colour space by **map role**, not by file format, and a role
//! cannot be recovered from the file: `roughness.png` and `colour.png` are the
//! same kind of file and want opposite treatment. Storing it is what makes the
//! assignment survive a round trip through the document, which §5.2 asks for
//! explicitly — and it is why [`TextureSource::color_space`] exists as an
//! override rather than as the primary record. The role is the fact; the colour
//! space is derived from it unless someone has deliberately said otherwise.
//!
//! [cache]: tequila_texture::cache::TextureCache

use tequila_materials::graph::TextureId;
use tequila_texture::image::ColorSpace;
use tequila_texture::texture::MapRole;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Where a texture comes from and what it is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureSource {
    /// Where the file is. Relative paths are resolved against the project.
    pub path: PathBuf,
    /// What the map is used for — this decides the colour space (§5.2).
    pub role: MapRole,
    /// An explicit colour space, overriding the one the role implies.
    ///
    /// Present only when a person has said so. A scanned normal map that was
    /// mistakenly saved with a transfer curve is the case that needs it, and
    /// it is rare enough that inferring from the role has to be the default or
    /// everyone spends their time answering a question they do not have an
    /// opinion about.
    pub color_space: Option<ColorSpace>,
}

impl TextureSource {
    pub fn new(path: impl Into<PathBuf>, role: MapRole) -> TextureSource {
        TextureSource {
            path: path.into(),
            role,
            color_space: None,
        }
    }

    /// The colour space to decode with: the override if there is one, the
    /// role's otherwise.
    pub fn effective_color_space(&self) -> ColorSpace {
        self.color_space.unwrap_or(self.role.default_color_space())
    }

    /// The name to show in the panel: the file's own, without the directories.
    pub fn display_name(&self) -> String {
        self.path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("texture")
            .to_string()
    }
}

/// The document's texture references, deduplicated by path and role.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextureStore {
    sources: BTreeMap<TextureId, TextureSource>,
    next_id: u32,
}

impl TextureStore {
    pub fn new() -> TextureStore {
        TextureStore {
            sources: BTreeMap::new(),
            next_id: 1,
        }
    }

    pub fn len(&self) -> usize {
        self.sources.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    pub fn get(&self, id: TextureId) -> Option<&TextureSource> {
        self.sources.get(&id)
    }

    pub fn get_mut(&mut self, id: TextureId) -> Option<&mut TextureSource> {
        self.sources.get_mut(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (TextureId, &TextureSource)> {
        self.sources.iter().map(|(k, v)| (*k, v))
    }

    pub fn ids(&self) -> impl Iterator<Item = TextureId> + '_ {
        self.sources.keys().copied()
    }

    pub fn next_id(&self) -> u32 {
        self.next_id
    }

    pub fn set_next_id(&mut self, next: u32) {
        self.next_id = self.next_id.max(next);
    }

    /// Find an existing reference to the same file in the same role, or add one.
    ///
    /// Deduplicating here rather than at load time is what makes one file used
    /// by twelve materials cost one cache entry instead of twelve. The role is
    /// part of the key because the same file genuinely can be both — a
    /// greyscale image used as a colour on one part and as a mask on another
    /// decodes differently for each, and sharing the id would make one of them
    /// wrong.
    pub fn intern(&mut self, source: TextureSource) -> TextureId {
        if let Some((id, _)) = self
            .sources
            .iter()
            .find(|(_, existing)| existing.path == source.path && existing.role == source.role)
        {
            return *id;
        }
        let id = TextureId(self.next_id);
        self.next_id += 1;
        self.sources.insert(id, source);
        id
    }

    /// Insert under a chosen id, as the file reader and undo do.
    ///
    /// Redo has to restore an id that something already refers to; allocating a
    /// fresh one would leave the graph pointing at a texture that no longer
    /// exists. The allocator is only ever advanced, never rewound.
    pub fn insert_with_id(&mut self, id: TextureId, source: TextureSource) {
        self.next_id = self.next_id.max(id.0 + 1);
        self.sources.insert(id, source);
    }

    pub fn remove(&mut self, id: TextureId) -> Option<TextureSource> {
        self.sources.remove(&id)
    }

    /// Drop references nothing points at any more.
    pub fn retain_ids(&mut self, keep: &dyn Fn(TextureId) -> bool) -> usize {
        let before = self.sources.len();
        self.sources.retain(|&id, _| keep(id));
        before - self.sources.len()
    }

    /// Every distinct file the store refers to, for a pre-load pass.
    pub fn files(&self) -> Vec<&Path> {
        let mut paths: Vec<&Path> = self.sources.values().map(|s| s.path.as_path()).collect();
        paths.sort();
        paths.dedup();
        paths
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_file_in_one_role_gets_one_id() {
        let mut store = TextureStore::new();
        let a = store.intern(TextureSource::new("maps/panel.png", MapRole::Color));
        let b = store.intern(TextureSource::new("maps/panel.png", MapRole::Color));
        assert_eq!(a, b, "the same file in the same role was interned twice");
        assert_eq!(store.len(), 1);
    }

    /// The same file in a different role is a different texture, because it
    /// decodes differently. Sharing the id would make one of the two wrong.
    #[test]
    fn one_file_in_two_roles_gets_two_ids() {
        let mut store = TextureStore::new();
        let colour = store.intern(TextureSource::new("maps/grey.png", MapRole::Color));
        let mask = store.intern(TextureSource::new("maps/grey.png", MapRole::Data));

        assert_ne!(colour, mask);
        assert_eq!(
            store.get(colour).unwrap().effective_color_space(),
            ColorSpace::Srgb
        );
        assert_eq!(
            store.get(mask).unwrap().effective_color_space(),
            ColorSpace::Raw
        );
        // ...but it is still one file to load.
        assert_eq!(store.files().len(), 1);
    }

    /// §5.2: the role decides the colour space, and an override wins when
    /// somebody has deliberately set one.
    #[test]
    fn the_role_decides_the_colour_space_unless_overridden() {
        let mut source = TextureSource::new("maps/normal.png", MapRole::Normal);
        assert_eq!(source.effective_color_space(), ColorSpace::Raw);

        source.color_space = Some(ColorSpace::Srgb);
        assert_eq!(
            source.effective_color_space(),
            ColorSpace::Srgb,
            "an explicit override was ignored"
        );
    }

    /// Ids are never reused, so a stale reference is detectable rather than
    /// silently resolving to a different texture.
    #[test]
    fn ids_are_not_reused_after_removal() {
        let mut store = TextureStore::new();
        let first = store.intern(TextureSource::new("a.png", MapRole::Color));
        store.remove(first);
        let second = store.intern(TextureSource::new("b.png", MapRole::Color));
        assert_ne!(first, second);
    }

    /// Redo must restore a texture under the id the graph still refers to.
    #[test]
    fn inserting_with_an_id_advances_the_allocator() {
        let mut store = TextureStore::new();
        store.insert_with_id(TextureId(50), TextureSource::new("a.png", MapRole::Color));
        let next = store.intern(TextureSource::new("b.png", MapRole::Color));
        assert!(
            next.0 > 50,
            "a freshly allocated id collided with a restored one"
        );
    }

    #[test]
    fn iteration_is_deterministic() {
        let build = || {
            let mut store = TextureStore::new();
            for i in 0..8 {
                store.intern(TextureSource::new(format!("t{i}.png"), MapRole::Data));
            }
            store
        };
        assert_eq!(build(), build());
        assert_eq!(
            build().ids().collect::<Vec<_>>(),
            build().ids().collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_display_name_is_the_file_not_the_path() {
        let source = TextureSource::new("textures/panels/brushed_steel.png", MapRole::Color);
        assert_eq!(source.display_name(), "brushed_steel.png");
    }
}
