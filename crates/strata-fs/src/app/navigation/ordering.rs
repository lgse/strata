// SPDX-License-Identifier: MIT

use std::{cmp::Ordering, collections::HashMap, rc::Rc};

use crate::{
    model::{FileEntry, MetadataValue, NameComparison, SortDirection, SortKey, ViewPreferences},
    services::{OTHER_TYPE_NAME, entry_type_description_for_name},
};

use super::EntryInsertion;

mod parallel;

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
    let incoming_entries = sort_entries(incoming, preferences);
    if existing.is_empty() {
        let insertion = EntryInsertion {
            position: 0,
            entries: incoming_entries.clone(),
        };
        return (incoming_entries, vec![insertion]);
    }

    merge_sorted(existing, incoming_entries, preferences)
}

fn merge_sorted(
    existing: Vec<FileEntry>,
    incoming: Vec<FileEntry>,
    preferences: ViewPreferences,
) -> (Vec<FileEntry>, Vec<EntryInsertion>) {
    let precompute_type = preferences.sort_key == SortKey::Type;
    let mut merged = Vec::with_capacity(existing.len() + incoming.len());
    let mut existing_items = existing
        .into_iter()
        .map(|entry| SortItem::new(entry, precompute_type))
        .peekable();
    let mut incoming_items = incoming
        .into_iter()
        .map(|entry| SortItem::new(entry, precompute_type))
        .peekable();
    let mut insertions = Vec::<EntryInsertion>::new();

    while existing_items.peek().is_some() || incoming_items.peek().is_some() {
        if takes_incoming(existing_items.peek(), incoming_items.peek(), preferences) {
            let Some(item) = incoming_items.next() else {
                break;
            };
            push_insertion(&mut insertions, merged.len(), item.entry.clone());
            merged.push(item.entry);
        } else if let Some(item) = existing_items.next() {
            merged.push(item.entry);
        }
    }

    (merged, insertions)
}

fn takes_incoming(
    existing: Option<&SortItem>,
    incoming: Option<&SortItem>,
    preferences: ViewPreferences,
) -> bool {
    match (existing, incoming) {
        (Some(left), Some(right)) => {
            if preferences.sort_key == SortKey::Unsorted {
                compare_sort_items(right, left, preferences) == Ordering::Less
            } else {
                compare_sort_items(right, left, preferences) != Ordering::Greater
            }
        }
        (None, Some(_)) => true,
        _ => false,
    }
}

/// Extends the previous insertion when `entry` lands right after it.
fn push_insertion(insertions: &mut Vec<EntryInsertion>, position: usize, entry: FileEntry) {
    match insertions
        .last_mut()
        .filter(|insertion| insertion.position + insertion.entries.len() == position)
    {
        Some(insertion) => insertion.entries.push(entry),
        None => insertions.push(EntryInsertion {
            position,
            entries: vec![entry],
        }),
    }
}

pub fn sort_entries(entries: Vec<FileEntry>, preferences: ViewPreferences) -> Vec<FileEntry> {
    if preferences.sort_key == SortKey::DeviceOrder || entries.len() <= 1 {
        return entries;
    }
    if preferences.sort_key == SortKey::Unsorted {
        return partition_unsorted(entries, preferences.folders_first);
    }

    let type_ranks = if preferences.sort_key == SortKey::Type {
        precompute_type_ranks(&entries)
    } else {
        Vec::new()
    };
    let folded: Vec<FoldedName> = entries
        .iter()
        .map(|entry| FoldedName::new(&entry.display_name))
        .collect();
    let mut proxies: Vec<SortProxy> = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| SortProxy {
            index,
            is_dir: entry.is_directory(),
            prefix: NamePrefix::new(&entry.display_name, preferences.name_comparison),
        })
        .collect();

    let ctx = SortContext {
        entries: &entries,
        folded: &folded,
        type_ranks: &type_ranks,
        preferences,
    };
    parallel::par_sort_by(&mut proxies, |left, right| ctx.compare(*left, *right));

    let mut slots: Vec<Option<FileEntry>> = entries.into_iter().map(Some).collect();
    proxies
        .into_iter()
        .map(|proxy| slots[proxy.index].take().expect("unique sort index"))
        .collect()
}

fn partition_unsorted(entries: Vec<FileEntry>, folders_first: bool) -> Vec<FileEntry> {
    if !folders_first {
        return entries;
    }
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for entry in entries {
        if entry.is_directory() {
            dirs.push(entry);
        } else {
            files.push(entry);
        }
    }
    dirs.extend(files);
    dirs
}

