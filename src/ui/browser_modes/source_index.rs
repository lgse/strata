// SPDX-License-Identifier: MIT

//! Maps between a pane's directory model and the filtered, sorted, or grouped
//! view models layered over it.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

use gtk::{gio, glib, prelude::*};

use super::grouped_model::GroupedModel;

/// A view's source positions by view position, built for the two maps' generations.
type ViewSources = ((u64, u64), Vec<u32>);
pub(super) const SPARSE_LOOKUP_LIMIT: usize = 64;
const UNMAPPED: u32 = u32::MAX;

/// One `items-changed` notification of a watched model.
#[derive(Clone, Copy)]
struct ItemsChange {
    position: u32,
    removed: u32,
    added: u32,
}

/// Filter, sort, and flatten models preserve item identity across position changes.
#[derive(Clone, Default)]
pub(super) struct SourceIndexMap {
    model: glib::WeakRef<gio::ListModel>,
    by_item: Rc<RefCell<Option<HashMap<glib::Object, usize>>>>,
    generation: Rc<Cell<u64>>,
    // Reading a filter model's item walks its match set, so selections map through this.
    sources: Rc<RefCell<Option<ViewSources>>>,
}

impl SourceIndexMap {
    pub(super) fn watch(source: &impl IsA<gio::ListModel>) -> Self {
        let source = source.as_ref();
        let map = Self {
            model: source.downgrade(),
            ..Self::default()
        };
        let tracked = map.clone();
        // Capturing the model here would keep it alive through its own signal.
        source.connect_items_changed(move |source, position, removed, added| {
            tracked.apply(
                source,
                ItemsChange {
                    position,
                    removed,
                    added,
                },
            );
        });
        if source.n_items() == 0 || !source.is::<GroupedModel>() {
            map.ensure_current();
        }
        map
    }

    fn apply(&self, source: &gio::ListModel, change: ItemsChange) {
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        let total = source.n_items();
        if total == 0 {
            *self.by_item.borrow_mut() = Some(HashMap::new());
            return;
        }
        if !self.can_append(change, total) {
            // A grouped insertion can replace almost the whole view. Coalesce
            // those ranges until a caller actually needs a reverse lookup.
            self.by_item.borrow_mut().take();
            return;
        }
        let arrivals: Vec<_> = (change.position..change.position.saturating_add(change.added))
            .filter_map(|index| source.item(index).map(|item| (item, index as usize)))
            .collect();
        if self.generation.get() == generation
            && let Some(by_item) = self.by_item.borrow_mut().as_mut()
        {
            by_item.extend(arrivals);
        }
    }

    fn can_append(&self, change: ItemsChange, total: u32) -> bool {
        change.removed == 0
            && change.position.saturating_add(change.added) == total
            && self
                .by_item
                .borrow()
                .as_ref()
                .is_some_and(|by_item| change.position == by_item.len() as u32)
    }

    pub(super) fn ensure_current(&self) -> bool {
        let Some(source) = self.model.upgrade() else {
            return false;
        };
        if self.by_item.borrow().is_some() {
            return true;
        }
        let generation = self.generation.get();
        let mut by_item = HashMap::with_capacity(source.n_items() as usize);
        for position in 0..source.n_items() {
            if let Some(item) = source.item(position) {
                by_item.insert(item, position as usize);
            }
        }
        // A custom model may emit another change while resolving its items.
        if self.generation.get() != generation {
            return false;
        }
        *self.by_item.borrow_mut() = Some(by_item);
        true
    }

    pub(super) fn of_item(&self, item: &glib::Object) -> Option<usize> {
        let model = self.model.upgrade()?;
        if let Some(by_item) = self.by_item.borrow().as_ref() {
            return by_item.get(item).copied();
        }
        if let Some(grouped) = model.downcast_ref::<GroupedModel>() {
            return grouped.position_of(item).map(|position| position as usize);
        }
        if !self.ensure_current() {
            return None;
        }
        self.by_item.borrow().as_ref()?.get(item).copied()
    }

    pub(super) fn of_view_position(
        &self,
        view: &impl IsA<gio::ListModel>,
        position: u32,
    ) -> Option<usize> {
        view.item(position).and_then(|item| self.of_item(&item))
    }

