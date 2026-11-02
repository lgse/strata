// SPDX-License-Identifier: MIT

use super::presentation;
use crate::adapters::gio_file_for_location;
use crate::model::{FileEntry, Location};
use crate::ui::browser::clipboard::locations_equal;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone)]
pub(super) struct OpenWithSelection {
    pub(super) locations: Vec<Location>,
    pub(super) files: Vec<gio::File>,
    pub(super) content_types: Vec<String>,
    pub(super) choices: crate::ui::open_with::Choices,
    pub(super) default: Option<gio::AppInfo>,
}

impl OpenWithSelection {
    pub(super) fn entries_match_target(&self, entries: &[FileEntry]) -> bool {
        self.locations.len() == entries.len()
            && self
                .locations
                .iter()
                .zip(entries)
                .all(|(location, entry)| locations_equal(location, &entry.location))
    }
}

/// The resolved applications for the menu's current target and the lookup that
/// may still be filling them in.
#[derive(Clone, Default)]
pub(super) struct OpenWithLookup {
    pub(super) selection: Rc<RefCell<Option<OpenWithSelection>>>,
    pub(super) generation: Rc<Cell<u64>>,
}

impl OpenWithLookup {
    /// Forgets the previous answer and returns the generation a new lookup reports under.
    pub(super) fn restart(&self) -> u64 {
        self.selection.replace(None);
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        generation
    }

    pub(super) fn invalidate(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
    }
}

/// The rows an application lookup enables: Open With… for one or several items,
/// and Open for several.
#[derive(Clone)]
pub(super) struct OpenWithButtons {
    pub(super) single: gtk::Button,
    pub(super) multiple: gtk::Button,
    pub(super) open: gtk::Button,
}

/// `explanation` is an untranslated source message.
pub(super) fn set_open_with_explanation(button: &gtk::Button, explanation: Option<&str>) {
    let explanation = explanation.map(crate::i18n::tr).unwrap_or_default();
    if let Some(option) = button.downcast_ref::<presentation::MenuOption>() {
        option.set_menu_description(explanation.as_str());
    }
    crate::ui::accessibility::set_description(button, Some(&explanation));
}

pub(super) fn prepare_folder_open_with(
    button: &gtk::Button,
    history: &Rc<RefCell<Option<crate::ui::recent_apps::State>>>,
    generation: &Rc<Cell<u64>>,
) {
    button.set_sensitive(false);
    set_open_with_explanation(button, Some("Looking for compatible applications…"));
    history.replace(None);
    let expected = generation.get().wrapping_add(1);
    generation.set(expected);
    let (button, history, generation) = (button.clone(), history.clone(), generation.clone());
    glib::MainContext::default().spawn_local(async move {
        let answer = crate::ui::recent_apps::read().await.unwrap_or_default();
        if generation.get() == expected {
            history.replace(Some(answer));
            button.set_sensitive(true);
            set_open_with_explanation(&button, None);
        }
    });
}

pub(super) fn prepare_open_with(
    entries: Vec<FileEntry>,
    buttons: &OpenWithButtons,
    lookup: &OpenWithLookup,
    expected_generation: u64,
) {
    if entries.is_empty() {
        return;
    }
    let locations = entries
        .iter()
        .map(|entry| entry.location.clone())
        .collect::<Vec<_>>();
    let files = entries
        .iter()
        .map(|entry| gio_file_for_location(&entry.location))
        .collect::<Vec<_>>();
    let buttons = buttons.clone();
    let lookup = lookup.clone();
    glib::MainContext::default().spawn_local(async move {
        let resolved = crate::ui::open_with::resolve(&files, || {
            lookup.generation.get() == expected_generation
        })
        .await;
        let applications = match resolved {
            None => return,
            Some(Ok(applications)) => applications,
            Some(Err(reason)) => {
                for button in [&buttons.single, &buttons.multiple] {
                    button.set_sensitive(false);
                    set_open_with_explanation(button, Some(reason));
                }
                return;
            }
        };
        let explanation = applications.unavailable_reason();
        buttons
            .open
            .set_visible(applications.default.is_some() || crate::ui::open_with::uses_portal());
        lookup.selection.replace(Some(OpenWithSelection {
            locations,
            files,
            content_types: applications.content_types,
            choices: applications.choices,
            default: applications.default,
        }));
        for button in [&buttons.single, &buttons.multiple] {
            button.set_sensitive(explanation.is_none());
            set_open_with_explanation(button, explanation);
        }
    });
}
