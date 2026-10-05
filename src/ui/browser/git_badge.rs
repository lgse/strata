// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
};

use gtk::{glib, prelude::*};

use crate::{
    model::Location,
    services::{GitHead, GitService, GitStatus},
    ui::preferences::PreferenceManager,
};

#[cfg(test)]
mod tests;

const STATUS_CLASSES: [&str; 3] = [
    "git-badge-modified",
    "git-badge-untracked",
    "git-badge-ignored",
];

thread_local! {
    static TRACKED_BADGES: RefCell<HashMap<usize, TrackedBadge>> = RefCell::new(HashMap::new());
    static TRACKED_BRANCHES: RefCell<HashMap<usize, TrackedBranch>> = RefCell::new(HashMap::new());
    static LISTENER_INITIALIZED: RefCell<bool> = const { RefCell::new(false) };
}

#[derive(Clone)]
struct TrackedBadge {
    badge: glib::WeakRef<gtk::Label>,
    path: PathBuf,
    is_directory: bool,
}

#[derive(Clone)]
struct TrackedBranch {
    indicator: glib::WeakRef<gtk::Box>,
    path: PathBuf,
}

pub(in crate::ui) fn init_git_badge_listener() {
    LISTENER_INITIALIZED.with(|init| {
        if init.replace(true) {
            return;
        }
        GitService::add_listener(refresh_git_indicators_for_root);
    });
}

pub(in crate::ui) fn create_git_badge() -> gtk::Label {
    let badge = gtk::Label::new(None);
    badge.add_css_class("git-badge");
    badge.set_valign(gtk::Align::Center);
    badge.set_halign(gtk::Align::Center);
    badge.set_single_line_mode(true);
    badge.set_xalign(0.5);
    badge.set_yalign(0.5);
    badge.set_visible(false);
    badge
}

pub(in crate::ui) fn create_git_branch_indicator() -> gtk::Box {
    let indicator = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    indicator.add_css_class("git-branch-indicator");
    indicator.set_valign(gtk::Align::Center);
    indicator.set_visible(false);

    let icon = crate::assets::primary_icon(crate::assets::icons::GIT_BRANCH, 13);
    icon.set_accessible_role(gtk::AccessibleRole::Presentation);
    let label = gtk::Label::new(None);
    label.add_css_class("git-branch-label");
    label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    label.set_max_width_chars(24);
    label.set_xalign(0.0);
    indicator.append(&icon);
    indicator.append(&label);
    indicator
}

fn clear_status_badge(badge: &gtk::Label) {
    badge.set_visible(false);
    badge.set_text("");
    for class in STATUS_CLASSES {
        badge.remove_css_class(class);
    }
}

fn render_status_badge(badge: &gtk::Label, status: Option<GitStatus>) {
    let Some(status) = status else {
        clear_status_badge(badge);
        return;
    };
    for class in STATUS_CLASSES {
        badge.remove_css_class(class);
    }
    badge.add_css_class(status.css_class());
    badge.set_text(status.badge_text());
    crate::ui::accessibility::set_label(badge, status.label());
    badge.set_visible(true);
}

fn apply_git_status_badge(badge: &gtk::Label, location: &Location, is_directory: bool) {
    if !PreferenceManager::shared().git_status_badges() {
        clear_status_badge(badge);
        return;
    }
    let Some(path) = location.native_path() else {
        clear_status_badge(badge);
        return;
    };
    render_status_badge(badge, GitService::status_for_path(path, is_directory));
}

pub(in crate::ui) fn track_git_badge(badge: &gtk::Label, location: &Location, is_directory: bool) {
    init_git_badge_listener();
    untrack_git_badge(badge);
    apply_git_status_badge(badge, location, is_directory);
    let Some(path) = location.native_path() else {
        return;
    };

    TRACKED_BADGES.with(|tracked| {
        let mut tracked = tracked.borrow_mut();
        tracked.retain(|_, item| item.badge.upgrade().is_some());
        tracked.insert(
            badge.as_ptr() as usize,
            TrackedBadge {
                badge: badge.downgrade(),
                path: path.to_path_buf(),
                is_directory,
            },
        );
    });
}

