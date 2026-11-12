// SPDX-License-Identifier: MIT

use super::open_with::prepare_folder_open_with;
use super::options::{context_menu_option, context_menu_toggle_option};
use super::{actions, bind_column_context_owner, focus_context_column, schedule_new_entry};
use crate::adapters::gio_file_for_location;
use crate::assets::icons;
use crate::model::Location;
use crate::ui::browser::ViewState;
use crate::ui::browser::chooser_context;
use crate::ui::browser::customization::show_customize_modal;
use crate::ui::browser::desktop::{can_open_terminal, launch_terminal};
use crate::ui::browser::paths::is_trash_location;
use crate::ui::shortcut_reference::ContextHint;
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

type RecentApps = Rc<RefCell<Option<crate::ui::recent_apps::State>>>;

fn hidden_files_toggle(shown: bool) -> (&'static str, &'static str) {
    if shown {
        (icons::EYE, "Hide Hidden Files")
    } else {
        (icons::EYE_OFF, "Show Hidden Files")
    }
}

struct FolderMenuOptions {
    content: gtk::Box,
    remaining: gtk::Box,
    new_folder: gtk::Button,
    new_file: gtk::Button,
    open_with: gtk::Button,
    open_terminal: gtk::Button,
    paste: gtk::Button,
    select_all: gtk::Button,
    refresh: gtk::Button,
    toggle_hidden: gtk::Button,
    toggle_hidden_icon: gtk::Image,
    toggle_hidden_label: gtk::Label,
    customize: gtk::Button,
    properties: gtk::Button,
    /// Creating, pasting, and opening apply to real folders, not Trash or Recent.
    directory_actions: bool,
}

impl FolderMenuOptions {
    fn build(hidden_files_shown: bool, location: &Location) -> Self {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("folder-context-menu");
        let remaining = gtk::Box::new(gtk::Orientation::Vertical, 0);
        remaining.add_css_class("folder-context-menu");
        let (hidden_icon, hidden_label) = hidden_files_toggle(hidden_files_shown);
        let (toggle_hidden, toggle_hidden_icon, toggle_hidden_label) =
            context_menu_toggle_option(hidden_icon, hidden_label, ContextHint::HiddenFiles);
        let options = Self {
            content,
            remaining,
            new_folder: context_menu_option(
                icons::FOLDER_PLUS,
                "New Folder",
                ContextHint::NewFolder,
            ),
            new_file: context_menu_option(icons::FILE_PLUS, "New File", ContextHint::None),
            open_with: context_menu_option(icons::APP_WINDOW, "Open With…", ContextHint::None),
            open_terminal: context_menu_option(
                icons::TERMINAL,
                "Open in Terminal",
                ContextHint::Terminal,
            ),
            paste: context_menu_option(icons::CLIPBOARD_PASTE, "Paste", ContextHint::Paste),
            select_all: context_menu_option(
                icons::LIST_CHECKS,
                "Select All",
                ContextHint::SelectAll,
            ),
            refresh: context_menu_option(icons::REFRESH, "Refresh", ContextHint::Refresh),
            toggle_hidden,
            toggle_hidden_icon,
            toggle_hidden_label,
            customize: context_menu_option(icons::PALETTE, "Customize…", ContextHint::None),
            properties: context_menu_option(icons::INFO, "Properties", ContextHint::None),
            directory_actions: !is_trash_location(location)
                && !location.is_recent_location()
                && !location.is_computer_root(),
        };
        options.apply_location(location);
        options.arrange();
        options
    }

    fn apply_location(&self, location: &Location) {
        let directory_actions = self.directory_actions;
        self.customize
            .set_visible(directory_actions && location.native_path().is_some());
        self.new_folder.set_visible(directory_actions);
        self.new_file.set_visible(directory_actions);
        self.open_with
            .set_visible(directory_actions && !location.is_administrator());
        self.open_terminal.set_visible(directory_actions);
        self.paste.set_visible(directory_actions);
        self.properties
            .set_visible(!location.is_recent_location() && !location.is_computer_root());
    }

    fn arrange(&self) {
        self.content.append(&self.new_folder);
        self.content.append(&self.new_file);
        self.content.append(&self.paste);
        self.content
            .append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        self.content.append(&self.open_with);
        self.content.append(&self.open_terminal);
        self.content.set_visible(self.directory_actions);
        self.remaining.append(&self.select_all);
        self.remaining.append(&self.refresh);
        self.remaining.append(&self.toggle_hidden);
        if self.customize.get_visible() || self.properties.get_visible() {
            self.remaining
                .append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        }
        self.remaining.append(&self.customize);
        self.remaining.append(&self.properties);
    }

    fn sync_hidden_files_toggle(&self, shown: bool) {
        let (icon, label) = hidden_files_toggle(shown);
        self.toggle_hidden_label.set_text(&crate::i18n::tr(label));
        crate::assets::set_primary_icon(&self.toggle_hidden_icon, icon);
    }
}

fn clipboard_has_pasteable() -> bool {
    gtk::gdk::Display::default().is_some_and(|display| {
        let formats = display.clipboard().formats();
        formats.contains_type(gtk::gdk::FileList::static_type())
            || formats.contain_mime_type("image/png")
    })
}

/// What every folder-menu handler needs: the view, its popover, and the folder.
#[derive(Clone)]
struct FolderMenu {
    state: Weak<ViewState>,
    popover: glib::WeakRef<gtk::Popover>,
    depth: usize,
    location: Location,
}

impl FolderMenu {
    fn popdown(&self) {
        if let Some(popover) = self.popover.upgrade() {
            popover.popdown();
        }
    }

