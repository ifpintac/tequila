//! The Configurator (`TEQUILA_SPEC.md` §1.13): Studios, Model Sets and
//! Multi-Materials turned into the questions a customer answers.
//!
//! A configurator is a list of **options** - "Colour", "Handle", "View" -
//! each with its **choices**. A choice switches one thing the document
//! already has: a Multi-Material's variant, the shown Model Set, or a
//! Studio. **Rules** say which choices go together: a choice can exclude
//! another option's choice ("the leather strap does not come in neon") or
//! require one ("the sport edition comes only in carbon"). The **layout**
//! says how the picker looks in Presentation mode and on the web.
//!
//! Rules are between two choices each, so whether a partial configuration
//! can still be completed is checked pair by pair, and finding the nearest
//! valid configuration to the one a customer just broke is a short search:
//! keep the choice they made, keep every other choice that still fits -
//! earlier options first - and change only what has to change.

use crate::document::SceneDocument;

/// What a choice switches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drives {
    /// The variant of the Multi-Material with this id; a choice's target
    /// is the variant's index.
    Material { multi_material: u64 },
    /// Which Model Set is shown; a choice's target is the set's index.
    ModelSet,
    /// Which Studio is applied; a choice's target is the Studio's index.
    Studio,
}

/// How an option's choices are drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Style {
    /// Round colour swatches, with the name underneath - for colours.
    #[default]
    Swatches,
    /// A row of buttons - for a handful of named versions.
    Buttons,
    /// A drop-down list - for long lists.
    List,
}

impl Style {
    pub const ALL: [Style; 3] = [Style::Swatches, Style::Buttons, Style::List];

    pub fn label(self) -> &'static str {
        match self {
            Style::Swatches => "Swatches",
            Style::Buttons => "Buttons",
            Style::List => "List",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Style::Swatches => "swatches",
            Style::Buttons => "buttons",
            Style::List => "list",
        }
    }

    pub fn from_key(key: &str) -> Option<Style> {
        Style::ALL.into_iter().find(|s| s.key() == key)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub label: String,
    /// The variant, set or Studio index this choice switches to.
    pub target: usize,
    /// The colour of its swatch, linear RGB; `None` takes it from what the
    /// choice switches to (a variant's base colour).
    pub swatch: Option<[f32; 3]>,
}

/// One question a customer answers.
#[derive(Clone, Debug, PartialEq)]
pub struct ConfigOption {
    /// Stable within the configurator, so rules can name it across edits.
    pub id: u64,
    pub name: String,
    pub drives: Drives,
    pub choices: Vec<Choice>,
    pub style: Style,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleKind {
    /// The first choice rules the second out.
    Excludes,
    /// The first choice needs the second.
    Requires,
}

/// "When `option`'s choice is `choice`, `other`'s choice `other_choice` is
/// excluded / required." Options by id, choices by index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule {
    pub option: u64,
    pub choice: usize,
    pub kind: RuleKind,
    pub other: u64,
    pub other_choice: usize,
}

/// Where the picker sits in Presentation mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Placement {
    Left,
    #[default]
    Right,
    Bottom,
}

impl Placement {
    pub const ALL: [Placement; 3] = [Placement::Left, Placement::Right, Placement::Bottom];

