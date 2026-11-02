// SPDX-License-Identifier: MIT

use super::bitset_positions;
use crate::services::SearchItem;
use crate::ui::inline_search::{RenamedResult, Settled};
use gtk::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    path::PathBuf,
    rc::Rc,
};

/// The search-hit list that replaces a column's rows, and the rename it follows.
#[derive(Clone)]
pub(super) struct ResultLists {
    pub(super) model: gtk::StringList,
    pub(super) results: Rc<RefCell<Vec<SearchItem>>>,
    pub(super) selection: gtk::MultiSelection,
    pub(super) syncing: Rc<Cell<bool>>,
    pub(super) renamed: RenamedResult,
}

pub(super) struct ResultsUpdate {
    /// The update changed which results are selected.
    pub(super) selection_changed: bool,
    /// The position the update moved a selected, renamed result to.
    pub(super) renamed: Option<u32>,
}

impl ResultLists {
    /// Shows `items`, keeping the selection by path unless `keep_selection` is off.
    pub(super) fn update(&self, items: Vec<SearchItem>, keep_selection: bool) -> ResultsUpdate {
        let settled = self.renamed.settle(&items);
        let previous = self.selected_paths();
        let selected_paths: HashSet<_> = if keep_selection {
            previous
                .iter()
                .cloned()
                .map(|path| match &settled {
                    Some(settled) => settled.follow(path),
                    None => path,
                })
                .collect()
        } else {
            HashSet::new()
        };
        let was_syncing = self.syncing.replace(true);
        self.splice(&items);
        let selected = gtk::Bitset::new_empty();
        for (position, item) in items.iter().enumerate() {
            if selected_paths.contains(&item.path) {
                selected.add(position as u32);
            }
        }
        self.selection
            .set_selection(&selected, &gtk::Bitset::new_range(0, items.len() as u32));
        self.syncing.set(was_syncing);
        let retained: HashSet<_> = items
            .iter()
            .filter(|item| selected_paths.contains(&item.path))
            .map(|item| item.path.clone())
            .collect();
        ResultsUpdate {
            selection_changed: retained != previous,
            renamed: renamed_position(settled, &selected_paths, &items),
        }
    }

    fn selected_paths(&self) -> HashSet<PathBuf> {
        bitset_positions(&self.selection.selection())
            .into_iter()
            .filter_map(|position| {
                self.results
                    .borrow()
                    .get(position as usize)
                    .map(|item| item.path.clone())
            })
            .collect()
    }

    /// Edits the hits into `items` in place, so unchanged rows keep their widgets.
    fn splice(&self, items: &[SearchItem]) {
        let (model, results) = (&self.model, &self.results);
        for (position, item) in items.iter().enumerate() {
            let existing = results.borrow()[position..].iter().position(|old| {
                old.path == item.path
                    && old.name == item.name
                    && old.is_directory == item.is_directory
            });
            match existing {
                Some(0) => {}
                Some(offset) => {
                    // Row binding reads results synchronously during each model notification.
                    results.borrow_mut().drain(position..position + offset);
                    model.splice(position as u32, offset as u32, &[]);
                }
                None => {
                    results.borrow_mut().insert(position, item.clone());
                    model.splice(position as u32, 0, &[item.name.as_str()]);
                }
            }
        }
        let removed = results.borrow().len() - items.len();
        if removed > 0 {
            results.borrow_mut().truncate(items.len());
            model.splice(items.len() as u32, removed as u32, &[]);
        }
    }
}

/// Where a selected result the update renamed now lists.
fn renamed_position(
    settled: Option<Settled>,
    selected_paths: &HashSet<PathBuf>,
    items: &[SearchItem],
) -> Option<u32> {
    let Some(Settled::Followed(renamed)) = settled else {
        return None;
    };
    Some(renamed)
        .filter(|renamed| selected_paths.contains(&renamed.to))
        .and_then(|renamed| items.iter().position(|item| item.path == renamed.to))
        .and_then(|position| u32::try_from(position).ok())
}