fn precompute_type_ranks(entries: &[FileEntry]) -> Vec<u32> {
    let mut distinct: Vec<Rc<str>> = Vec::new();
    let mut lookup: HashMap<Rc<str>, usize> = HashMap::new();
    let mut raw_ids: Vec<usize> = Vec::with_capacity(entries.len());
    for entry in entries {
        let desc = entry_type_description_for_name(entry.kind, &entry.display_name);
        let id = match lookup.get(&desc) {
            Some(&id) => id,
            None => {
                let id = distinct.len();
                distinct.push(desc.clone());
                lookup.insert(desc, id);
                id
            }
        };
        raw_ids.push(id);
    }
    let mut order: Vec<usize> = (0..distinct.len()).collect();
    order.sort_by(|&a, &b| compare_type_descriptions(&distinct[a], &distinct[b]));
    let mut rank_of = vec![0_u32; distinct.len()];
    for (rank, raw_id) in order.into_iter().enumerate() {
        rank_of[raw_id] = u32::try_from(rank).unwrap_or(u32::MAX);
    }
    raw_ids.into_iter().map(|id| rank_of[id]).collect()
}

fn compare_type_descriptions(left: &str, right: &str) -> Ordering {
    match (left == OTHER_TYPE_NAME, right == OTHER_TYPE_NAME) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => compare_display_names(left, right),
    }
}

const PREFIX_INVALID: u8 = u8::MAX;

/// Compact 8-byte mapped ASCII prefix for fast-path name comparisons without chasing pointers.
#[derive(Clone, Copy)]
struct NamePrefix {
    prefix: u64,
    safe_len: u8,
}

impl NamePrefix {
    fn new(name: &str, comparison: NameComparison) -> Self {
        if !name.is_ascii() {
            return Self {
                prefix: 0,
                safe_len: PREFIX_INVALID,
            };
        }
        let bytes = name.as_bytes();
        let len = bytes.len().min(8);
        let mut prefix = 0_u64;
        let mut safe_len = 8_u8;
        for (i, &byte) in bytes[..len].iter().enumerate() {
            if byte == 0 {
                return Self {
                    prefix: 0,
                    safe_len: PREFIX_INVALID,
                };
            }
            let natural_digit = comparison.natural && byte.is_ascii_digit();
            if safe_len == 8 && natural_digit {
                safe_len = i as u8;
            }
            let mapped = name_byte(byte, comparison.case_sensitive);
            prefix |= u64::from(mapped) << (56 - 8 * i);
        }
        Self { prefix, safe_len }
    }

    fn is_valid(self) -> bool {
        self.safe_len != PREFIX_INVALID
    }

    fn fast_cmp(self, other: Self) -> Option<Ordering> {
        if !self.is_valid() || !other.is_valid() {
            return None;
        }
        if self.prefix == other.prefix {
            return None;
        }
        let diff_pos = ((self.prefix ^ other.prefix).leading_zeros() / 8) as u8;
        (diff_pos < self.safe_len || diff_pos < other.safe_len)
            .then(|| self.prefix.cmp(&other.prefix))
    }
}

#[derive(Clone, Copy)]
struct SortProxy {
    index: usize,
    is_dir: bool,
    prefix: NamePrefix,
}

struct SortContext<'a> {
    entries: &'a [FileEntry],
    folded: &'a [FoldedName],
    type_ranks: &'a [u32],
    preferences: ViewPreferences,
}

