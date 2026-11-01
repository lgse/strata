// SPDX-License-Identifier: MIT

use std::cmp::Ordering;

use crate::model::{FileEntry, MetadataValue, SortDirection, SortKey, ViewPreferences};

use super::EntryInsertion;

pub(super) fn merge_entries(
    mut existing: Vec<FileEntry>,
    incoming: Vec<FileEntry>,
    preferences: ViewPreferences,
) -> (Vec<FileEntry>, Vec<EntryInsertion>) {
    if preferences.sort_key == SortKey::DeviceOrder {
        let insertion = EntryInsertion {
            position: existing.len(),
            entries: incoming.clone(),
        };
        existing.extend(incoming);
        return (existing, vec![insertion]);
    }
    let precompute_type = preferences.sort_key == SortKey::Type;
    let mut incoming_items: Vec<SortItem> = incoming
        .into_iter()
        .map(|entry| SortItem::new(entry, precompute_type))
        .collect();
    incoming_items.sort_unstable_by(|left, right| compare_sort_items(left, right, preferences));
    if existing.is_empty() {
        let incoming_entries: Vec<FileEntry> =
            incoming_items.into_iter().map(|item| item.entry).collect();
        let insertion = EntryInsertion {
            position: 0,
            entries: incoming_entries.clone(),
        };
        return (incoming_entries, vec![insertion]);
    }

    let existing_count = existing.len();
    let incoming_count = incoming_items.len();
    let mut existing_items = existing
        .into_iter()
        .map(|entry| SortItem::new(entry, precompute_type))
        .peekable();
    let mut incoming_items = incoming_items.into_iter().peekable();
    let mut merged = Vec::with_capacity(existing_count + incoming_count);
    let mut insertions = Vec::<EntryInsertion>::new();

    while existing_items.peek().is_some() || incoming_items.peek().is_some() {
        let take_incoming = match (existing_items.peek(), incoming_items.peek()) {
            (Some(left), Some(right)) => {
                compare_sort_items(right, left, preferences) != Ordering::Greater
            }
            (None, Some(_)) => true,
            _ => false,
        };

        if take_incoming {
            let Some(item) = incoming_items.next() else {
                break;
            };
            let entry = item.entry;
            let position = merged.len();
            if let Some(insertion) = insertions
                .last_mut()
                .filter(|insertion| insertion.position + insertion.entries.len() == position)
            {
                insertion.entries.push(entry.clone());
            } else {
                insertions.push(EntryInsertion {
                    position,
                    entries: vec![entry.clone()],
                });
            }
            merged.push(entry);
        } else if let Some(item) = existing_items.next() {
            merged.push(item.entry);
        }
    }

    (merged, insertions)
}

pub(crate) fn sort_entries(
    entries: Vec<FileEntry>,
    preferences: ViewPreferences,
) -> Vec<FileEntry> {
    if preferences.sort_key == SortKey::DeviceOrder || entries.len() <= 1 {
        return entries;
    }
    let precompute_type = preferences.sort_key == SortKey::Type;
    let mut items: Vec<SortItem> = entries
        .into_iter()
        .map(|entry| SortItem::new(entry, precompute_type))
        .collect();

    items.sort_unstable_by(|left, right| compare_sort_items(left, right, preferences));

    items.into_iter().map(|item| item.entry).collect()
}

enum FoldedName {
    Ascii,
    Folded(glib::GString),
}

impl FoldedName {
    fn new(name: &str) -> Self {
        if name.is_ascii() {
            Self::Ascii
        } else {
            Self::Folded(glib::casefold(name))
        }
    }

    fn as_bytes<'a>(&'a self, raw: &'a str) -> &'a [u8] {
        match self {
            Self::Ascii => raw.as_bytes(),
            Self::Folded(folded) => folded.as_bytes(),
        }
    }
}

fn compare_with_folded(
    left_folded: &FoldedName,
    left_raw: &str,
    right_folded: &FoldedName,
    right_raw: &str,
) -> Ordering {
    natural_compare(
        left_folded.as_bytes(left_raw),
        right_folded.as_bytes(right_raw),
    )
    .then_with(|| left_raw.cmp(right_raw))
}

struct SortItem {
    entry: FileEntry,
    folded_name: FoldedName,
    entry_type: Option<crate::services::EntryType>,
}

impl SortItem {
    fn new(entry: FileEntry, precompute_type: bool) -> Self {
        let folded_name = FoldedName::new(&entry.display_name);
        let entry_type = precompute_type.then(|| crate::services::entry_type(&entry));
        Self {
            entry,
            folded_name,
            entry_type,
        }
    }
}

