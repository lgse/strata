// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::ui::browser_modes::{ActiveModeRename, install_mode_rename_handlers};
use std::{cell::RefCell, rc::Rc};

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
            })));
            let weak = Rc::downgrade(&active);
            install_mode_rename_handlers(&field, active.clone(), std::rc::Weak::new());
            drop(active);
            assert!(weak.upgrade().is_none());
            field.emit_activate();
        },
    );
}
