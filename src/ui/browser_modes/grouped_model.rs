// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
};

use gio::subclass::prelude::ListModelImpl;
use gtk::{gio, glib, prelude::*, subclass::prelude::*};

#[derive(Default)]
struct Group {
    items: Vec<glib::Object>,
}

#[derive(Default)]
struct Rows {
    source: Vec<glib::Object>,
    groups: Vec<Group>,
    grouped: bool,
    count: u32,
}

impl Rows {
    fn item(&self, position: u32) -> Option<glib::Object> {
        if !self.grouped {
            return self.source.get(position as usize).cloned();
        }
        let mut remaining = position as usize;
        for group in &self.groups {
            if remaining < group.items.len() {
                return group.items.get(remaining).cloned();
            }
            remaining -= group.items.len();
        }
        None
    }

    fn start(&self, group: usize) -> u32 {
        self.groups[..group]
            .iter()
            .map(|group| group.items.len() as u32)
            .sum()
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct GroupedModel {
        pub(super) source: RefCell<Option<(gio::ListModel, glib::SignalHandlerId)>>,
        pub(super) sorter: RefCell<Option<(gtk::CustomSorter, glib::SignalHandlerId)>>,
        pub(super) rows: RefCell<Rows>,
        pub(super) revision: Cell<u64>,
        pub(super) changing: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GroupedModel {
        const NAME: &'static str = "StrataTypeGroupedModel";
        type Type = super::GroupedModel;
        type Interfaces = (gio::ListModel, gtk::SectionModel);
    }

    impl ObjectImpl for GroupedModel {
        fn dispose(&self) {
            if let Some((source, handler)) = self.source.take() {
                source.disconnect(handler);
            }
            if let Some((sorter, handler)) = self.sorter.take() {
                sorter.disconnect(handler);
            }
        }
    }

    impl ListModelImpl for GroupedModel {
        fn item_type(&self) -> glib::Type {
            gtk::StringObject::static_type()
        }

        fn n_items(&self) -> u32 {
            self.rows.borrow().count
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            self.rows.borrow().item(position)
        }
    }

    impl SectionModelImpl for GroupedModel {
        fn section(&self, position: u32) -> (u32, u32) {
            let rows = self.rows.borrow();
            if position >= rows.count {
                return (rows.count, u32::MAX);
            }
            if !rows.grouped {
                return (0, rows.count);
            }
            let mut start = 0;
            for group in &rows.groups {
                let end = start + group.items.len() as u32;
                if position < end {
                    return (start, end);
                }
                start = end;
            }
            (rows.count, u32::MAX)
        }
    }
}

glib::wrapper! {
    pub struct GroupedModel(ObjectSubclass<imp::GroupedModel>)
        @implements gio::ListModel, gtk::SectionModel;
}

impl GroupedModel {
    pub(super) fn new(
        source: &impl IsA<gio::ListModel>,
        sorter: Option<&gtk::CustomSorter>,
    ) -> Self {
        let model: Self = glib::Object::new();
        let weak = model.downgrade();
        let handler = source.connect_items_changed(move |source, position, removed, added| {
            if let Some(model) = weak.upgrade() {
                model.source_changed(source.as_ref(), position, removed, added);
            }
        });
        model
            .imp()
            .source
            .replace(Some((source.as_ref().clone(), handler)));
        model.set_sorter(sorter);
        model.reload();
        model
    }

    pub(super) fn set_sorter(&self, sorter: Option<&gtk::CustomSorter>) {
        if let Some((previous, handler)) = self.imp().sorter.take() {
            previous.disconnect(handler);
        }
        if let Some(sorter) = sorter {
            let weak = self.downgrade();
            let handler = sorter.connect_changed(move |_, _| {
                if let Some(model) = weak.upgrade() {
                    model.regroup();
                }
            });
            self.imp().sorter.replace(Some((sorter.clone(), handler)));
        }
        self.regroup();
    }

    pub(super) fn position_of(&self, item: &glib::Object) -> Option<u32> {
        let rows = self.imp().rows.borrow();
        if !rows.grouped {
            return rows
                .source
                .iter()
                .position(|candidate| candidate == item)
                .map(|position| position as u32);
        }
        let mut start = 0;
        for group in &rows.groups {
            if let Some(offset) = group.items.iter().position(|candidate| candidate == item) {
                return Some(start + offset as u32);
            }
            start += group.items.len() as u32;
        }
        None
    }

    fn sorter(&self) -> Option<gtk::CustomSorter> {
        self.imp()
            .sorter
            .borrow()
            .as_ref()
            .map(|(sorter, _)| sorter.clone())
    }

    fn reload(&self) {
        let source = self
            .imp()
            .source
            .borrow()
            .as_ref()
            .map(|(source, _)| source.clone());
        let items = source
            .map(|source| {
                let indexed = indexed_source(&source);
                (0..source.n_items())
                    .filter_map(|position| indexed.item(position))
                    .collect()
            })
            .unwrap_or_default();
        let old = self.current_items();
        self.imp().rows.borrow_mut().source = items;
        self.regroup_from(old);
    }

    fn current_items(&self) -> Vec<glib::Object> {
        let rows = self.imp().rows.borrow();
        (0..rows.count)
            .filter_map(|position| rows.item(position))
            .collect()
    }

    fn regroup(&self) {
        self.regroup_from(self.current_items());
    }

    fn regroup_from(&self, old: Vec<glib::Object>) {
        self.imp()
            .revision
            .set(self.imp().revision.get().wrapping_add(1));
        let sorter = self.sorter();
        let source = self.imp().rows.borrow().source.clone();
        let groups = sorter
            .as_ref()
            .map(|sorter| partition(&source, sorter))
            .unwrap_or_default();
        {
            let mut rows = self.imp().rows.borrow_mut();
            rows.grouped = sorter.is_some();
            rows.groups = groups;
            rows.count = source.len() as u32;
        }
        let new: Vec<_> = {
            let rows = self.imp().rows.borrow();
            (0..rows.count)
                .filter_map(|position| rows.item(position))
                .collect()
        };
        let prefix = old
            .iter()
            .zip(&new)
            .take_while(|(left, right)| left == right)
            .count();
        let suffix = old[prefix..]
            .iter()
            .rev()
            .zip(new[prefix..].iter().rev())
            .take_while(|(left, right)| left == right)
            .count();
        let removed = old.len() - prefix - suffix;
        let added = new.len() - prefix - suffix;
        if removed > 0 || added > 0 {
            self.items_changed(prefix as u32, removed as u32, added as u32);
        }
        if self.n_items() > 0 {
            self.sections_changed(0, self.n_items());
        }
    }

    fn source_changed(&self, source: &gio::ListModel, position: u32, removed: u32, added: u32) {
        let revision = self.imp().revision.get().wrapping_add(1);
        self.imp().revision.set(revision);
        if self.imp().changing.replace(true) {
            self.reload();
            return;
        }
        self.apply_source_change(source, position, removed, added, revision);
        self.imp().changing.set(false);
    }

    fn apply_source_change(
        &self,
        source: &gio::ListModel,
        position: u32,
        removed: u32,
        added: u32,
        revision: u64,
    ) {
        let old_len = self.imp().rows.borrow().source.len() as u32;
        if position.saturating_add(removed) > old_len
            || old_len.saturating_sub(removed).saturating_add(added) != source.n_items()
        {
            self.reload();
            return;
        }
        let indexed = indexed_source(source);
        let arrivals: Vec<_> = (position..position.saturating_add(added))
            .filter_map(|index| indexed.item(index))
            .collect();
        let departing: Vec<_> = self
            .imp()
            .rows
            .borrow_mut()
            .source
            .splice(
                position as usize..(position + removed) as usize,
                arrivals.iter().cloned(),
            )
            .collect();
        let Some(sorter) = self.sorter() else {
            self.imp().rows.borrow_mut().count = source.n_items();
            self.items_changed(position, removed, added);
            return;
        };
        let departing: HashSet<_> = departing.into_iter().collect();
        let group_count = if departing.is_empty() {
            0
        } else {
            self.imp().rows.borrow().groups.len()
        };
        for index in (0..group_count).rev() {
            if self.imp().revision.get() != revision {
                return;
            }
            let change = {
                let mut rows = self.imp().rows.borrow_mut();
                let start = rows.start(index);
                let first = rows.groups[index]
                    .items
                    .iter()
                    .position(|item| departing.contains(item));
                if let Some(first) = first {
                    let group = &mut rows.groups[index];
                    let count = group.items[first..]
                        .iter()
                        .take_while(|item| departing.contains(*item))
                        .count();
                    group.items.drain(first..first + count);
                    if group.items.is_empty() {
                        rows.groups.remove(index);
                    }
                    rows.count -= count as u32;
                    Some((start + first as u32, count as u32))
                } else {
                    None
                }
            };
            if let Some((position, count)) = change {
                self.items_changed(position, count, 0);
            }
        }
        let append = removed == 0 && position == old_len;
        let ranks: HashMap<_, _> = if append {
            HashMap::new()
        } else {
            self.imp()
                .rows
                .borrow()
                .source
                .iter()
                .cloned()
                .enumerate()
                .map(|(index, item)| (item, index as u32))
                .collect()
        };
        for pending in partition(&arrivals, &sorter) {
            if self.imp().revision.get() != revision {
                return;
            }
            let representative = &pending.items[0];
            let (position, count) = {
                let mut rows = self.imp().rows.borrow_mut();
                let group = rows.groups.binary_search_by(|group| {
                    sorter.compare(&group.items[0], representative).into()
                });
                let count = pending.items.len() as u32;
                let position = match group {
                    Ok(index) => {
                        let start = rows.start(index);
                        let items = &mut rows.groups[index].items;
                        let offset = if append {
                            items.len()
                        } else {
                            items.partition_point(|item| {
                                ranks.get(item).is_some_and(|rank| *rank < position)
                            })
                        };
                        items.splice(offset..offset, pending.items);
                        start + offset as u32
                    }
                    Err(index) => {
                        let start = rows.start(index);
                        rows.groups.insert(index, pending);
                        start
                    }
                };
                rows.count += count;
                (position, count)
            };
            self.items_changed(position, 0, count);
        }
        if self.imp().revision.get() == revision && self.n_items() > 0 {
            self.sections_changed(0, self.n_items());
        }
    }
}

fn indexed_source(source: &gio::ListModel) -> gio::ListModel {
    // With no excluded rows the filter preserves every source position. Avoid
    // GTK's bitmap-rank lookup for each item in a large unfiltered listing.
    if let Some(filtered) = source.downcast_ref::<gtk::FilterListModel>()
        && let Some(model) = filtered.model()
        && model.n_items() == source.n_items()
    {
        return model;
    }
    source.clone()
}

fn partition(items: &[glib::Object], sorter: &gtk::CustomSorter) -> Vec<Group> {
    let mut by_label = HashMap::<String, usize>::new();
    let mut groups = Vec::<Group>::new();
    for item in items {
        let Some(value) = item
            .downcast_ref::<gtk::StringObject>()
            .map(|item| item.string())
        else {
            continue;
        };
        let label = super::super::browser::model_type_group(&value);
        let index = if let Some(index) = by_label.get(label) {
            *index
        } else {
            let index = groups
                .iter()
                .position(|group| sorter.compare(&group.items[0], item) == gtk::Ordering::Equal)
                .unwrap_or_else(|| {
                    groups.push(Group::default());
                    groups.len() - 1
                });
            by_label.insert(label.to_owned(), index);
            index
        };
        groups[index].items.push(item.clone());
    }
    groups.sort_by(|left, right| sorter.compare(&left.items[0], &right.items[0]).into());
    groups
}

#[cfg(test)]
mod tests;
