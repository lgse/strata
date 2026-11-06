// SPDX-License-Identifier: MIT

//! File-type grouping of a pane's rows: the sorter that clusters them and the
//! headings a grouped list shows.

use std::{cell::Cell, rc::Rc};

use gtk::{glib, prelude::*};

use crate::{
    app::{Browser, compare_type_groups},
    model::ViewPreferences,
    ui::browser::model_type_group,
};

fn model_value(item: &glib::Object) -> String {
    item.downcast_ref::<gtk::StringObject>()
        .map(|value| value.string().to_string())
        .unwrap_or_default()
}

fn model_string(item: &glib::Object) -> Option<glib::GString> {
    item.downcast_ref::<gtk::StringObject>()
        .map(|value| value.string())
}

/// Groups a pane's rows by file type. GTK's stable sort preserves the pane's
/// filename ordering within each type.
#[derive(Clone)]
pub(super) struct TypeGroupSorter {
    pub(super) sorter: gtk::CustomSorter,
    /// The preferences the last comparison read, so a re-sort only follows a
    /// preference change GTK has not already sorted spliced rows under.
    compared_with: Rc<Cell<Option<ViewPreferences>>>,
}

impl TypeGroupSorter {
    pub(super) fn new(preferences: impl Fn() -> Option<ViewPreferences> + 'static) -> Self {
        let compared_with = Rc::new(Cell::new(None));
        let seen = compared_with.clone();
        let sorter = gtk::CustomSorter::new(move |left, right| {
            let preferences = preferences();
            seen.set(preferences);
            let (left, right) = (model_string(left), model_string(right));
            compare_type_groups(
                model_type_group(left.as_deref().unwrap_or("")),
                model_type_group(right.as_deref().unwrap_or("")),
                preferences.unwrap_or_default(),
            )
            .into()
        });
        Self {
            sorter,
            compared_with,
        }
    }

    /// Re-sorts after a preference change; a full re-sort is the one expensive
    /// operation on a large pane, so rows spliced under the current preferences
    /// never trigger one.
    pub(super) fn refresh(&self, preferences: Option<ViewPreferences>) {
        if self.compared_with.get() == preferences {
            return;
        }
        self.compared_with.set(preferences);
        self.sorter.changed(gtk::SorterChange::Different);
    }
}

pub(super) fn pane_type_group_sorter(browser: &Rc<Browser>, depth: usize) -> TypeGroupSorter {
    let browser = Rc::downgrade(browser);
    TypeGroupSorter::new(move || {
        browser
            .upgrade()
            .and_then(|browser| browser.column_preferences(depth))
    })
}

fn type_group_heading(label: &str) -> gtk::Label {
    let heading = gtk::Label::new(Some(&crate::i18n::tr(label)));
    heading.add_css_class("type-group-heading");
    heading.set_xalign(0.0);
    heading
}

/// Section headings for a grouped list view.
pub(super) fn type_group_header_factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, header| {
        let Some(header) = header.downcast_ref::<gtk::ListHeader>() else {
            return;
        };
        header.set_child(Some(&type_group_heading("")));
    });
    factory.connect_bind(|_, header| {
        let Some(header) = header.downcast_ref::<gtk::ListHeader>() else {
            return;
        };
        let Some(heading) = header.child().and_downcast::<gtk::Label>() else {
            return;
        };
        let value = header
            .item()
            .map(|item| model_value(&item))
            .unwrap_or_default();
        let group = model_type_group(&value);
        heading.set_label(&crate::i18n::tr(group));
        heading.set_visible(!group.is_empty());
    });
    factory
}
