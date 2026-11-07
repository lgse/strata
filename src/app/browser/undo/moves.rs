// SPDX-License-Identifier: MIT

use std::collections::{HashMap, HashSet};

use crate::{
    model::Location,
    services::{ItemIdentity, MoveRecord, RenamedItem, UndoMoveItem},
};

pub(in crate::app::browser) fn undo_move_parents(items: &[UndoMoveItem]) -> HashSet<Location> {
    items
        .iter()
        .flat_map(|item| [&item.record.current, &item.record.original])
        .filter_map(Location::parent)
        .collect()
}

pub(in crate::app::browser) fn moved_target(
    source: &Location,
    destination: &Location,
    moved_to: &HashMap<Location, Location>,
) -> Option<Location> {
    moved_to
        .get(source)
        .cloned()
        .or_else(|| source.transfer_target(destination))
}

/// Items that never left their own directory carry no undo.
pub(in crate::app::browser) fn move_records(
    sources: &[Location],
    destination: &Location,
    moved_to: &HashMap<Location, Location>,
    renamed: &[RenamedItem],
) -> Vec<MoveRecord> {
    let mut records = sources
        .iter()
        .filter_map(|source| {
            let current = moved_target(source, destination, moved_to)?;
            (&current != source).then(|| MoveRecord {
                original: source.clone(),
                current,
                renamed: Vec::new(),
                replaced: None,
            })
        })
        .collect::<Vec<_>>();
    let moved = records
        .iter()
        .enumerate()
        .map(|(index, record)| (record.current.clone(), index))
        .collect::<HashMap<_, _>>();
    for item in renamed {
        let mut folder = item.saved_as.parent();
        while let Some(location) = folder {
            if let Some(&index) = moved.get(&location) {
                records[index].renamed.push(item.clone());
                break;
            }
            folder = location.parent();
        }
    }
    records
}

pub(in crate::app::browser) fn record_replaced_originals(
    records: &mut [MoveRecord],
    trashed: &HashMap<Location, ItemIdentity>,
    forward: bool,
) {
    for record in records {
        let location = if forward {
            &record.current
        } else {
            &record.original
        };
        record.replaced = trashed.get(location).copied();
    }
}