    /// Closes the menu, then runs `run` while the view is alive.
    fn on_click(&self, button: &gtk::Button, run: impl Fn(&Rc<ViewState>, &FolderMenu) + 'static) {
        let menu = self.clone();
        button.connect_clicked(move |_| {
            menu.popdown();
            if let Some(state) = menu.state.upgrade() {
                run(&state, &menu);
            }
        });
    }

    fn connect_new_entries(&self, new_folder: &gtk::Button, new_file: &gtk::Button) {
        for (button, is_directory) in [(new_folder, true), (new_file, false)] {
            self.on_click(button, move |state, menu| {
                schedule_new_entry(state, menu.depth, menu.location.clone(), is_directory);
            });
        }
    }

    fn connect(&self, options: &FolderMenuOptions, history: &RecentApps) {
        self.connect_new_entries(&options.new_folder, &options.new_file);
        self.on_click(&options.paste, |state, menu| {
            state.paste_into(menu.location.clone());
        });
        self.on_click(&options.select_all, |state, menu| {
            state.select_all(menu.depth)
        });
        self.on_click(&options.refresh, |state, menu| {
            crate::adapters::trash_support::forget_all();
            state.browser.retry_column(menu.depth);
        });
        self.on_click(&options.toggle_hidden, |state, _| {
            state.browser.toggle_hidden()
        });
        self.on_click(&options.customize, |state, menu| {
            let Some(path) = menu.location.native_path() else {
                return;
            };
            show_customize_modal(&state.overlay, path.to_path_buf(), true, icons::FOLDER);
        });
        self.on_click(&options.properties, |state, menu| {
            state.show_folder_properties(&menu.location);
        });
        let history = history.clone();
        self.on_click(&options.open_with, move |state, menu| {
            open_folder_with(state, &menu.location, &history);
        });
        self.on_click(&options.open_terminal, |state, menu| {
            launch_terminal(&menu.location, &state.overlay);
        });
    }
}

fn open_folder_with(state: &Rc<ViewState>, location: &Location, history: &RecentApps) {
    if location.is_recent_location() {
        return;
    }
    let file = gio_file_for_location(location);
    let requires_uris = crate::ui::open_with::requires_uri_handlers(std::slice::from_ref(&file));
    let Some(choices) = history.borrow().as_ref().map(|history| {
        crate::ui::open_with::choices_for_type(history, "inode/directory", requires_uris)
    }) else {
        return;
    };
    let browser = Rc::downgrade(&state.browser);
    crate::ui::open_with::show(
        &state.overlay,
        vec![file],
        vec!["inode/directory".to_string()],
        choices,
        crate::ui::open_with::OpenWithContext::Explicit,
        Rc::new(move || {
            if let Some(browser) = browser.upgrade() {
                browser.focus_active();
            }
        }),
    );
}

fn install_folder_menu_click(
    parent: &gtk::Widget,
    is_item_target: Rc<dyn Fn(&gtk::Widget) -> bool>,
    open_at: Rc<dyn Fn(f64, f64)>,
) {
    let menu_click = gtk::GestureClick::new();
    menu_click.set_button(3);
    menu_click.connect_pressed(move |gesture, _, x, y| {
        let over_item = gesture
            .widget()
            .and_then(|widget| widget.pick(x, y, gtk::PickFlags::DEFAULT))
            .is_some_and(|picked| is_item_target(&picked));
        if over_item {
            return;
        }
        gesture.set_state(gtk::EventSequenceState::Claimed);
        open_at(x, y);
    });
    parent.add_controller(menu_click);
}

pub(in crate::ui) fn install_folder_context_menu(
    state: &Rc<ViewState>,
    parent: &gtk::Widget,
    has_entries: Rc<dyn Fn() -> bool>,
    is_item_target: Rc<dyn Fn(&gtk::Widget) -> bool>,
    depth: usize,
    location: Location,
) -> Rc<dyn Fn(f64, f64)> {
    if !state.interactive {
        return chooser_context::install_folder(state, parent, is_item_target, depth, location);
    }
    let options = FolderMenuOptions::build(state.browser.preferences().show_hidden, &location);
    let action_section = actions::ActionMenuSection::new(
        &options.content,
        &options.remaining,
        None,
        state.overlay.upcast_ref(),
        None,
    );
    let popover = action_section.popover();
    bind_column_context_owner(state, &popover, depth);
    let menu = FolderMenu {
        state: Rc::downgrade(state),
        popover: popover.downgrade(),
        depth,
        location,
    };
    let open_with_history: RecentApps = Rc::new(RefCell::new(None));
    menu.connect(&options, &open_with_history);

    let browser = state.browser.clone();
    let parent_for_trigger = parent.downgrade();
    let open_with_generation = Rc::new(Cell::new(0_u64));
    let open_at: Rc<dyn Fn(f64, f64)> = Rc::new(move |x: f64, y: f64| {
        options.paste.set_sensitive(clipboard_has_pasteable());
        options.select_all.set_sensitive(has_entries());
        options
            .open_terminal
            .set_sensitive(can_open_terminal(&menu.location));
        if options.directory_actions {
            prepare_folder_open_with(
                &options.open_with,
                &open_with_history,
                &open_with_generation,
            );
        }
        options.sync_hidden_files_toggle(browser.preferences().show_hidden);
        if let Some(state) = menu.state.upgrade()
            && let Some(parent) = parent_for_trigger.upgrade()
        {
            focus_context_column(&state, depth);
            action_section.rebuild_for_folder(&state, &menu.location);
            action_section.show(&parent, x, y);
        }
    });
    install_folder_menu_click(parent, is_item_target, open_at.clone());
    open_at
}
