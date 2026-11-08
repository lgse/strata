// SPDX-License-Identifier: MIT

//! Keeps the manager current with files that change behind it: the user
//! stylesheet, Omarchy's theme state, the custom theme folder and the targets
//! its dotfile links point at, and the desktop's text scaling.

use std::{
    cell::RefCell,
    fs, io,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use gtk::{gio, glib, prelude::*};

use super::super::preferences::OmarchyVariant;
use super::{
    OmarchyRefresh, Theme, ThemeManager, builtins, is_custom_theme_event, is_omarchy_theme_event,
    linked_theme_targets, load_custom_themes, merge_builtin_and_custom_themes, themes_directory,
};
use crate::{adapters::Watched, model::Location, services::LoadHandle};

const USER_STYLE_FILE: &str = "style.css";

/// A refresh waiting out a burst of file events, cancelled by the next event.
type PendingRefresh = RefCell<Option<glib::SourceId>>;

impl ThemeManager {
    /// Coalesces a burst of change events into one `refresh` after they settle.
    fn schedule_refresh(
        self: &Rc<Self>,
        pending: fn(&Self) -> &PendingRefresh,
        refresh: fn(&Rc<Self>),
    ) {
        if let Some(previous) = pending(self).borrow_mut().take() {
            previous.remove();
        }
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local_once(Duration::from_millis(75), move || {
            let Some(manager) = weak.upgrade() else {
                return;
            };
            pending(&manager).borrow_mut().take();
            refresh(&manager);
        });
        pending(self).replace(Some(source));
    }

    /// Watches `location`, scheduling `refresh` on each event `relevant` accepts and once
    /// the watch goes live, which catches a write made before it did.
    fn watch_for_refresh(
        self: &Rc<Self>,
        target: (&Location, Watched),
        relevant: impl Fn(&Rc<Self>, &gio::File, Option<&gio::File>) -> bool + 'static,
        schedule: fn(&Rc<Self>),
    ) -> LoadHandle {
        let (changed, live) = (Rc::downgrade(self), Rc::downgrade(self));
        crate::adapters::watch(
            target.0,
            target.1,
            move |file, other_file| {
                let Some(manager) = changed.upgrade() else {
                    return;
                };
                if relevant(&manager, file, other_file) {
                    schedule(&manager);
                }
            },
            move || {
                if let Some(manager) = live.upgrade() {
                    schedule(&manager);
                }
            },
        )
    }

    /// A missing or unreadable file clears the provider, so deleting
    /// `style.css` restores Strata's own appearance without a restart.
    fn refresh_user_stylesheet(self: &Rc<Self>) {
        let read = crate::storage::writer::run(read_user_stylesheet);
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            if let (Ok(css), Some(manager)) = (read.await, weak.upgrade()) {
                manager.user_provider.load_from_string(&css);
            }
        });
    }

    pub(super) fn monitor_user_stylesheet(self: &Rc<Self>) {
        let monitor = self.watch_for_refresh(
            (&Location::local(user_stylesheet_path()), Watched::File),
            |_, _, _| true,
            Self::schedule_user_style_refresh,
        );
        self.user_style_monitor.replace(Some(monitor));
    }

    fn schedule_user_style_refresh(self: &Rc<Self>) {
        self.schedule_refresh(
            |manager| &manager.pending_user_style_refresh,
            Self::refresh_user_stylesheet,
        );
    }

    pub(super) fn monitor_text_scaling(self: &Rc<Self>) {
        let Some(settings) = gtk::Settings::default() else {
            return;
        };
        let weak = Rc::downgrade(self);
        settings.connect_gtk_xft_dpi_notify(move |_| {
            let Some(manager) = weak.upgrade() else {
                return;
            };
            manager.apply_selected();
        });
    }

    pub(super) fn monitor_omarchy(
        self: &Rc<Self>,
        directories: &[PathBuf],
        theme_directory: Option<&Path>,
    ) {
        let mut monitors = self.omarchy_monitors.take();
        monitors.retain(|(path, _)| directories.contains(path));
        for path in directories {
            if monitors.iter().any(|(watched, _)| watched == path) {
                continue;
            }
            let colors =
                (Some(path.as_path()) == theme_directory).then(|| path.join("colors.toml"));
            let relevant = move |file: &gio::File| {
                is_omarchy_theme_event(file)
                    || colors
                        .as_ref()
                        .is_some_and(|colors| file.path().as_ref() == Some(colors))
            };
            let monitor = self.watch_for_refresh(
                (&Location::local(path), Watched::Folder),
                move |_, file, other_file| relevant(file) || other_file.is_some_and(&relevant),
                Self::schedule_omarchy_refresh,
            );
            monitors.push((path.clone(), monitor));
        }
        self.omarchy_monitors.replace(monitors);
    }

    fn schedule_omarchy_refresh(self: &Rc<Self>) {
        self.schedule_refresh(
            |manager| &manager.pending_omarchy_refresh,
            Self::refresh_omarchy_state,
        );
    }

    pub(in crate::ui) fn refresh_omarchy_state(self: &Rc<Self>) {
        let read = crate::storage::writer::run(OmarchyRefresh::read);
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            if let (Ok(refresh), Some(manager)) = (read.await, weak.upgrade()) {
                manager.apply_omarchy_refresh(refresh);
            }
        });
    }

    fn apply_omarchy_refresh(self: &Rc<Self>, refresh: OmarchyRefresh) {
        self.monitor_omarchy(&refresh.directories, refresh.theme_directory.as_deref());
        let unchanged = self.omarchy_state.replace(refresh.state.clone()) == refresh.state;
        if let Some(state) = &refresh.state {
            self.preferences.set_active_omarchy_theme(&state.name);
            self.appearance.borrow_mut().omarchy_variant = self.preferences.omarchy_variant();
        }
        let available = self.omarchy_still_available(refresh.name_exists);
        let availability_changed = self.omarchy_available.replace(available) != available;
        if !available && self.follows_omarchy() {
            self.preview.take();
            self.preferences.set_theme_selection("theme", None);
            return;
        }
        if availability_changed {
            self.preferences.notify_changes();
            return;
        }
        if !unchanged {
            self.reapply_omarchy_theme();
        }
    }

    /// A theme whose colors are unreadable mid-switch stays available while its name exists.
    fn omarchy_still_available(&self, name_exists: bool) -> bool {
        self.omarchy_tokens(OmarchyVariant::Lighter).is_some()
            || (self.is_omarchy_available() && name_exists)
    }

    fn reapply_omarchy_theme(&self) {
        if self.follows_omarchy() {
            self.apply_selected();
        }
        // Republishes the variant remembered for a newly active theme.
        self.preferences.notify_changes();
    }

    pub(super) fn monitor_custom_themes(self: &Rc<Self>, directory_exists: bool) {
        if !directory_exists || self.themes_monitor.borrow().is_some() {
            return;
        }
        let monitor = self.watch_for_refresh(
            (&Location::local(themes_directory()), Watched::Folder),
            |_, file, other_file| {
                is_custom_theme_event(file) || other_file.is_some_and(is_custom_theme_event)
            },
            Self::schedule_themes_refresh,
        );
        self.themes_monitor.replace(Some(monitor));
    }

    /// Writes through a dotfiles symlink only reach the target's directory.
    pub(super) fn monitor_linked_themes(self: &Rc<Self>, targets: Vec<PathBuf>) {
        let directories = parent_directories(&targets);
        self.linked_theme_targets.replace(targets);
        let mut monitors = self.linked_theme_monitors.take();
        monitors.retain(|(path, _)| directories.contains(path));
        for path in directories {
            if monitors.iter().any(|(watched, _)| *watched == path) {
                continue;
            }
            let monitor = self.watch_for_refresh(
                (&Location::local(&path), Watched::Folder),
                |manager, file, other_file| {
                    manager.is_linked_theme(file)
                        || other_file.is_some_and(|other| manager.is_linked_theme(other))
                },
                Self::schedule_themes_refresh,
            );
            monitors.push((path, monitor));
        }
        self.linked_theme_monitors.replace(monitors);
    }

    fn is_linked_theme(&self, file: &gio::File) -> bool {
        file.path()
            .is_some_and(|path| self.linked_theme_targets.borrow().contains(&path))
    }

    fn schedule_themes_refresh(self: &Rc<Self>) {
        self.schedule_refresh(
            |manager| &manager.pending_themes_refresh,
            Self::refresh_custom_themes,
        );
    }

    pub(in crate::ui) fn refresh_custom_themes(self: &Rc<Self>) {
        let loaded = crate::storage::writer::run(|| {
            (
                load_custom_themes(),
                linked_theme_targets(&themes_directory()),
            )
        });
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            if let (Ok((custom, targets)), Some(manager)) = (loaded.await, weak.upgrade()) {
                // Links may have been added, removed, or repointed.
                manager.monitor_linked_themes(targets);
                manager.adopt_custom_themes(custom);
            }
        });
    }

    /// A selected file caught mid-write, invalid, or removed keeps its last
    /// valid colors until a read succeeds again.
    fn adopt_custom_themes(&self, custom: Vec<Theme>) {
        let selected = self.preferences.selected_theme_id();
        let effective = self.effective_theme_id();
        let before = self.current_tokens();
        let mut themes = merge_builtin_and_custom_themes(builtins(), custom);
        for retained in [selected, effective] {
            if !themes.iter().any(|theme| theme.id == retained)
                && let Some(previous) = self.theme(&retained)
            {
                themes.push(previous);
            }
        }
        self.themes.replace(themes);
        let after = self.current_tokens();
        let colors_kept = after.is_none() || after == before;
        if self.follows_omarchy() || colors_kept {
            return;
        }
        self.apply_selected();
        self.preferences.notify_changes();
    }

    fn theme(&self, id: &str) -> Option<Theme> {
        self.themes
            .borrow()
            .iter()
            .find(|theme| theme.id == id)
            .cloned()
    }
}

/// Each target's directory, once.
fn parent_directories(targets: &[PathBuf]) -> Vec<PathBuf> {
    let mut directories: Vec<PathBuf> = Vec::new();
    for parent in targets.iter().filter_map(|target| target.parent()) {
        if !directories.iter().any(|path| path == parent) {
            directories.push(parent.to_path_buf());
        }
    }
    directories
}

pub(super) fn user_stylesheet_path() -> PathBuf {
    super::super::preferences::config_directory().join(USER_STYLE_FILE)
}

pub(super) fn read_user_stylesheet() -> String {
    let path = user_stylesheet_path();
    match fs::read_to_string(&path) {
        Ok(css) => css,
        Err(error) => {
            if error.kind() != io::ErrorKind::NotFound {
                tracing::warn!(%error, path = %path.display(), "cannot read user stylesheet");
            }
            String::new()
        }
    }
}