    /// Positions in `source` of this map's `view` items at `positions`.
    pub(super) fn sources_of(
        &self,
        source: &SourceIndexMap,
        view: &gio::ListModel,
        positions: impl Iterator<Item = u32>,
    ) -> Vec<Option<usize>> {
        let mut positions = positions;
        let first: Vec<_> = positions.by_ref().take(SPARSE_LOOKUP_LIMIT + 1).collect();
        // A cursor or small selection must not rebuild the entire view index.
        if first.len() <= SPARSE_LOOKUP_LIMIT {
            return first
                .into_iter()
                .map(|position| source.of_view_position(view, position))
                .collect();
        }
        let positions = first.into_iter().chain(positions);
        if !self.ensure_current() || !source.ensure_current() {
            return positions.map(|_| None).collect();
        }
        let key = (self.generation.get(), source.generation.get());
        let mut cache = self.sources.borrow_mut();
        let forward = match &mut *cache {
            Some((built, forward)) if *built == key => forward,
            slot => {
                let Some(forward) = self.forward_map(source, view) else {
                    return positions.map(|_| None).collect();
                };
                &mut slot.insert((key, forward)).1
            }
        };
        positions
            .map(|position| match forward.get(position as usize) {
                Some(&source) if source != UNMAPPED => Some(source as usize),
                _ => source.of_view_position(view, position),
            })
            .collect()
    }

    /// Every view position's source position, `UNMAPPED` where the view item is unknown.
    fn forward_map(&self, source: &SourceIndexMap, view: &gio::ListModel) -> Option<Vec<u32>> {
        let mut forward = vec![UNMAPPED; view.n_items() as usize];
        let by_source_item = source.by_item.borrow();
        let by_view_item = self.by_item.borrow();
        let (Some(by_source_item), Some(by_view_item)) =
            (by_source_item.as_ref(), by_view_item.as_ref())
        else {
            return None;
        };
        for (item, position) in by_view_item {
            let Some(slot) = forward.get_mut(*position) else {
                continue;
            };
            if let Some(source) = by_source_item
                .get(item)
                .and_then(|source| u32::try_from(*source).ok())
            {
                *slot = source;
            }
        }
        Some(forward)
    }
}

/// A section's view model and the map back to the pane's directory entries.
#[derive(Clone)]
pub(super) struct PanePositions {
    pub(super) index: SourceIndexMap,
    pub(super) view: gio::ListModel,
}

impl PanePositions {
    pub(super) fn source_position(&self, view_position: u32) -> Option<usize> {
        source_position_for_view(&self.index, Some(&self.view), view_position)
    }

    pub(super) fn view_position(&self, source_position: usize) -> Option<u32> {
        let guessed = source_position as u32;
        // Unfiltered views keep source order; a non-source prefix shifts positions.
        [guessed, guessed.saturating_add(1)]
            .into_iter()
            .find(|candidate| self.source_position(*candidate) == Some(source_position))
            .or_else(|| {
                (0..self.view.n_items())
                    .find(|candidate| self.source_position(*candidate) == Some(source_position))
            })
    }
}

pub(super) fn source_position_for_view(
    map: &SourceIndexMap,
    view: Option<&gio::ListModel>,
    position: u32,
) -> Option<usize> {
    let Some(view) = view else {
        return Some(position as usize);
    };
    map.of_view_position(view, position)
}

pub(super) fn view_position_for_source(
    source: &gtk::StringList,
    filtered: Option<&gio::ListModel>,
    position: usize,
) -> Option<u32> {
    let Some(filtered) = filtered else {
        return Some(position as u32);
    };
    let item = source.item(position as u32)?;
    if let Some(grouped) = filtered.downcast_ref::<GroupedModel>() {
        return grouped.position_of(&item);
    }
    let guessed = position as u32;
    // Unfiltered views keep source order.
    if filtered.item(guessed).is_some_and(|value| value == item) {
        return Some(guessed);
    }
    let shifted = guessed.saturating_add(1);
    if filtered.item(shifted).is_some_and(|value| value == item) {
        return Some(shifted);
    }
    (0..filtered.n_items())
        .find(|candidate| filtered.item(*candidate).is_some_and(|value| value == item))
}
