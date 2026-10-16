//! Material instances and the linking model.
//!
//! `TEQUILA_SPEC.md` §1.4 calls this "the single most commonly misunderstood
//! part" of the product, and warns that getting it wrong "makes the app feel
//! broken in ways users can't articulate":
//!
//! > Materials are shared instances. Dropping a material onto a part assigns
//! > that material *instance* to the part; parts that already shared a
//! > material instance change together. "Unlink" creates an independent copy.
//!
//! So a material is stored once and referenced by key. Editing it changes
//! every part that shares it — that is the feature, not a bug — and
//! [`MaterialStore::unlink`] is the escape hatch.

use crate::tree::MaterialKey;
use tequila_core::spectrum::Spectrum;
use tequila_materials::graph::MaterialGraph;
use tequila_materials::material_type::{MaterialType, Param};
use tequila_render_core::material::Material;
use std::collections::BTreeMap;

/// A material as the document stores it: a name, a catalog type, and the
/// §2.2 parameter block.
///
/// The name lives here and only here. It was briefly duplicated onto
/// [`Material`] as well, and the two drifted: `unlink` and `material.rename`
/// each updated one of them, so an unlinked copy read as "Paint 2" in the
/// library and "Paint" everywhere that looked at the material itself. One
/// owner removes the possibility rather than the symptom.
///
/// [`MaterialDefinition::material_type`] is a *preset identity*, not a
/// shading mode. Nothing in the renderer branches on it — `docs/02` §1 is
/// explicit that there is one surface model and a type is only a set of
/// defaults. What it decides is which parameters the Material tab shows
/// (§04 §3), and it stays with the material so that reopening a file gives
/// back the panel the user was working in rather than a wall of all
/// thirty-seven.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialDefinition {
    pub name: String,
    pub material_type: MaterialType,
    /// The material, as a graph.
    ///
    /// `docs/02` §4.2 is emphatic that the simple parameter panel is a *view
    /// over the graph* rather than a second representation, so there is one
    /// stored thing and both views read it. A material with no textures is a
    /// graph holding a single surface node whose literals are the §2.2
    /// parameter block, which is what every material here starts as — so
    /// nothing pays for the graph until it has something in it.
    ///
    /// Reach the block through [`MaterialDefinition::material`] rather than
    /// storing a copy beside this. A copy is the thing §4.2 forbids: the two
    /// drift, and "I changed it in the graph and the panel still shows the old
    /// value" becomes unfixable rather than merely unfixed.
    pub graph: MaterialGraph,
}

impl MaterialDefinition {
    pub fn new(
        name: impl Into<String>,
        material_type: MaterialType,
        material: Material,
    ) -> Self {
        MaterialDefinition {
            name: name.into(),
            material_type,
            graph: MaterialGraph::from_material(material),
        }
    }

    /// Build from a graph directly, as the file reader does.
    pub fn from_graph(
        name: impl Into<String>,
        material_type: MaterialType,
        graph: MaterialGraph,
    ) -> Self {
        MaterialDefinition {
            name: name.into(),
            material_type,
            graph,
        }
    }

    /// The §2.2 parameter block — the surface node's literals.
    pub fn material(&self) -> &Material {
        self.graph.material()
    }

    pub fn material_mut(&mut self) -> &mut Material {
        self.graph.material_mut()
    }

    /// Whether this material needs more than the simple panel to show it.
    pub fn has_graph(&self) -> bool {
        self.graph.len() > 1 || !self.graph.decals.is_empty()
    }

    /// A material at its type's defaults, which is what dropping a fresh
    /// preset onto a part produces.
    pub fn preset(name: impl Into<String>, material_type: MaterialType) -> Self {
        MaterialDefinition::new(name, material_type, material_type.defaults())
    }

    pub fn plastic(name: impl Into<String>) -> Self {
        MaterialDefinition::new(
            name,
            MaterialType::Plastic,
            Material::plastic(Spectrum::splat(0.5), 0.2),
        )
    }

    /// The parameters the Material tab shows for this material (§04 §3).
    pub fn exposed(&self) -> Vec<Param> {
        self.material_type.exposed()
    }

    /// Switch to another catalog type, carrying across what still applies.
    ///
    /// Only the parameter block is rebuilt; the graph's other nodes and their
    /// links survive, because a texture driving base colour is still driving
    /// base colour after the type changes.
    ///
    /// `TEQUILA_SPEC.md` §1.4 calls for "best-effort parameter transfer", and
    /// the effort worth making is this: start from the new type's defaults,
    /// then re-apply every parameter that **both** types expose and that the
    /// user had moved away from the old type's default.
    ///
    /// The two conditions matter for different reasons. Only carrying exposed
    /// parameters stops a value the user could never see from following them
    /// around — switching Plastic to Metal should not silently drag a
    /// transmission depth along. Only carrying *edited* ones stops the old
    /// type's defaults from overwriting the new type's, which would make
    /// switching to Metal produce something that is not the Metal preset.
    pub fn retype(&mut self, material_type: MaterialType) {
        if material_type == self.material_type {
            return;
        }
        let old_defaults = self.material_type.defaults();
        let shared: Vec<Param> = material_type
            .exposed()
            .into_iter()
            .filter(|p| self.material_type.exposed().contains(p))
            .collect();

        let mut next = material_type.defaults();
        for param in shared {
            let (Some(current), Some(default)) =
                (param.read(self.material()), param.read(&old_defaults))
            else {
                continue;
            };
            if current != default {
                param.write(&mut next, current);
            }
        }

        self.material_type = material_type;
        *self.material_mut() = next;
    }
}