impl SortContext<'_> {
    fn compare(&self, left: SortProxy, right: SortProxy) -> Ordering {
        if self.preferences.folders_first {
            let directory_order = right.is_dir.cmp(&left.is_dir);
            if directory_order != Ordering::Equal {
                return directory_order;
            }
        }
        let left_entry = &self.entries[left.index];
        let right_entry = &self.entries[right.index];
        if let Some(order) = compare_optional_column(left_entry, right_entry, self.preferences) {
            return order
                .then_with(|| self.compare_proxy_names(left, right))
                .then_with(|| left_entry.location.compare(&right_entry.location));
        }
        self.compare_directed(left, right)
            .then_with(|| self.compare_tie_name(left, right))
            .then_with(|| left_entry.location.compare(&right_entry.location))
    }

    fn compare_directed(&self, left: SortProxy, right: SortProxy) -> Ordering {
        let primary = self.compare_primary(left, right);
        match self.preferences.sort_direction {
            SortDirection::Ascending => primary,
            SortDirection::Descending => primary.reverse(),
        }
    }

    fn compare_tie_name(&self, left: SortProxy, right: SortProxy) -> Ordering {
        if self.preferences.sort_key == SortKey::Name {
            Ordering::Equal
        } else {
            self.compare_proxy_names(left, right)
        }
    }

    fn compare_primary(&self, left: SortProxy, right: SortProxy) -> Ordering {
        let left_entry = &self.entries[left.index];
        let right_entry = &self.entries[right.index];
        match self.preferences.sort_key {
            SortKey::DeviceOrder
            | SortKey::Unsorted
            | SortKey::Created
            | SortKey::Duration
            | SortKey::Dimensions => Ordering::Equal,
            SortKey::Recency => compare_metadata(
                &left_entry.recent_unix_seconds,
                &right_entry.recent_unix_seconds,
            ),
            SortKey::Name => self.compare_proxy_names(left, right),
            SortKey::Extension => compare_extensions(
                &left_entry.display_name,
                &right_entry.display_name,
                self.preferences.name_comparison,
            ),
            SortKey::Type => self.type_ranks[left.index].cmp(&self.type_ranks[right.index]),
            SortKey::Size => compare_metadata(&left_entry.size, &right_entry.size),
            SortKey::Modified => compare_metadata(
                &left_entry.modified_unix_seconds,
                &right_entry.modified_unix_seconds,
            ),
        }
    }

    fn compare_proxy_names(&self, left: SortProxy, right: SortProxy) -> Ordering {
        if let Some(order) = left.prefix.fast_cmp(right.prefix) {
            return order;
        }
        compare_names(
            (
                &self.folded[left.index],
                &self.entries[left.index].display_name,
            ),
            (
                &self.folded[right.index],
                &self.entries[right.index].display_name,
            ),
            self.preferences.name_comparison,
        )
    }
}

enum FoldedName {
    Ascii,
    Folded(Box<str>),
}

