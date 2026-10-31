//! What is selected in the editor.
//!
//! Items are labels and textures today. The layout engine's islands,
//! vertices and edges join `Item` as variants of their own when its editing
//! is wired in: the canvas picks and the panels act on whatever the
//! selection holds, so a new kind of item is a new arm in each, not a new
//! selection.

use tequila_materials::material_type::Param;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Item {
    /// A label, by its place in the material's list, bottom first.
    Label(usize),
    /// A texture on the layout, by the parameter it drives.
    Texture(Param),
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
        self.items.contains(&item)
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Only `item`.
    pub fn set(&mut self, item: Item) {
        self.items.clear();
        self.items.push(item);
    }

    /// `item` as well. A texture is placed on its own, so adding one to
    /// labels, or a label to a texture, starts over with it.
    pub fn add(&mut self, item: Item) {
        let kind = |i: &Item| matches!(i, Item::Texture(_));
        if self.items.iter().any(|i| kind(i) != kind(&item)) || kind(&item) {
            self.set(item);
        } else if !self.contains(item) {
            self.items.push(item);
        }
    }

    /// In if it was out, out if it was in: a Shift-click.
    pub fn toggle(&mut self, item: Item) {
        if let Some(at) = self.items.iter().position(|i| *i == item) {
            self.items.remove(at);
        } else {
            self.add(item);
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

    /// Keep only what `keep` says still exists.
    pub fn retain(&mut self, keep: impl Fn(Item) -> bool) {
        self.items.retain(|i| keep(*i));
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
}
