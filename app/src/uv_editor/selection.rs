//! What is selected in the editor, and what a click picks.
//!
//! The mode says what a click picks: labels and textures, as stage one
//! had it, or the layout itself - its islands, edges or vertices, the
//! layout engine's. The canvas picks and the panels act on whatever the
//! selection holds, so each kind of item is an arm in each, not a
//! selection of its own; and the kinds never mix, so what a button acts on
//! is never in doubt.
//!
//! Layout items are named by numbers every edit keeps (`tequila_scene::uv`):
//! an island by one of its triangles - an island's own number moves when a
//! cut or a stitch renumbers them, a triangle's never does - a vertex by its
//! mesh vertex, an edge by the mesh vertices at its ends. So a selection
//! made before an edit still means the same thing after it.

use tequila_materials::material_type::Param;

/// What a click picks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub enum Mode {
    #[default]
    Labels,
    Islands,
    Edges,
    Vertices,
}

impl Mode {
    pub const ALL: [Mode; 4] = [Mode::Labels, Mode::Islands, Mode::Edges, Mode::Vertices];

    /// Its name on the scripting channel.
    pub fn key(self) -> &'static str {
        match self {
            Mode::Labels => "labels",
            Mode::Islands => "islands",
            Mode::Edges => "edges",
            Mode::Vertices => "vertices",
        }
    }

    pub fn from_key(key: &str) -> Option<Mode> {
        Mode::ALL.into_iter().find(|m| m.key() == key.to_ascii_lowercase())
    }

    /// Whether it picks the layout itself rather than what is placed on it.
    pub fn is_layout(self) -> bool {
        self != Mode::Labels
    }

    pub fn icon(self) -> crate::icons::Icon {
        use crate::icons::Icon;
        match self {
            Mode::Labels => Icon::Sticker,
            Mode::Islands => Icon::Islands,
            Mode::Edges => Icon::EdgeSelect,
            Mode::Vertices => Icon::VertexSelect,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Labels => "Labels",
            Mode::Islands => "Islands",
            Mode::Edges => "Edges",
            Mode::Vertices => "Vertices",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Mode::Labels => "Pick the labels and textures placed on the layout",
            Mode::Islands => "Pick whole islands of the layout, to move, turn, scale, pack and straighten",
            Mode::Edges => "Pick edges of the layout, to cut along or stitch",
            Mode::Vertices => "Pick vertices of the layout, to move or pin",
        }
    }

    pub fn key_name(self) -> &'static str {
        match self {
            Mode::Labels => "1",
            Mode::Islands => "2",
            Mode::Edges => "3",
            Mode::Vertices => "4",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Item {
    /// A label, by its place in the material's list, bottom first.
    Label(usize),
    /// A texture on the layout, by the parameter it drives.
    Texture(Param),
    /// An island, by its first triangle.
    Island(u32),
    /// An edge, by the mesh vertices at its ends, on the side picked.
    Edge([u32; 2]),
    /// A vertex of the layout, by a mesh vertex on it.
    Vertex(u32),
}

impl Item {
    /// Which kind it is: items of different kinds are not selected together.
    fn kind(&self) -> u8 {
        match self {
            Item::Label(_) => 0,
            Item::Texture(_) => 1,
            Item::Island(_) => 2,
            Item::Edge(_) => 3,
            Item::Vertex(_) => 4,
        }
    }

    /// The mode it is picked in.
    pub fn mode(&self) -> Mode {
        match self {
            Item::Label(_) | Item::Texture(_) => Mode::Labels,
            Item::Island(_) => Mode::Islands,
            Item::Edge(_) => Mode::Edges,
            Item::Vertex(_) => Mode::Vertices,
        }
    }
}

/// An edge named either way round is the same edge.
pub fn same_edge(a: [u32; 2], b: [u32; 2]) -> bool {
    a == b || a == [b[1], b[0]]
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    /// In the order they were selected; the last is the one the panels
    /// describe first.
    items: Vec<Item>,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn contains(&self, item: Item) -> bool {
        match item {
            Item::Edge(edge) => self.items.iter().any(|i| matches!(i, Item::Edge(e) if same_edge(*e, edge))),
            _ => self.items.contains(&item),
        }
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Only `item`.
    pub fn set(&mut self, item: Item) {
        self.items.clear();
        self.items.push(item);
    }

    /// `item` as well. Items of one kind go together; a texture is placed on
    /// its own. Adding one of another kind starts over with it.
    pub fn add(&mut self, item: Item) {
        let texture = matches!(item, Item::Texture(_));
        if self.items.iter().any(|i| i.kind() != item.kind()) || texture {
            self.set(item);
        } else if !self.contains(item) {
            self.items.push(item);
        }
    }

    /// In if it was out, out if it was in: a Shift-click.
    pub fn toggle(&mut self, item: Item) {
        let at = match item {
            Item::Edge(edge) => self.items.iter().position(|i| matches!(i, Item::Edge(e) if same_edge(*e, edge))),
            _ => self.items.iter().position(|i| *i == item),
        };
        match at {
            Some(at) => {
                self.items.remove(at);
            }
            None => self.add(item),
        }
    }

    /// The labels selected, bottom of the stack first.
    pub fn labels(&self) -> Vec<usize> {
        let mut out: Vec<usize> = self.items.iter().filter_map(|i| if let Item::Label(n) = i { Some(*n) } else { None }).collect();
        out.sort_unstable();
        out
    }

    pub fn texture(&self) -> Option<Param> {
        self.items.iter().find_map(|i| if let Item::Texture(p) = i { Some(*p) } else { None })
    }

    /// The islands selected, each by the triangle that names it.
    pub fn island_triangles(&self) -> Vec<u32> {
        self.items.iter().filter_map(|i| if let Item::Island(t) = i { Some(*t) } else { None }).collect()
    }

    pub fn edges(&self) -> Vec<[u32; 2]> {
        self.items.iter().filter_map(|i| if let Item::Edge(e) = i { Some(*e) } else { None }).collect()
    }

    pub fn vertices(&self) -> Vec<u32> {
        self.items.iter().filter_map(|i| if let Item::Vertex(v) = i { Some(*v) } else { None }).collect()
    }

    /// Whether what is selected is of the layout itself.
    pub fn is_layout(&self) -> bool {
        self.items.first().is_some_and(|i| i.mode().is_layout())
    }

    /// Keep only what `keep` says still exists.
    pub fn retain(&mut self, keep: impl Fn(Item) -> bool) {
        self.items.retain(|i| keep(*i));
    }

    /// Each item put through `map` - an island renamed by its first
    /// triangle after an edit - and what comes out twice kept once.
    pub fn rename(&mut self, map: impl Fn(Item) -> Option<Item>) {
        let mut out: Vec<Item> = Vec::with_capacity(self.items.len());
        for item in self.items.iter().filter_map(|i| map(*i)) {
            if !out.contains(&item) {
                out.push(item);
            }
        }
        self.items = out;
    }

    /// The labels at `from` moved to `to` in a list of `count`, as a
    /// reorder does: each selected label follows itself to its new place.
    pub fn reordered(&mut self, from: usize, to: usize) {
        for item in &mut self.items {
            if let Item::Label(n) = item {
                *n = moved_index(*n, from, to);
            }
        }
    }

    /// The labels after `removed` were taken out of the list.
    pub fn removed(&mut self, removed: &[usize]) {
        self.items.retain(|i| !matches!(i, Item::Label(n) if removed.contains(n)));
        for item in &mut self.items {
            if let Item::Label(n) = item {
                *n -= removed.iter().filter(|r| **r < *n).count();
            }
        }
    }

    /// Exactly these labels.
    pub fn set_labels(&mut self, labels: impl IntoIterator<Item = usize>) {
        self.items = labels.into_iter().map(Item::Label).collect();
    }

    /// Exactly these items, of the first one's kind, each once: a box or
    /// Ctrl+A over a layout of a hundred thousand edges is not a hundred
    /// thousand searches of the list.
    pub fn set_all(&mut self, items: impl IntoIterator<Item = Item>) {
        self.items.clear();
        let mut seen = std::collections::HashSet::new();
        for item in items {
            if self.items.first().is_some_and(|first| first.kind() != item.kind()) {
                continue;
            }
            let key = match item {
                Item::Edge([a, b]) => Item::Edge([a.min(b), a.max(b)]),
                other => other,
            };
            if seen.insert(key) {
                self.items.push(item);
            }
        }
    }

    /// These items as well, of the selection's kind.
    pub fn add_all(&mut self, items: impl IntoIterator<Item = Item>) {
        let kept = std::mem::take(&mut self.items);
        self.set_all(kept.into_iter().chain(items));
    }
}

/// Where the item at `n` ends up when the one at `from` moves to `to`.
pub fn moved_index(n: usize, from: usize, to: usize) -> usize {
    if n == from {
        to
    } else if from < to && n > from && n <= to {
        n - 1
    } else if to < from && n >= to && n < from {
        n + 1
    } else {
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_add_toggle_and_follow_their_places() {
        let mut selection = Selection::default();
        selection.set(Item::Label(2));
        selection.add(Item::Label(0));
        selection.toggle(Item::Label(4));
        assert_eq!(selection.labels(), [0, 2, 4]);
        selection.toggle(Item::Label(2));
        assert_eq!(selection.labels(), [0, 4]);
        // A texture is placed on its own.
        selection.add(Item::Texture(Param::BaseColor));
        assert_eq!(selection.items(), [Item::Texture(Param::BaseColor)]);
        selection.add(Item::Label(1));
        assert_eq!(selection.items(), [Item::Label(1)]);

        selection.set_labels([1, 3, 5]);
        selection.removed(&[0, 3]);
        assert_eq!(selection.labels(), [0, 3], "3 went; 1 and 5 each moved down past what was removed below them");
        selection.set_labels([0, 2]);
        selection.reordered(0, 3);
        assert_eq!(selection.labels(), [1, 3]);
        assert_eq!((moved_index(4, 4, 1), moved_index(1, 4, 1), moved_index(0, 4, 1)), (1, 2, 0));
    }

    /// Layout items of one kind go together; another kind starts over. An
    /// edge is the same edge named either way round.
    #[test]
    fn layout_items_keep_to_their_kind() {
        let mut selection = Selection::default();
        selection.set(Item::Island(4));
        selection.add(Item::Island(9));
        assert_eq!(selection.island_triangles(), [4, 9]);
        assert!(selection.is_layout());
        selection.add(Item::Edge([3, 7]));
        assert_eq!(selection.items(), [Item::Edge([3, 7])], "an edge starts over");
        assert!(selection.contains(Item::Edge([7, 3])));
        selection.toggle(Item::Edge([7, 3]));
        assert!(selection.is_empty(), "named the other way round, the same edge goes");
        selection.set_all([Item::Vertex(2), Item::Vertex(5), Item::Vertex(2)]);
        assert_eq!(selection.vertices(), [2, 5]);
        selection.add(Item::Label(0));
        assert!(!selection.is_layout());
        // Islands renamed after an edit: two that became one are one.
        selection.set_all([Item::Island(0), Item::Island(6)]);
        selection.rename(|item| match item {
            Item::Island(_) => Some(Item::Island(0)),
            other => Some(other),
        });
        assert_eq!(selection.items(), [Item::Island(0)]);
        for mode in Mode::ALL {
            assert_eq!(Mode::from_key(mode.key()), Some(mode));
        }
    }
}