pub(in crate::ui) fn untrack_git_badge(badge: &gtk::Label) {
    TRACKED_BADGES.with(|tracked| {
        tracked.borrow_mut().remove(&(badge.as_ptr() as usize));
    });
}

pub(in crate::ui) fn suspend_git_badge(badge: &gtk::Label) {
    untrack_git_badge(badge);
    badge.set_visible(false);
}

fn branch_label(indicator: &gtk::Box) -> Option<gtk::Label> {
    indicator.last_child()?.downcast().ok()
}

fn render_git_branch(indicator: &gtk::Box, head: Option<GitHead>) {
    let Some(label) = branch_label(indicator) else {
        indicator.set_visible(false);
        return;
    };
    let Some(head) = head else {
        label.set_text("");
        indicator.set_tooltip_text(None);
        indicator.remove_css_class("detached");
        indicator.set_visible(false);
        return;
    };

    let display_text = head.display_text();
    label.set_text(&display_text);
    indicator.set_tooltip_text(Some(&display_text));
    crate::ui::accessibility::set_label(&label, &head.accessible_label());
    if head.is_detached() {
        indicator.add_css_class("detached");
    } else {
        indicator.remove_css_class("detached");
    }
    indicator.set_visible(true);
}

fn apply_git_branch(indicator: &gtk::Box, path: &Path) {
    if !PreferenceManager::shared().git_status_badges() {
        render_git_branch(indicator, None);
        return;
    }
    render_git_branch(indicator, GitService::head_for_path(path));
}

pub(in crate::ui) fn track_git_branch(indicator: &gtk::Box, location: &Location) {
    init_git_badge_listener();
    untrack_git_branch(indicator);
    let Some(path) = location.native_path() else {
        render_git_branch(indicator, None);
        return;
    };
    apply_git_branch(indicator, path);

    TRACKED_BRANCHES.with(|tracked| {
        let mut tracked = tracked.borrow_mut();
        tracked.retain(|_, item| item.indicator.upgrade().is_some());
        tracked.insert(
            indicator.as_ptr() as usize,
            TrackedBranch {
                indicator: indicator.downgrade(),
                path: path.to_path_buf(),
            },
        );
    });
}

fn untrack_git_branch(indicator: &gtk::Box) {
    TRACKED_BRANCHES.with(|tracked| {
        tracked.borrow_mut().remove(&(indicator.as_ptr() as usize));
    });
}

fn refresh_git_indicators_for_root(root: &Path) {
    let badges =
        TRACKED_BADGES.with(|tracked| tracked.borrow().values().cloned().collect::<Vec<_>>());
    for item in badges {
        if item.path.starts_with(root)
            && let Some(badge) = item.badge.upgrade()
        {
            apply_git_status_badge(&badge, &Location::local(&item.path), item.is_directory);
        }
    }

    let branches =
        TRACKED_BRANCHES.with(|tracked| tracked.borrow().values().cloned().collect::<Vec<_>>());
    for item in branches {
        if item.path.starts_with(root)
            && let Some(indicator) = item.indicator.upgrade()
        {
            apply_git_branch(&indicator, &item.path);
        }
    }
}

pub(in crate::ui) fn refresh_all_git_indicators() {
    let badges =
        TRACKED_BADGES.with(|tracked| tracked.borrow().values().cloned().collect::<Vec<_>>());
    for item in badges {
        if let Some(badge) = item.badge.upgrade() {
            apply_git_status_badge(&badge, &Location::local(&item.path), item.is_directory);
        }
    }

    let branches =
        TRACKED_BRANCHES.with(|tracked| tracked.borrow().values().cloned().collect::<Vec<_>>());
    for item in branches {
        if let Some(indicator) = item.indicator.upgrade() {
            apply_git_branch(&indicator, &item.path);
        }
    }
}