    pub fn label(self) -> &'static str {
        match self {
            Placement::Left => "Left",
            Placement::Right => "Right",
            Placement::Bottom => "Bottom",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Placement::Left => "left",
            Placement::Right => "right",
            Placement::Bottom => "bottom",
        }
    }

    pub fn from_key(key: &str) -> Option<Placement> {
        Placement::ALL.into_iter().find(|p| p.key() == key)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Configurator {
    /// The product's name, at the top of the picker.
    pub title: String,
    /// A line under it.
    pub subtitle: String,
    pub placement: Placement,
    pub options: Vec<ConfigOption>,
    pub rules: Vec<Rule>,
    /// The choice made for each option, in order: the configuration on show.
    pub current: Vec<usize>,
}

/// The most configurations [`Configurator::combinations`] lists.
pub const MOST_COMBINATIONS: usize = 10_000;

impl Configurator {
    pub fn is_empty(&self) -> bool {
        self.options.is_empty()
    }

    /// A fresh option id.
    pub fn next_id(&self) -> u64 {
        self.options.iter().map(|o| o.id).max().map_or(1, |m| m + 1)
    }

    pub fn index_of(&self, id: u64) -> Option<usize> {
        self.options.iter().position(|o| o.id == id)
    }

    /// Whether choosing `a` for option `i` and `b` for option `j` breaks a
    /// rule.
    fn clash(&self, i: usize, a: usize, j: usize, b: usize) -> bool {
        let (oi, oj) = (self.options[i].id, self.options[j].id);
        self.rules.iter().any(|rule| {
            let breaks = |first: u64, first_choice: usize, second: u64, second_choice: usize| {
                rule.option == first
                    && rule.choice == first_choice
                    && rule.other == second
                    && match rule.kind {
                        RuleKind::Excludes => rule.other_choice == second_choice,
                        RuleKind::Requires => rule.other_choice != second_choice,
                    }
            };
            breaks(oi, a, oj, b) || breaks(oj, b, oi, a)
        })
    }

    /// Whether every choice is in range and no rule is broken.
    pub fn is_valid(&self, choices: &[usize]) -> bool {
        choices.len() == self.options.len()
            && choices.iter().zip(&self.options).all(|(&c, o)| c < o.choices.len())
            && (0..choices.len()).all(|i| (i + 1..choices.len()).all(|j| !self.clash(i, choices[i], j, choices[j])))
    }

    /// The configuration nearest `from` with option `option` set to
    /// `choice`: what a customer gets when they pick something the rest of
    /// their configuration rules out. `None` when nothing valid has that
    /// choice.
    pub fn resolve(&self, from: &[usize], option: usize, choice: usize) -> Option<Vec<usize>> {
        if option >= self.options.len() || choice >= self.options[option].choices.len() {
            return None;
        }
        let mut order: Vec<usize> = (0..self.options.len()).collect();
        order.retain(|&i| i != option);
        order.insert(0, option);
        let mut assigned: Vec<Option<usize>> = vec![None; self.options.len()];
        let prefer = |i: usize| -> Vec<usize> {
            if i == option {
                return vec![choice];
            }
            let count = self.options[i].choices.len();
            let keep = from.get(i).copied().filter(|&c| c < count);
            keep.into_iter().chain((0..count).filter(move |&c| Some(c) != keep)).collect()
        };
        if self.search(&order, 0, &mut assigned, &prefer) {
            Some(assigned.into_iter().map(|c| c.unwrap_or(0)).collect())
        } else {
            None
        }
    }

    fn search(&self, order: &[usize], depth: usize, assigned: &mut Vec<Option<usize>>, prefer: &dyn Fn(usize) -> Vec<usize>) -> bool {
        let Some(&i) = order.get(depth) else {
            return true;
        };
        for c in prefer(i) {
            let fits = assigned.iter().enumerate().all(|(j, a)| a.is_none_or(|b| !self.clash(i, c, j, b)));
            if fits {
                assigned[i] = Some(c);
                if self.search(order, depth + 1, assigned, prefer) {
                    return true;
                }
                assigned[i] = None;
            }
        }
        false
    }

    /// The current configuration if it is valid, else the nearest valid one
    /// to it; `None` when the rules allow nothing at all.
    pub fn settled(&self) -> Option<Vec<usize>> {
        if self.options.is_empty() {
            return Some(Vec::new());
        }
        if self.is_valid(&self.current) {
            return Some(self.current.clone());
        }
        let from: Vec<usize> = (0..self.options.len()).map(|i| self.current.get(i).copied().unwrap_or(0)).collect();
        // The first option keeps its choice if anything valid has it.
        std::iter::once(from[0]).chain(0..self.options[0].choices.len()).find_map(|c| self.resolve(&from, 0, c))
    }

    /// How many configurations there would be without the rules.
    pub fn product(&self) -> usize {
        self.options.iter().fold(1usize, |n, o| n.saturating_mul(o.choices.len()))
    }

    /// Every valid configuration, in order, up to [`MOST_COMBINATIONS`].
    pub fn combinations(&self) -> Vec<Vec<usize>> {
        let mut out = Vec::new();
        if self.options.iter().any(|o| o.choices.is_empty()) {
            return out;
        }
        let mut current = Vec::with_capacity(self.options.len());
        self.enumerate(&mut current, &mut out);
        out
    }

    fn enumerate(&self, current: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        if out.len() >= MOST_COMBINATIONS {
            return;
        }
        let i = current.len();
        if i == self.options.len() {
            out.push(current.clone());
            return;
        }
        for c in 0..self.options[i].choices.len() {
            if current.iter().enumerate().all(|(j, &b)| !self.clash(i, c, j, b)) {
                current.push(c);
                self.enumerate(current, out);
                current.pop();
            }
        }
    }

    /// The choices of `option` no valid configuration has: shown greyed
    /// out, or a picker would offer what cannot be had.
    pub fn impossible(&self, option: usize) -> Vec<bool> {
        let Some(o) = self.options.get(option) else {
            return Vec::new();
        };
        let from = self.current.clone();
        (0..o.choices.len()).map(|c| self.resolve(&from, option, c).is_none()).collect()
    }

    /// A configuration's choices as words, for a file name or a caption:
    /// "Red, Leather, Front".
    pub fn describe(&self, choices: &[usize]) -> Vec<String> {
        self.options
            .iter()
            .zip(choices)
            .map(|(o, &c)| o.choices.get(c).map(|c| c.label.clone()).unwrap_or_default())
            .collect()
    }

    /// Problems with what the configurator points at in `document`: a
    /// Multi-Material, set or Studio deleted since, a variant past the end.
    /// Empty when everything resolves.
    pub fn problems(&self, document: &SceneDocument) -> Vec<String> {
        let mut out = Vec::new();
        for option in &self.options {
            let limit = match option.drives {
                Drives::Material { multi_material } => {
                    match document.multi_materials.iter().find(|m| m.id == multi_material) {
                        Some(m) => m.variants.len(),
                        None => {
                            out.push(format!("“{}” switches a Multi-Material that no longer exists", option.name));
                            continue;
                        }
                    }
                }
                Drives::ModelSet => document.model_sets.sets.len(),
                Drives::Studio => document.studios.len(),
            };
            for choice in &option.choices {
                if choice.target >= limit {
                    out.push(format!("“{}” in “{}” points past the end of what it switches", choice.label, option.name));
                }
            }
            if option.choices.is_empty() {
                out.push(format!("“{}” has no choices", option.name));
            }
        }
        for rule in &self.rules {
            if self.index_of(rule.option).is_none() || self.index_of(rule.other).is_none() {
                out.push("A rule names an option that was removed".into());
            }
        }
        if !self.options.is_empty() && self.settled().is_none() {
            out.push("The rules leave no configuration that works".into());
        }
        out
    }

    /// An option for each Multi-Material, then one for the Model Sets and
    /// one for the Studios when there are two or more: the first draft the
    /// wizard starts from.
    pub fn draft(document: &SceneDocument) -> Configurator {
        let mut c = Configurator { title: document.meta.name.clone(), ..Default::default() };
        for multi in &document.multi_materials {
            let choices = multi
                .variants
                .iter()
                .enumerate()
                .map(|(i, key)| Choice {
                    label: document.materials.get(*key).map(|m| m.name.clone()).unwrap_or_else(|| format!("Variant {}", i + 1)),
                    target: i,
                    swatch: None,
                })
                .collect();
            let id = c.next_id();
            c.options.push(ConfigOption { id, name: multi.name.clone(), drives: Drives::Material { multi_material: multi.id }, choices, style: Style::Swatches });
        }
        if document.model_sets.sets.len() >= 2 {
            let choices = document.model_sets.sets.iter().enumerate().map(|(i, s)| Choice { label: s.name.clone(), target: i, swatch: None }).collect();
            let id = c.next_id();
            c.options.push(ConfigOption { id, name: "Model".into(), drives: Drives::ModelSet, choices, style: Style::Buttons });
        }
        if document.studios.len() >= 2 {
            let choices = document.studios.iter().enumerate().map(|(i, s)| Choice { label: s.name.clone(), target: i, swatch: None }).collect();
            let id = c.next_id();
            c.options.push(ConfigOption { id, name: "View".into(), drives: Drives::Studio, choices, style: Style::Buttons });
        }
        c.current = vec![0; c.options.len()];
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn option(id: u64, name: &str, choices: &[&str]) -> ConfigOption {
        ConfigOption {
            id,
            name: name.into(),
            drives: Drives::ModelSet,
            choices: choices.iter().enumerate().map(|(i, l)| Choice { label: (*l).into(), target: i, swatch: None }).collect(),
            style: Style::Buttons,
        }
    }

    /// A watch: colour (Black, Neon, Gold), strap (Rubber, Leather), case
    /// (Steel, Carbon). Leather excludes Neon; Carbon requires Rubber.
    fn watch() -> Configurator {
        Configurator {
            options: vec![
                option(1, "Colour", &["Black", "Neon", "Gold"]),
                option(2, "Strap", &["Rubber", "Leather"]),
                option(3, "Case", &["Steel", "Carbon"]),
            ],
            rules: vec![
                Rule { option: 2, choice: 1, kind: RuleKind::Excludes, other: 1, other_choice: 1 },
                Rule { option: 3, choice: 1, kind: RuleKind::Requires, other: 2, other_choice: 0 },
            ],
            current: vec![0, 0, 0],
            ..Default::default()
        }
    }

    #[test]
    fn rules_decide_what_is_valid() {
        let w = watch();
        assert!(w.is_valid(&[0, 1, 0]), "black leather steel");
        assert!(!w.is_valid(&[1, 1, 0]), "neon leather is excluded");
        assert!(!w.is_valid(&[0, 1, 1]), "carbon needs rubber");
        assert!(w.is_valid(&[1, 0, 1]), "neon rubber carbon");
        assert!(!w.is_valid(&[0, 0]), "a choice short");
        assert!(!w.is_valid(&[3, 0, 0]), "past the last colour");
    }

    /// 3 × 2 × 2 = 12 without rules; neon-leather takes 2, carbon-leather
    /// takes 3 more (one shared): 8 left.
    #[test]
    fn every_valid_combination_is_listed_once() {
        let w = watch();
        let all = w.combinations();
        assert_eq!(w.product(), 12);
        assert_eq!(all.len(), 8, "{all:?}");
        assert!(all.iter().all(|c| w.is_valid(c)));
        let unique: std::collections::BTreeSet<_> = all.iter().collect();
        assert_eq!(unique.len(), all.len());
    }

    /// Picking leather on a neon carbon watch keeps leather - the choice
    /// just made - and changes what has to change, keeping earlier options
    /// where it can: the colour goes to black (neon is out), the case to
    /// steel (carbon needs rubber).
    #[test]
    fn a_broken_configuration_is_mended_nearby() {
        let w = watch();
        assert_eq!(w.resolve(&[1, 0, 1], 1, 1), Some(vec![0, 1, 0]));
        // A pick that breaks nothing changes nothing else.
        assert_eq!(w.resolve(&[2, 0, 1], 0, 0), Some(vec![0, 0, 1]));
    }

    #[test]
    fn a_choice_no_configuration_has_is_impossible() {
        let mut w = watch();
        // Gold excludes both straps: gold can never be had.
        w.rules.push(Rule { option: 1, choice: 2, kind: RuleKind::Excludes, other: 2, other_choice: 0 });
        w.rules.push(Rule { option: 1, choice: 2, kind: RuleKind::Excludes, other: 2, other_choice: 1 });
        assert_eq!(w.impossible(0), vec![false, false, true]);
        assert_eq!(w.resolve(&[0, 0, 0], 0, 2), None);
    }

    #[test]
    fn an_invalid_current_settles_on_a_valid_one() {
        let mut w = watch();
        w.current = vec![1, 1, 1];
        let settled = w.settled().expect("something works");
        assert!(w.is_valid(&settled), "{settled:?}");
        assert_eq!(settled[0], 1, "the first option keeps its choice where it can");
    }
}
