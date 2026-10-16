//! Material Templates (`docs/04` §3.3): rule-based bulk assignment.
//!
//! "Match by CAD colour, part name regex, metadata field, or layer, and
//! assign a material to every match. Show a preview table of what will be
//! assigned before committing. This is how a 4,000-part assembly gets
//! materials in a minute rather than a day."
//!
//! Names are matched with wildcards — `*` for any run of characters, `?` for
//! one — rather than regular expressions. A CAD part list is full of `.`,
//! `(` and `+`, each of which means something to a regex and nothing to the
//! person typing `bolt*`; and it keeps the project free of a regex engine.
//! A CAD layer arrives as a metadata field, so a metadata rule covers it.
//!
//! The first rule that matches a part decides it, as a list of exceptions
//! above a general case reads. Nothing is assigned until the plan is
//! applied, and applying it is one undo step.

use crate::document::SceneDocument;
use crate::tree::{MaterialKey, NodeId, NodeKind};
use crate::Command;
use tequila_core::spectrum::Spectrum;

/// What a rule looks for in a part.
#[derive(Clone, Debug, PartialEq)]
pub enum Condition {
    /// The part's name, by wildcard.
    Name(String),
    /// The name of the material the part wears now, by wildcard: how the
    /// material names a CAD exporter wrote are mapped onto real ones.
    Material(String),
    /// The base colour of the material the part wears now, within a
    /// tolerance: the CAD colour, which is what an importer turns a shape's
    /// colour into.
    Colour { color: Spectrum, tolerance: f32 },
    /// A metadata field — a CAD layer, a part number — by key, and its
    /// value by wildcard.
    Metadata { key: String, value: String },
}

impl Condition {
    pub fn label(&self) -> &'static str {
        match self {
            Condition::Name(_) => "Part name",
            Condition::Material(_) => "Material name",
            Condition::Colour { .. } => "Colour",
            Condition::Metadata { .. } => "Metadata",
        }
    }

    fn matches(&self, document: &SceneDocument, part: NodeId) -> bool {
        let Some(node) = document.tree.get(part) else {
            return false;
        };
        let material = node.material.and_then(|key| document.materials.get(key));
        match self {
            Condition::Name(pattern) => wildcard(pattern, &node.name),
            Condition::Material(pattern) => material.is_some_and(|m| wildcard(pattern, &m.name)),
            Condition::Colour { color, tolerance } => material.is_some_and(|m| {
                let c = m.material().base_color;
                (c.r - color.r).abs() <= *tolerance
                    && (c.g - color.g).abs() <= *tolerance
                    && (c.b - color.b).abs() <= *tolerance
            }),
            Condition::Metadata { key, value } => node
                .metadata
                .iter()
                .any(|(k, v)| k.eq_ignore_ascii_case(key) && wildcard(value, v)),
        }
    }
}

/// One rule: a condition and the material every part it matches gets.
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub condition: Condition,
    pub material: MaterialKey,
}

/// A named, ordered list of rules.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialTemplate {
    pub name: String,
    pub rules: Vec<Rule>,
}

/// One line of the preview: a part, what it wears, what it will wear, and
/// which rule decided.
#[derive(Clone, Debug, PartialEq)]
pub struct Planned {
    pub part: NodeId,
    pub from: Option<MaterialKey>,
    pub to: MaterialKey,
    pub rule: usize,
}

/// What applying `template` would do: every part a rule matches, by the
/// first rule that does, leaving out parts that already wear their target
/// and rules whose material the project no longer has.
pub fn plan(document: &SceneDocument, template: &MaterialTemplate) -> Vec<Planned> {
    let mut out = Vec::new();
    for id in document.tree.depth_first() {
        let Some(node) = document.tree.get(id) else { continue };
        if !matches!(node.kind, NodeKind::Part { .. }) {
            continue;
        }
        let decided = template
            .rules
            .iter()
            .enumerate()
            .find(|(_, rule)| rule.condition.matches(document, id));
        if let Some((index, rule)) = decided {
            if document.materials.get(rule.material).is_some() && node.material != Some(rule.material) {
                out.push(Planned {
                    part: id,
                    from: node.material,
                    to: rule.material,
                    rule: index,
                });
            }
        }
    }
    out
}

/// The commands that carry a plan out: one assignment per material, so a
/// thousand parts are a handful of commands. The caller applies them as
/// one undo step.
pub fn commands(plan: &[Planned]) -> Vec<Command> {
    let mut by_material: Vec<(MaterialKey, Vec<NodeId>)> = Vec::new();
    for line in plan {
        match by_material.iter_mut().find(|(m, _)| *m == line.to) {
            Some((_, parts)) => parts.push(line.part),
            None => by_material.push((line.to, vec![line.part])),
        }
    }
    by_material
        .into_iter()
        .map(|(material, nodes)| Command::AssignMaterial { nodes, material })
        .collect()
}

/// Case-insensitive wildcard match: `*` any run of characters, `?` any one.
pub fn wildcard(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    // The usual two-pointer match with backtracking to the last star.
    let (mut i, mut j) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while j < t.len() {
        if i < p.len() && (p[i] == '?' || p[i] == t[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some((i, j));
            i += 1;
        } else if let Some((si, sj)) = star {
            i = si + 1;
            j = sj + 1;
            star = Some((si, sj + 1));
        } else {
            return false;
        }
    }
    while i < p.len() && p[i] == '*' {
        i += 1;
    }
    i == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards_match_as_a_person_expects() {
        assert!(wildcard("bolt*", "Bolt M6x20"));
        assert!(wildcard("*screw*", "cap_SCREW.001"));
        assert!(wildcard("part-??", "part-12"));
        assert!(!wildcard("part-??", "part-123"));
        assert!(wildcard("*", ""));
        assert!(!wildcard("nut", "nuts"));
        // Characters a regex would read as syntax are only characters here.
        assert!(wildcard("housing (left).step", "Housing (Left).STEP"));
    }
}