impl FoldedName {
    fn new(name: &str) -> Self {
        if name.is_ascii() {
            Self::Ascii
        } else {
            Self::Folded(glib::casefold(name).as_str().into())
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
    compare_names(
        (left_folded, left_raw),
        (right_folded, right_raw),
        NameComparison::default(),
    )
}

struct SortItem {
    entry: FileEntry,
    folded_name: FoldedName,
    entry_type: Option<Rc<str>>,
}

impl SortItem {
    fn new(entry: FileEntry, precompute_type: bool) -> Self {
        let folded_name = FoldedName::new(&entry.display_name);
        let entry_type = precompute_type
            .then(|| entry_type_description_for_name(entry.kind, &entry.display_name));
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

    if preferences.sort_key == SortKey::Unsorted {
        return Ordering::Equal;
    }
    if let Some(order) = compare_optional_column(&left.entry, &right.entry, preferences) {
        return order
            .then_with(|| {
                compare_names(
                    (&left.folded_name, &left.entry.display_name),
                    (&right.folded_name, &right.entry.display_name),
                    preferences.name_comparison,
                )
            })
            .then_with(|| left.entry.location.compare(&right.entry.location));
    }

    let ordering = match preferences.sort_key {
        SortKey::DeviceOrder
        | SortKey::Unsorted
        | SortKey::Created
        | SortKey::Duration
        | SortKey::Dimensions => Ordering::Equal,
        SortKey::Recency => compare_metadata(
            &left.entry.recent_unix_seconds,
            &right.entry.recent_unix_seconds,
        ),
        SortKey::Name => compare_names(
            (&left.folded_name, &left.entry.display_name),
            (&right.folded_name, &right.entry.display_name),
            preferences.name_comparison,
        ),
        SortKey::Extension => compare_extensions(
            &left.entry.display_name,
            &right.entry.display_name,
            preferences.name_comparison,
        ),
        SortKey::Type => {
            let left_type = left
                .entry_type
                .as_deref()
                .expect("Type sorting precomputes entry types");
            let right_type = right
                .entry_type
                .as_deref()
                .expect("Type sorting precomputes entry types");
            compare_type_descriptions(left_type, right_type)
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
            compare_names(
                (&left.folded_name, &left.entry.display_name),
                (&right.folded_name, &right.entry.display_name),
                preferences.name_comparison,
            )
        })
        .then_with(|| left.entry.location.compare(&right.entry.location))
}

pub(super) fn compare_entries(
    left: &FileEntry,
    right: &FileEntry,
    preferences: ViewPreferences,
) -> Ordering {
    if preferences.sort_key == SortKey::DeviceOrder {
        return Ordering::Equal;
    }
    if preferences.folders_first {
        let directory_order = right.is_directory().cmp(&left.is_directory());
        if directory_order != Ordering::Equal {
            return directory_order;
        }
    }

    if preferences.sort_key == SortKey::Unsorted {
        return Ordering::Equal;
    }
    if let Some(order) = compare_optional_column(left, right, preferences) {
        return order
            .then_with(|| {
                compare_configured_names(
                    &left.display_name,
                    &right.display_name,
                    preferences.name_comparison,
                )
            })
            .then_with(|| left.location.compare(&right.location));
    }

    let left_folded = FoldedName::new(&left.display_name);
    let right_folded = FoldedName::new(&right.display_name);

    let ordering = match preferences.sort_key {
        SortKey::DeviceOrder
        | SortKey::Unsorted
        | SortKey::Created
        | SortKey::Duration
        | SortKey::Dimensions => Ordering::Equal,
        SortKey::Recency => compare_metadata(&left.recent_unix_seconds, &right.recent_unix_seconds),
        SortKey::Name => compare_names(
            (&left_folded, &left.display_name),
            (&right_folded, &right.display_name),
            preferences.name_comparison,
        ),
        SortKey::Extension => compare_extensions(
            &left.display_name,
            &right.display_name,
            preferences.name_comparison,
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
            compare_names(
                (&left_folded, &left.display_name),
                (&right_folded, &right.display_name),
                preferences.name_comparison,
            )
        })
        .then_with(|| left.location.compare(&right.location))
}

fn compare_entry_types(left: &FileEntry, right: &FileEntry) -> Ordering {
    let left_type = entry_type_description_for_name(left.kind, &left.display_name);
    let right_type = entry_type_description_for_name(right.kind, &right.display_name);
    compare_type_descriptions(&left_type, &right_type)
}

pub fn compare_display_names(left: &str, right: &str) -> Ordering {
    let left_folded = FoldedName::new(left);
    let right_folded = FoldedName::new(right);
    compare_with_folded(&left_folded, left, &right_folded, right)
}

fn natural_compare(left: &[u8], right: &[u8], case_sensitive: bool) -> Ordering {
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
            let lb = name_byte(left[li], case_sensitive);
            let rb = name_byte(right[ri], case_sensitive);
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

fn compare_names(
    left: (&FoldedName, &str),
    right: (&FoldedName, &str),
    comparison: NameComparison,
) -> Ordering {
    let (left_bytes, right_bytes) = if comparison.case_sensitive {
        (left.1.as_bytes(), right.1.as_bytes())
    } else {
        (left.0.as_bytes(left.1), right.0.as_bytes(right.1))
    };
    let ordering = if comparison.natural {
        natural_compare(left_bytes, right_bytes, comparison.case_sensitive)
    } else if comparison.case_sensitive {
        left_bytes.cmp(right_bytes)
    } else {
        left_bytes
            .iter()
            .map(u8::to_ascii_lowercase)
            .cmp(right_bytes.iter().map(u8::to_ascii_lowercase))
    };
    ordering.then_with(|| left.1.cmp(right.1))
}

fn compare_configured_names(left: &str, right: &str, comparison: NameComparison) -> Ordering {
    compare_names(
        (&FoldedName::new(left), left),
        (&FoldedName::new(right), right),
        comparison,
    )
}

fn compare_extensions(left: &str, right: &str, comparison: NameComparison) -> Ordering {
    fn extension(name: &str) -> &str {
        std::path::Path::new(name)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
    }
    compare_configured_names(extension(left), extension(right), comparison)
}

fn compare_optional_column(
    left: &FileEntry,
    right: &FileEntry,
    preferences: ViewPreferences,
) -> Option<Ordering> {
    let direction = preferences.sort_direction;
    match preferences.sort_key {
        SortKey::Created => Some(compare_available(
            known(&left.created_unix_seconds),
            known(&right.created_unix_seconds),
            direction,
        )),
        SortKey::Duration => Some(compare_available(
            known(&left.duration_seconds),
            known(&right.duration_seconds),
            direction,
        )),
        SortKey::Dimensions => Some(compare_available(
            pixel_count(left),
            pixel_count(right),
            direction,
        )),
        _ => None,
    }
}

fn known<T: Copy>(value: &MetadataValue<T>) -> Option<T> {
    match value {
        MetadataValue::Known(value) => Some(*value),
        _ => None,
    }
}

fn pixel_count(entry: &FileEntry) -> Option<u64> {
    known(&entry.image_dimensions).map(|(width, height)| u64::from(width) * u64::from(height))
}

fn compare_available<T: Ord>(
    left: Option<T>,
    right: Option<T>,
    direction: SortDirection,
) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) if direction == SortDirection::Descending => right.cmp(&left),
        (Some(left), Some(right)) => left.cmp(&right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn name_byte(byte: u8, case_sensitive: bool) -> u8 {
    if case_sensitive {
        byte
    } else {
        byte.to_ascii_lowercase()
    }
}
