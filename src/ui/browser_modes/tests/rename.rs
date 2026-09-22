// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::browser_modes::view_position_for_source;
use crate::ui::collection_edit::{self, EditWidgets};
use std::{cell::RefCell, rc::Rc};

#[test]
fn rename_position_lookup_follows_live_view_order() {
    gtk_test(
        "ui::browser_modes::tests::rename::rename_position_lookup_follows_live_view_order",
        || {
            let source = gtk::StringList::new(&["alpha", "beta", "gamma"]);
            let reversed = Rc::new(std::cell::Cell::new(false));
            let order = reversed.clone();
            let sorter = gtk::CustomSorter::new(move |left, right| {
                let left = left
                    .downcast_ref::<gtk::StringObject>()
                    .expect("string item")
                    .string();
                let right = right
                    .downcast_ref::<gtk::StringObject>()
                    .expect("string item")
                    .string();
                if order.get() {
                    right.cmp(&left)
                } else {
                    left.cmp(&right)
                }
                .into()
            });
            let live = gtk::SortListModel::new(Some(source.clone()), Some(sorter.clone()));
            assert_eq!(
                view_position_for_source(&source, Some(live.upcast_ref()), 0),
                Some(0)
            );
            reversed.set(true);
            sorter.changed(gtk::SorterChange::Different);
            assert_eq!(
                view_position_for_source(&source, Some(live.upcast_ref()), 0),
                Some(2)
            );
            assert_eq!(
                view_position_for_source(&source, Some(live.upcast_ref()), 2),
                Some(0)
            );
        },
    );
}

#[test]
fn rename_handlers_do_not_keep_the_active_editor_alive_after_the_view_drops() {
    gtk_test(
        "ui::browser_modes::tests::rename::rename_handlers_do_not_keep_the_active_editor_alive_after_the_view_drops",
        || {
            let field = gtk::Entry::new();
            let entry = FileEntry {
                location: Location::local("/fixture/folder"),
                native_name: "folder".into(),
                display_name: "folder".to_owned(),
                kind: EntryKind::Directory,
                thumbnail_path: None,
                size: MetadataValue::Unknown,
                modified_unix_seconds: MetadataValue::Unknown,
                recent_unix_seconds: MetadataValue::Unknown,
                is_hidden: false,
                mode: MetadataValue::Unknown,
                image_dimensions: MetadataValue::Unknown,
                child_count: MetadataValue::Unknown,
                duration_seconds: MetadataValue::Unknown,
            };
            let active = Rc::new(RefCell::new(None));
            let weak = Rc::downgrade(&active);
            let widgets = EditWidgets::new(&field, &gtk::Label::new(Some("folder")));
            widgets.bind(&entry.location);
            assert!(collection_edit::begin(
                &active,
                entry,
                widgets.into(),
                Rc::new(|_| {})
            ));
            drop(active);
            assert!(weak.upgrade().is_none());
            field.emit_activate();
        },
    );
}
