// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::ui::browser_modes::{
    ActiveModeRename, install_mode_rename_handlers, view_position_for_source,
};
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
                is_hidden: false,
                mode: MetadataValue::Unknown,
            };
            let active = Rc::new(RefCell::new(Some(ActiveModeRename {
                entry,
                field: field.clone(),
                label: gtk::Label::new(Some("folder")).upcast(),
                viewport_tick: None,
            })));
            let weak = Rc::downgrade(&active);
            install_mode_rename_handlers(
                &field,
                active.clone(),
                std::rc::Weak::new(),
                std::rc::Weak::new(),
            );
            drop(active);
            assert!(weak.upgrade().is_none());
            field.emit_activate();
        },
    );
}