/// Deduplicated material storage.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MaterialStore {
    materials: BTreeMap<MaterialKey, MaterialDefinition>,
    next_key: u64,
}

impl MaterialStore {
    pub fn new() -> Self {
        MaterialStore {
            materials: BTreeMap::new(),
            next_key: 1,
        }
    }

    pub fn len(&self) -> usize {
        self.materials.len()
    }

    pub fn is_empty(&self) -> bool {
        self.materials.is_empty()
    }

    pub fn get(&self, key: MaterialKey) -> Option<&MaterialDefinition> {
        self.materials.get(&key)
    }

    pub(crate) fn get_mut(&mut self, key: MaterialKey) -> Option<&mut MaterialDefinition> {
        self.materials.get_mut(&key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (MaterialKey, &MaterialDefinition)> {
        self.materials.iter().map(|(k, m)| (*k, m))
    }

    pub fn keys(&self) -> impl Iterator<Item = MaterialKey> + '_ {
        self.materials.keys().copied()
    }

    pub fn next_key(&self) -> u64 {
        self.next_key
    }

    pub fn set_next_key(&mut self, next: u64) {
        self.next_key = self.next_key.max(next);
    }

    pub fn insert(&mut self, definition: MaterialDefinition) -> MaterialKey {
        let key = MaterialKey(self.next_key);
        self.next_key += 1;
        self.materials.insert(key, definition);
        key
    }

    pub fn insert_with_key(&mut self, key: MaterialKey, definition: MaterialDefinition) {
        self.next_key = self.next_key.max(key.0 + 1);
        self.materials.insert(key, definition);
    }

    pub(crate) fn remove(&mut self, key: MaterialKey) -> Option<MaterialDefinition> {
        self.materials.remove(&key)
    }

    /// Copy a material into a new, independent instance.
    ///
    /// This is "Unlink". The copy is named with a numeric suffix, as §1.4
    /// describes for the In-Project Library: "deduplicated, with numeric
    /// suffixes distinguishing unlinked copies".
    pub(crate) fn unlink(&mut self, key: MaterialKey) -> Option<MaterialKey> {
        let (copy, name) = self.unlinked_copy(key)?;
        Some(self.insert(MaterialDefinition { name, ..copy }))
    }

    /// Unlink into a chosen key rather than a freshly allocated one.
    ///
    /// Redo needs the copy to come back under the key it had the first time,
    /// or the node it was assigned to is left pointing at a key that no longer
    /// exists. See [`SceneTree::insert_with_id`](crate::tree::SceneTree).
    pub(crate) fn unlink_with_key(
        &mut self,
        key: MaterialKey,
        into: MaterialKey,
    ) -> Option<MaterialKey> {
        let (copy, name) = self.unlinked_copy(key)?;
        self.insert_with_key(into, MaterialDefinition { name, ..copy });
        Some(into)
    }

    fn unlinked_copy(&self, key: MaterialKey) -> Option<(MaterialDefinition, String)> {
        let original = self.get(key)?.clone();
        let name = self.next_available_name(&original.name);
        Some((original, name))
    }

    /// `"Paint"` -> `"Paint 2"`, `"Paint 2"` -> `"Paint 3"`, and so on.
    fn next_available_name(&self, base: &str) -> String {
        // Strip an existing suffix so unlinking twice gives "Paint 3", not
        // "Paint 2 2".
        let stem = match base.rsplit_once(' ') {
            Some((head, tail)) if tail.parse::<u32>().is_ok() && !head.is_empty() => head,
            _ => base,
        };
        let taken: std::collections::BTreeSet<&str> =
            self.materials.values().map(|m| m.name.as_str()).collect();
        for n in 2.. {
            let candidate = format!("{stem} {n}");
            if !taken.contains(candidate.as_str()) {
                return candidate;
            }
        }
        unreachable!("the loop is unbounded")
    }

    pub fn retain_keys(&mut self, keep: &dyn Fn(MaterialKey) -> bool) -> usize {
        let before = self.materials.len();
        self.materials.retain(|&key, _| keep(key));
        before - self.materials.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_materials::material_type::ParamValue;

    /// §1.4's "best-effort parameter transfer": an edit the user made and can
    /// still see must survive a type switch.
    #[test]
    fn switching_type_carries_across_what_both_types_expose() {
        let mut definition = MaterialDefinition::preset("Body", MaterialType::Plastic);
        // Roughness is exposed by Plastic and by Metal, and the user moved it.
        Param::SpecularRoughness.write(definition.material_mut(), ParamValue::Scalar(0.62));

        definition.retype(MaterialType::Metal);

        assert_eq!(definition.material_type, MaterialType::Metal);
        assert_eq!(
            Param::SpecularRoughness.read(definition.material()),
            Some(ParamValue::Scalar(0.62)),
            "an edit both types expose was lost in the switch"
        );
        // ...and the rest is genuinely the Metal preset, not Plastic wearing
        // its name.
        assert_eq!(
            definition.material().base_metalness,
            MaterialType::Metal.defaults().base_metalness
        );
    }

    /// An untouched parameter must **not** carry across, or the old type's
    /// defaults overwrite the new type's and switching to Metal produces
    /// something that is not the Metal preset.
    #[test]
    fn switching_type_does_not_drag_the_old_defaults_along() {
        let plastic = MaterialType::Plastic.defaults();
        let mut definition = MaterialDefinition::preset("Body", MaterialType::Plastic);
        assert_ne!(
            plastic.specular_roughness,
            MaterialType::Metal.defaults().specular_roughness,
            "the fixture needs the two presets to disagree"
        );

        definition.retype(MaterialType::Metal);

        assert_eq!(
            *definition.material(),
            MaterialType::Metal.defaults(),
            "an unedited material became something other than the target preset"
        );
    }

    /// A parameter the target type does not show must not follow the material
    /// around invisibly. Switching a glass to a metal should not leave a
    /// transmission depth behind that nothing in the panel can reach.
    #[test]
    fn switching_type_drops_parameters_the_new_type_cannot_show() {
        let mut definition = MaterialDefinition::preset("Bottle", MaterialType::GlassSolid);
        Param::TransmissionDepth.write(definition.material_mut(), ParamValue::Scalar(37.0));
        assert!(
            !MaterialType::Metal.exposed().contains(&Param::TransmissionDepth),
            "the fixture assumes Metal does not expose transmission depth"
        );

        definition.retype(MaterialType::Metal);

        assert_eq!(
            definition.material().transmission_depth,
            MaterialType::Metal.defaults().transmission_depth,
            "a hidden parameter survived a switch that cannot display it"
        );
    }

    #[test]
    fn switching_to_the_same_type_changes_nothing() {
        let mut definition = MaterialDefinition::preset("Body", MaterialType::Plastic);
        Param::SpecularRoughness.write(definition.material_mut(), ParamValue::Scalar(0.62));
        let before = definition.clone();
        definition.retype(MaterialType::Plastic);
        assert_eq!(definition, before);
    }

    #[test]
    fn materials_are_shared_instances() {
        let mut store = MaterialStore::new();
        let key = store.insert(MaterialDefinition::plastic("Paint"));
        // Two parts referencing one key see one material: editing it once
        // changes both, which is the linking model.
        assert_eq!(store.len(), 1);
        assert_eq!(store.get(key).unwrap().name, "Paint");
    }

    /// Unlinking must produce an *independent* copy — that is the whole point
    /// — and name it distinctly so the two are tellable apart in the library.
    #[test]
    fn unlinking_creates_an_independent_copy() {
        let mut store = MaterialStore::new();
        let original = store.insert(MaterialDefinition::plastic("Paint"));
        let copy = store.unlink(original).unwrap();

        assert_ne!(original, copy);
        assert_eq!(store.get(copy).unwrap().name, "Paint 2");
        assert_eq!(
            store.get(copy).unwrap().material(),
            store.get(original).unwrap().material(),
            "the copy starts identical"
        );

        // ...but is independent from here on.
        store.get_mut(copy).unwrap().material_mut().specular_roughness = 0.9;
        assert_ne!(
            store.get(copy).unwrap().material().specular_roughness,
            store.get(original).unwrap().material().specular_roughness
        );
    }

    #[test]
    fn repeated_unlinking_keeps_counting_rather_than_nesting() {
        let mut store = MaterialStore::new();
        let a = store.insert(MaterialDefinition::plastic("Paint"));
        let b = store.unlink(a).unwrap();
        let c = store.unlink(b).unwrap();
        let d = store.unlink(a).unwrap();

        assert_eq!(store.get(b).unwrap().name, "Paint 2");
        assert_eq!(store.get(c).unwrap().name, "Paint 3");
        assert_eq!(store.get(d).unwrap().name, "Paint 4");
    }

    #[test]
    fn unlinking_a_missing_material_reports_nothing() {
        let mut store = MaterialStore::new();
        assert!(store.unlink(MaterialKey(42)).is_none());
    }

    #[test]
    fn keys_are_not_reused_after_collection() {
        let mut store = MaterialStore::new();
        let a = store.insert(MaterialDefinition::plastic("A"));
        store.retain_keys(&|_| false);
        let b = store.insert(MaterialDefinition::plastic("B"));
        assert_ne!(a, b);
    }

    #[test]
    fn iteration_is_deterministic() {
        let build = || {
            let mut store = MaterialStore::new();
            for i in 0..16 {
                store.insert(MaterialDefinition::plastic(format!("m{i}")));
            }
            store
        };
        assert_eq!(build(), build());
        assert_eq!(
            build().keys().collect::<Vec<_>>(),
            build().keys().collect::<Vec<_>>()
        );
    }
}
