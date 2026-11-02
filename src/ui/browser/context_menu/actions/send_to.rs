// SPDX-License-Identifier: MIT

use gtk::{gio, prelude::*};
use std::{collections::HashMap, path::PathBuf, rc::Rc};

use crate::assets::icons;
use crate::model::Location;
use crate::ui::browser::context_menu::commands::MenuDispatch;
use crate::ui::browser::{SendToMenuHandlers, ViewState};

/// The removable destinations Send to… offers and what choosing one does.
pub(super) struct SendToChoices {
    pub(super) destinations: Vec<crate::ui::RemovableDestination>,
    pub(super) recent_destinations: HashMap<String, Vec<PathBuf>>,
    pub(super) handlers: SendToMenuHandlers,
}

impl SendToChoices {
    pub(super) fn current(state: &Rc<ViewState>) -> Self {
        let weak_state = Rc::downgrade(state);
        let activate_root: Rc<dyn Fn(String, Vec<Location>)> = Rc::new(move |id, sources| {
            if let Some(state) = weak_state.upgrade() {
                state.send_to_removable_device(id, sources);
            }
        });
        let weak_state = Rc::downgrade(state);
        let choose_folder: Rc<dyn Fn(String, Vec<Location>)> = Rc::new(move |id, sources| {
            if let Some(state) = weak_state.upgrade() {
                state.show_send_to_folder_dialog(id, sources);
            }
        });
        let weak_state = Rc::downgrade(state);
        let activate_recent: Rc<dyn Fn(String, PathBuf, Vec<Location>)> =
            Rc::new(move |id, relative, sources| {
                if let Some(state) = weak_state.upgrade() {
                    state.send_to_recent_destination(id, relative, sources);
                }
            });
        let handlers = SendToMenuHandlers {
            activate_root,
            activate_recent,
            choose_folder,
        };
        let destinations = crate::ui::removable_destinations();
        let recent_destinations = destinations
            .iter()
            .map(|destination| {
                (
                    destination.id.clone(),
                    crate::ui::send_to_recent_folders(destination),
                )
            })
            .collect();
        Self {
            destinations,
            recent_destinations,
            handlers,
        }
    }
}

/// Where Send to… registers its actions and how it defers them past the menu closing.
pub(super) struct SendToActions<'a> {
    pub(super) actions: &'a gio::SimpleActionGroup,
    pub(super) dispatch: &'a MenuDispatch,
}

impl SendToActions<'_> {
    /// Registers `name` to run `activate` with the selection once the menu has closed.
    fn register(&self, name: &str, activate: Rc<dyn Fn()>) {
        let dispatch = self.dispatch.clone();
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate(move |_, _| {
            let activate = activate.clone();
            dispatch.defer(move || activate());
        });
        self.actions.add_action(&action);
    }

    fn root_section(
        &self,
        (index, destination): (usize, &crate::ui::RemovableDestination),
        choices: &SendToChoices,
        sources: &[Location],
    ) -> gio::Menu {
        let name = format!("send-to-{index}");
        let (id, selected) = (destination.id.clone(), sources.to_vec());
        let activate_root = choices.handlers.activate_root.clone();
        self.register(
            &name,
            Rc::new(move || activate_root(id.clone(), selected.clone())),
        );
        let root = gio::Menu::new();
        root.append(Some("Drive root"), Some(&format!("custom.{name}")));
        let recent_paths = choices
            .recent_destinations
            .get(&destination.id)
            .into_iter()
            .flatten();
        for (recent_index, relative) in recent_paths.enumerate() {
            let recent_name = format!("send-to-recent-{index}-{recent_index}");
            let (id, selected) = (destination.id.clone(), sources.to_vec());
            let relative_path = relative.clone();
            let activate_recent = choices.handlers.activate_recent.clone();
            self.register(
                &recent_name,
                Rc::new(move || {
                    activate_recent(id.clone(), relative_path.clone(), selected.clone());
                }),
            );
            root.append(
                Some(&format!(
                    "{} (recent)",
                    relative.to_string_lossy().replace('_', "__")
                )),
                Some(&format!("custom.{recent_name}")),
            );
        }
        root
    }

    fn choose_section(
        &self,
        (index, destination): (usize, &crate::ui::RemovableDestination),
        choices: &SendToChoices,
        sources: &[Location],
    ) -> gio::Menu {
        let folder_name = format!("choose-folder-{index}");
        let (id, selected) = (destination.id.clone(), sources.to_vec());
        let choose_folder = choices.handlers.choose_folder.clone();
        self.register(
            &folder_name,
            Rc::new(move || choose_folder(id.clone(), selected.clone())),
        );
        let choose = gio::Menu::new();
        choose.append(
            Some(&crate::i18n::tr("Choose folder…")),
            Some(&format!("custom.{folder_name}")),
        );
        choose
    }

    pub(super) fn append_devices(
        &self,
        devices: &gio::Menu,
        choices: &SendToChoices,
        sources: &[Location],
    ) {
        if sources.is_empty() {
            return;
        }
        for (index, destination) in choices.destinations.iter().enumerate() {
            let root = self.root_section((index, destination), choices, sources);
            let choose = self.choose_section((index, destination), choices, sources);
            let device = gio::Menu::new();
            device.append_section(None, &root);
            device.append_section(None, &choose);
            let item =
                gio::MenuItem::new_submenu(Some(&destination.name.replace('_', "__")), &device);
            item.set_icon(&gio::ThemedIcon::new(icons::HARD_DRIVE));
            item.set_attribute_value("x-strata-send-to-device", Some(&true.to_variant()));
            item.set_attribute_value("x-strata-description", Some(&destination.name.to_variant()));
            devices.append_item(&item);
        }
    }
}
