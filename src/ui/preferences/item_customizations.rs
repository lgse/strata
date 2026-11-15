// SPDX-License-Identifier: MIT

//! Folder colors and custom icons are keyed by absolute path, so the file
//! operations Strata performs carry, drop or bring back those keys.

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
};

use gtk::glib;
use strata_desktop::settings::item_customization::{TrashedCustomizations, key_path};

use crate::services::ItemIdentity;

use super::{PreferenceManager, trashed::TrashedValues};

#[derive(Default)]
pub(super) struct ItemCustomizationState {
    trashed: RefCell<TrashedValues<TrashedCustomizations>>,
    save_scheduled: Cell<bool>,
}

impl PreferenceManager {
    /// Carries the customization of the item at `from`, and of everything under
    /// it, to `to`, or drops them when `to` is `None` because the item left
    /// local storage. Stale keys at and under a non-merged destination are
    /// dropped first; a merged destination keeps its own key.
    pub fn relocate_item_customizations(
        self: &Rc<Self>,
        from: &Path,
        to: Option<&Path>,
        merged_into_existing: bool,
    ) {
        let changed = self.preferences.borrow_mut().relocate_item_customizations(
            from,
            to,
            merged_into_existing,
        );
        if changed {
            match to {
                Some(to) => self.finish_item_customization_change(&[key_path(from), key_path(to)]),
                None => self.finish_item_customization_change(&[key_path(from)]),
            }
        }
    }

    /// Drops the customizations at and under `root`. A `trash_identity` keeps
    /// them for [`Self::restore_item_customizations`] until the session ends.
    pub fn forget_item_customizations(
        self: &Rc<Self>,
        root: &Path,
        trash_identity: Option<ItemIdentity>,
    ) {
        let changed = {
            let mut preferences = self.preferences.borrow_mut();
            let mut trashed = self.item_customizations.trashed.borrow_mut();
            preferences.forget_item_customizations(root, trash_identity, &mut trashed)
        };
        if changed {
            self.finish_item_customization_change(&[key_path(root)]);
        }
    }

    /// Re-applies what [`Self::forget_item_customizations`] kept for the very
    /// item now back at `root`; another item restored there gets nothing.
    pub fn restore_item_customizations(self: &Rc<Self>, root: &Path, identity: ItemIdentity) {
        let changed = {
            let mut preferences = self.preferences.borrow_mut();
            let mut trashed = self.item_customizations.trashed.borrow_mut();
            preferences.restore_item_customizations(root, identity, &mut trashed)
        };
        if changed {
            self.finish_item_customization_change(&[key_path(root)]);
        }
    }

    /// Operations report items one by one, so saves coalesce on idle.
    fn finish_item_customization_change(self: &Rc<Self>, roots: &[PathBuf]) {
        super::super::thumbnail::refresh_customized_icons_within(roots);
        if self.item_customizations.save_scheduled.replace(true) {
            return;
        }
        let manager = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(manager) = manager.upgrade() {
                manager.item_customizations.save_scheduled.set(false);
                manager.save_preferences();
            }
        });
    }
}
