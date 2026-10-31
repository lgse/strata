// SPDX-License-Identifier: MIT

//! GTK has no public getter for accessible labels, relations or states, so the naming
//! helpers record what they set for the `--qa` harness.

use std::{cell::RefCell, collections::HashMap};

use gtk::{glib, prelude::*};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mark {
    Pane(&'static str),
    EntryContainer,
    PaneSurface,
    Entry,
    Dialog,
    MenuItem,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Naming {
    pub(crate) label: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) mark: Option<Mark>,
    pub(crate) checked: Option<bool>,
    pub(crate) pressed: Option<bool>,
    pub(crate) selected: Option<bool>,
    pub(crate) busy: Option<bool>,
}

struct Record {
    object: glib::WeakRef<glib::Object>,
    naming: Naming,
    labelled_by: Vec<glib::WeakRef<gtk::Widget>>,
    labelled_by_reset: bool,
}

const PRUNE_EVERY: usize = 1024;

#[derive(Default)]
struct Records {
    by_address: HashMap<usize, Record>,
    inserted: usize,
}

thread_local! {
    static RECORDS: RefCell<Records> = RefCell::default();
}

fn address(object: &glib::Object) -> usize {
    object.as_ptr() as usize
}

pub(crate) fn record(object: &impl ObjectType, update: impl FnOnce(&mut Naming)) {
    with_record(object, |record| update(&mut record.naming));
}

pub(crate) fn record_labelled_by_reset(object: &impl ObjectType) {
    with_record(object, |record| {
        record.labelled_by.clear();
        record.labelled_by_reset = true;
    });
}

pub(crate) fn labelled_by_reset(object: &impl ObjectType) -> bool {
    let Some(object) = object.dynamic_cast_ref::<glib::Object>() else {
        return false;
    };
    RECORDS.with_borrow(|records| {
        records
            .by_address
            .get(&address(object))
            .filter(|record| record.object.upgrade().as_ref() == Some(object))
            .is_some_and(|record| record.labelled_by_reset)
    })
}

pub(crate) fn record_labelled_by(object: &impl ObjectType, labels: &[&gtk::Widget]) {
    with_record(object, |record| {
        record.labelled_by = labels.iter().map(|label| label.downgrade()).collect();
    });
}

fn with_record(object: &impl ObjectType, update: impl FnOnce(&mut Record)) {
    let Some(object) = object.dynamic_cast_ref::<glib::Object>() else {
        return;
    };
    RECORDS.with_borrow_mut(|records| {
        let key = address(object);
        // A freed object's address can be reused; its record must not leak into the new one.
        let stale = records
            .by_address
            .get(&key)
            .is_some_and(|record| record.object.upgrade().as_ref() != Some(object));
        if stale {
            records.by_address.remove(&key);
        }
        let record = records.by_address.entry(key).or_insert_with(|| Record {
            object: object.downgrade(),
            naming: Naming::default(),
            labelled_by: Vec::new(),
            labelled_by_reset: false,
        });
        update(record);
        records.inserted += 1;
        if records.inserted.is_multiple_of(PRUNE_EVERY) {
            records
                .by_address
                .retain(|_, record| record.object.upgrade().is_some());
        }
    });
}

pub(crate) fn naming(object: &impl ObjectType) -> Option<Naming> {
    let object = object.dynamic_cast_ref::<glib::Object>()?;
    RECORDS.with_borrow(|records| {
        records
            .by_address
            .get(&address(object))
            .filter(|record| record.object.upgrade().as_ref() == Some(object))
            .map(|record| record.naming.clone())
    })
}

pub(crate) fn labelled_by(object: &impl ObjectType) -> Vec<gtk::Widget> {
    let Some(object) = object.dynamic_cast_ref::<glib::Object>() else {
        return Vec::new();
    };
    RECORDS.with_borrow(|records| {
        records
            .by_address
            .get(&address(object))
            .filter(|record| record.object.upgrade().as_ref() == Some(object))
            .map(|record| {
                record
                    .labelled_by
                    .iter()
                    .filter_map(glib::WeakRef::upgrade)
                    .collect()
            })
            .unwrap_or_default()
    })
}

pub(crate) fn entries() -> Vec<(gtk::ListItem, Naming)> {
    RECORDS.with_borrow(|records| {
        records
            .by_address
            .values()
            .filter(|record| record.naming.mark == Some(Mark::Entry))
            .filter_map(|record| {
                let item = record.object.upgrade()?.downcast::<gtk::ListItem>().ok()?;
                Some((item, record.naming.clone()))
            })
            .collect()
    })
}