fn compare_sort_items(left: &SortItem, right: &SortItem, preferences: ViewPreferences) -> Ordering {
    if preferences.sort_key == SortKey::DeviceOrder {
        return Ordering::Equal;
    }
    if preferences.folders_first {
        let directory_order = right.entry.is_directory().cmp(&left.entry.is_directory());
        if directory_order != Ordering::Equal {
            return directory_order;
        }
    }

    let ordering = match preferences.sort_key {
        SortKey::DeviceOrder => Ordering::Equal,
        SortKey::Recency => compare_metadata(
            &left.entry.recent_unix_seconds,
            &right.entry.recent_unix_seconds,
        ),
        SortKey::Name => compare_with_folded(
            &left.folded_name,
            &left.entry.display_name,
            &right.folded_name,
            &right.entry.display_name,
        ),
        SortKey::Type => {
            let left_type = left
                .entry_type
                .as_ref()
                .expect("Type sorting precomputes entry types");
            let right_type = right
                .entry_type
                .as_ref()
                .expect("Type sorting precomputes entry types");
            compare_entry_type_values(left_type, right_type)
        }
        SortKey::Size => compare_metadata(&left.entry.size, &right.entry.size),
        SortKey::Modified => compare_metadata(
            &left.entry.modified_unix_seconds,
            &right.entry.modified_unix_seconds,
        ),
    };
    let ordering = match preferences.sort_direction {
        SortDirection::Ascending => ordering,
        SortDirection::Descending => ordering.reverse(),
    };
    ordering
        .then_with(|| {
            compare_with_folded(
                &left.folded_name,
                &left.entry.display_name,
                &right.folded_name,
                &right.entry.display_name,
            )
        })
        .then_with(|| left.entry.location.compare(&right.entry.location))
}

pub(super) fn compare_entries(left: &FileEntry, right: &FileEntry, preferences: ViewPreferences) -> Ordering {
    if preferences.sort_key == SortKey::DeviceOrder {
        return Ordering::Equal;
    }
    if preferences.folders_first {
        let directory_order = right.is_directory().cmp(&left.is_directory());
        if directory_order != Ordering::Equal {
            return directory_order;
        }
    }

    let left_folded = FoldedName::new(&left.display_name);
    let right_folded = FoldedName::new(&right.display_name);

    let ordering = match preferences.sort_key {
        SortKey::DeviceOrder => Ordering::Equal,
        SortKey::Recency => compare_metadata(&left.recent_unix_seconds, &right.recent_unix_seconds),
        SortKey::Name => compare_with_folded(
            &left_folded,
            &left.display_name,
            &right_folded,
            &right.display_name,
        ),
        SortKey::Type => compare_entry_types(left, right),
        SortKey::Size => compare_metadata(&left.size, &right.size),
        SortKey::Modified => {
            compare_metadata(&left.modified_unix_seconds, &right.modified_unix_seconds)
        }
    };
    let ordering = match preferences.sort_direction {
        SortDirection::Ascending => ordering,
        SortDirection::Descending => ordering.reverse(),
    };
    ordering
        .then_with(|| {
            compare_with_folded(
                &left_folded,
                &left.display_name,
                &right_folded,
                &right.display_name,
            )
        })
        .then_with(|| left.location.compare(&right.location))
}

fn compare_entry_types(left: &FileEntry, right: &FileEntry) -> Ordering {
    let left_type = crate::services::entry_type(left);
    let right_type = crate::services::entry_type(right);
    compare_entry_type_values(&left_type, &right_type)
}

fn compare_entry_type_values(
    left_type: &crate::services::EntryType,
    right_type: &crate::services::EntryType,
) -> Ordering {
    use crate::services::EntryType;
    match (left_type, right_type) {
        (EntryType::Other, EntryType::Other) => Ordering::Equal,
        (EntryType::Other, _) => Ordering::Greater,
        (_, EntryType::Other) => Ordering::Less,
        _ => compare_display_names(left_type.description(), right_type.description()),
    }
}

pub(crate) fn compare_display_names(left: &str, right: &str) -> Ordering {
    let left_folded = FoldedName::new(left);
    let right_folded = FoldedName::new(right);
    compare_with_folded(&left_folded, left, &right_folded, right)
}

fn natural_compare(left: &[u8], right: &[u8]) -> Ordering {
    let (mut li, mut ri) = (0, 0);
    while li < left.len() && ri < right.len() {
        if left[li].is_ascii_digit() && right[ri].is_ascii_digit() {
            let (lv, lo) = take_number(left, li);
            let (rv, ro) = take_number(right, ri);
            let cmp = lv
                .len()
                .cmp(&rv.len())
                .then_with(|| lv.cmp(rv))
                .then_with(|| left[li..lo].cmp(&right[ri..ro]));
            if cmp != Ordering::Equal {
                return cmp;
            }
            li = lo;
            ri = ro;
        } else {
            let lb = left[li].to_ascii_lowercase();
            let rb = right[ri].to_ascii_lowercase();
            if lb != rb {
                return lb.cmp(&rb);
            }
            li += 1;
            ri += 1;
        }
    }
    left.len().cmp(&right.len())
}

fn take_number(bytes: &[u8], start: usize) -> (&[u8], usize) {
    let mut end = start;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    let mut significant = start;
    while significant < end && bytes[significant] == b'0' {
        significant += 1;
    }
    (&bytes[significant..end], end)
}

fn compare_metadata<T: Ord>(left: &MetadataValue<T>, right: &MetadataValue<T>) -> Ordering {
    match (left, right) {
        (MetadataValue::Known(left), MetadataValue::Known(right)) => left.cmp(right),
        (MetadataValue::Known(_), _) => Ordering::Less,
        (_, MetadataValue::Known(_)) => Ordering::Greater,
        (MetadataValue::Unknown, MetadataValue::Unavailable) => Ordering::Less,
        (MetadataValue::Unavailable, MetadataValue::Unknown) => Ordering::Greater,
        _ => Ordering::Equal,
    }
}
