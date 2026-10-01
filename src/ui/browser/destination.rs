// SPDX-License-Identifier: MIT

use crate::services::{SearchEvent, index_tree};
use crate::ui::browser::paths::compact_native_path;
use crate::ui::controls::{form_entry, form_label, navigation_button};
use crate::ui::{PlaceGroup, PlaceShortcut, sidebar_button};
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use super::location::is_breadcrumb_button_target;

struct TransferSearchScope {
    base: PathBuf,
    search_root: PathBuf,
    root_limit: Option<PathBuf>,
    show_hidden: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EmptySuggestions {
    NoSubfolders,
    NoMatches,
}

impl EmptySuggestions {
    fn label(self) -> &'static str {
        match self {
            Self::NoSubfolders => "No subfolders",
            Self::NoMatches => "No matching folders",
        }
    }
}

struct PathSuggestions {
    paths: Vec<PathBuf>,
    empty: EmptySuggestions,
}

#[derive(Default)]
struct DestinationHistory {
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
}

impl DestinationHistory {
    fn visit(&mut self, from: &Path, to: &Path) {
        if from == to {
            return;
        }
        self.back.push(from.to_path_buf());
        self.forward.clear();
    }

    fn go_back(&mut self, from: &Path) -> Option<PathBuf> {
        let target = self.back.pop()?;
        self.forward.push(from.to_path_buf());
        Some(target)
    }

    fn go_forward(&mut self, from: &Path) -> Option<PathBuf> {
        let target = self.forward.pop()?;
        self.back.push(from.to_path_buf());
        Some(target)
    }
}

fn append_empty_label(suggestions: &gtk::Box, empty: EmptySuggestions) {
    let label = gtk::Label::new(Some(empty.label()));
    label.add_css_class("transfer-suggestions-empty");
    label.set_xalign(0.0);
    suggestions.append(&label);
}

fn looks_like_path(input: &str) -> bool {
    input.trim().contains(std::path::MAIN_SEPARATOR)
        || input.trim().starts_with('~')
        || input.trim().is_empty()
}

fn set_destination_entry(field: &gtk::Entry, path: &Path) {
    field.remove_css_class("error");
    field.set_text(&folder_input_path(path));
    field.set_position(-1);
    field.grab_focus();
}

// GtkEntry delegates focus to GtkText; move focus before modal teardown.
pub(super) fn hand_off_destination_focus(field: &gtk::Entry, confirm: &gtk::Button) {
    let entry_focused = field.has_focus()
        || field
            .root()
            .and_downcast::<gtk::Window>()
            .and_then(|window| gtk::prelude::GtkWindowExt::focus(&window))
            .is_some_and(|focus| focus.is_ancestor(field));
    if entry_focused {
        confirm.grab_focus();
    }
}

fn render_transfer_suggestions(
    suggestions: &gtk::Box,
    items: Vec<crate::services::SearchItem>,
    root_limit: Option<&Path>,
    on_select: &Rc<dyn Fn(&Path)>,
) {
    while let Some(child) = suggestions.first_child() {
        suggestions.remove(&child);
    }
    let mut dirs: Vec<_> = items
        .into_iter()
        .filter_map(|mut item| {
            if !item.is_directory {
                return None;
            }
            if let Some(root) = root_limit {
                item.path = canonical_directory_within(root, &item.path)?;
            }
            Some(item)
        })
        .collect();
    dirs.sort_by_key(|item| item.path.ancestors().count());
    dirs.truncate(8);
    if dirs.is_empty() {
        append_empty_label(suggestions, EmptySuggestions::NoMatches);
        return;
    }
    for item in dirs {
        append_suggestion(suggestions, &item.path, on_select);
    }
}

fn append_suggestion(suggestions: &gtk::Box, path: &Path, on_select: &Rc<dyn Fn(&Path)>) {
    let option = gtk::Button::new();
    option.add_css_class("transfer-suggestion");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 9);
    row.append(&crate::assets::primary_icon(
        crate::assets::icons::FOLDER,
        16,
    ));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let label = gtk::Label::new(Some(&name));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    row.append(&label);
    if let Some(parent) = path.parent().map(compact_native_path) {
        let parent_label = gtk::Label::new(Some(&parent));
        parent_label.add_css_class("transfer-suggestion-parent");
        parent_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        parent_label.set_xalign(1.0);
        row.append(&parent_label);
    }
    option.set_child(Some(&row));
    option.set_widget_name(&path.to_string_lossy());
    crate::ui::accessibility::set_description(&option, Some(&path.to_string_lossy()));
    let select = on_select.clone();
    let path = path.to_path_buf();
    option.connect_clicked(move |_| select(&path));
    suggestions.append(&option);
}

fn setup_transfer_search(
    field: &gtk::Entry,
    suggestions: &gtk::Box,
    generation: &Rc<Cell<u64>>,
    scope: TransferSearchScope,
    on_select: Rc<dyn Fn(&Path)>,
    on_changed: impl Fn(&gtk::Entry) + 'static,
) {
    let TransferSearchScope {
        base,
        search_root,
        root_limit,
        show_hidden,
    } = scope;
    let (search_handle, search_receiver) = index_tree(search_root, show_hidden);
    let search_handle = Rc::new(search_handle);
    let query_handle = Rc::downgrade(&search_handle);
    let search_mode = Rc::new(Cell::new(false));
    let poll_suggestions = suggestions.downgrade();
    let poll_field = field.downgrade();
    let poll_mode = search_mode.clone();
    let poll_root_limit = root_limit.clone();
    let poll_select = on_select.clone();
    let _poll = glib::timeout_add_local(Duration::from_millis(16), move || {
        let _keep_search_alive = &search_handle;
        let (Some(suggestions), Some(field)) = (poll_suggestions.upgrade(), poll_field.upgrade())
        else {
            return glib::ControlFlow::Break;
        };
        if field.root().is_none() {
            return glib::ControlFlow::Break;
        }
        if !poll_mode.get() {
            return glib::ControlFlow::Continue;
        }
        let mut latest = None;
        for _ in 0..8 {
            match search_receiver.try_recv() {
                Ok(event) => latest = Some(event),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    return glib::ControlFlow::Break;
                }
            }
        }
        if let Some(SearchEvent::Results { query, items, .. }) = latest
            && query == field.text().trim()
        {
            render_transfer_suggestions(
                &suggestions,
                items,
                poll_root_limit.as_deref(),
                &poll_select,
            );
        }
        glib::ControlFlow::Continue
    });
    let suggestions_clone = suggestions.clone();
    let generation_clone = generation.clone();
    field.connect_changed(move |field| {
        on_changed(field);
        let input = field.text().to_string();
        let request = generation_clone.get().saturating_add(1);
        generation_clone.set(request);
        let looks_like_path = looks_like_path(&input);
        if looks_like_path {
            search_mode.set(false);
            let gen_check = generation_clone.clone();
            let home = glib::home_dir();
            let base = base.clone();
            let root_limit = root_limit.clone();
            let completed_select = on_select.clone();
            let suggestions_clone = suggestions_clone.clone();
            glib::MainContext::default().spawn_local(async move {
                let matches = gio::spawn_blocking(move || {
                    path_suggestions(&input, &base, &home, root_limit.as_deref())
                })
                .await;
                if gen_check.get() != request {
                    return;
                }
                while let Some(child) = suggestions_clone.first_child() {
                    suggestions_clone.remove(&child);
                }
                let Ok(PathSuggestions { paths, empty }) = matches else {
                    return;
                };
                if paths.is_empty() {
                    append_empty_label(&suggestions_clone, empty);
                }
                for path in paths {
                    append_suggestion(&suggestions_clone, &path, &completed_select);
                }
            });
        } else {
            search_mode.set(true);
            if let Some(search_handle) = query_handle.upgrade() {
                search_handle.query(&input);
            }
        }
    });
}

pub(super) fn folder_input_path(path: &Path) -> String {
    let path = compact_native_path(path);
    if path.ends_with(std::path::MAIN_SEPARATOR) {
        path
    } else {
        format!("{path}{}", std::path::MAIN_SEPARATOR)
    }
}

pub(super) fn resolve_destination_path(input: &str, base: &Path, home: &Path) -> PathBuf {
    let input = input.trim();
    if input == "~" {
        home.to_path_buf()
    } else if let Some(relative) = input.strip_prefix("~/") {
        home.join(relative)
    } else {
        let path = Path::new(input);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            base.join(path)
        }
    }
}

fn path_suggestions(
    input: &str,
    base: &Path,
    home: &Path,
    root_limit: Option<&Path>,
) -> PathSuggestions {
    let no_matches = PathSuggestions {
        paths: Vec::new(),
        empty: EmptySuggestions::NoMatches,
    };
    let resolved = resolve_destination_path(input, base, home);
    let trailing_separator = input.trim_end().ends_with(std::path::MAIN_SEPARATOR);
    let (directory, prefix) = if trailing_separator {
        (resolved, String::new())
    } else {
        (
            resolved.parent().unwrap_or(base).to_path_buf(),
            resolved
                .file_name()
                .map(|name| name.to_string_lossy().to_lowercase())
                .unwrap_or_default(),
        )
    };
    let directory = match root_limit {
        Some(root) => {
            let Some(directory) = canonical_directory_within(root, &directory) else {
                return no_matches;
            };
            directory
        }
        None => directory,
    };
    let Ok(children) = std::fs::read_dir(directory) else {
        return no_matches;
    };
    let mut matches = children
        .filter_map(Result::ok)
        .map(|child| child.path())
        .filter_map(|path| {
            if !path.is_dir() {
                return None;
            }
            path.file_name()
                .is_some_and(|name| {
                    let name = name.to_string_lossy().to_lowercase();
                    (prefix.starts_with('.') || !name.starts_with('.')) && name.starts_with(&prefix)
                })
                .then_some(())?;
            match root_limit {
                Some(root) => canonical_directory_within(root, &path),
                None => Some(path),
            }
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|path| {
        path.file_name()
            .map(|name| name.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    });
    // An empty prefix lists a directory's children for the scrollable
    // suggestions; only a typed prefix is an autocomplete window.
    if !prefix.is_empty() {
        matches.truncate(8);
    }
    PathSuggestions {
        paths: matches,
        empty: if prefix.is_empty() {
            EmptySuggestions::NoSubfolders
        } else {
            EmptySuggestions::NoMatches
        },
    }
}

/// The folder Up leads to, or `None` in name search, at the filesystem root,
/// and at or outside a confining root.
fn parent_destination(
    input: &str,
    base: &Path,
    root_limit: Option<&Path>,
    canonical_root: Option<&Path>,
    home: &Path,
) -> Option<PathBuf> {
    if !looks_like_path(input) {
        return None;
    }
    let resolved = resolve_destination_path(input, base, home);
    if root_limit.is_none() {
        return resolved.parent().map(Path::to_path_buf);
    }
    let root = canonical_root?;
    let current = canonical_directory_within(root, &resolved)?;
    if current == root {
        return None;
    }
    current.parent().map(Path::to_path_buf)
}

pub(super) fn canonical_existing_directory(path: &Path) -> Option<PathBuf> {
    let canonical = std::fs::canonicalize(path).ok()?;
    canonical.is_dir().then_some(canonical)
}

pub(super) fn canonical_directory_within(root: &Path, candidate: &Path) -> Option<PathBuf> {
    let root = canonical_existing_directory(root)?;
    let candidate = canonical_existing_directory(candidate)?;
    candidate.strip_prefix(root).ok()?;
    Some(candidate)
}

pub(super) fn rebind_directory_within_root(
    opened_root: &Path,
    selected: &Path,
    current_root: &Path,
) -> Option<PathBuf> {
    let relative = selected.strip_prefix(opened_root).ok()?;
    let current_root = canonical_existing_directory(current_root)?;
    canonical_directory_within(&current_root, &current_root.join(relative))
}

const DESTINATION_CRUMB_LABEL_MAX_CHARS: i32 = 32;

fn ellipsize_destination_crumb(label: &gtk::Label) {
    label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    label.set_max_width_chars(DESTINATION_CRUMB_LABEL_MAX_CHARS);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DestinationCrumbKind {
    Ancestor,
    Current,
    Scope,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DestinationCrumb {
    label: String,
    target: PathBuf,
    kind: DestinationCrumbKind,
}

fn path_crumb_label(path: &Path, home: &Path) -> String {
    if path == home {
        return "~".to_owned();
    }
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn scope_crumb_label(
    search_root: &Path,
    root_limit: Option<&Path>,
    root_label: Option<&str>,
    home: &Path,
) -> String {
    if root_limit.is_some() {
        return root_label.map(str::to_owned).unwrap_or_else(|| {
            search_root
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| search_root.to_string_lossy().into_owned())
        });
    }
    if search_root == home {
        return "~".to_owned();
    }
    path_crumb_label(search_root, home)
}

fn confined_destination_crumbs(
    resolved: &Path,
    root: &Path,
    canonical_root: Option<&Path>,
    root_label: Option<&str>,
) -> Vec<DestinationCrumb> {
    let root_name = || {
        root_label
            .map(str::to_owned)
            .or_else(|| {
                root.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| root.to_string_lossy().into_owned())
    };
    let Some(canonical_root) = canonical_root else {
        return vec![DestinationCrumb {
            label: root_name(),
            target: root.to_path_buf(),
            kind: DestinationCrumbKind::Scope,
        }];
    };
    let canonical_root = canonical_root.to_path_buf();
    // Confirmation re-validates against the current device root.
    let Some(canonical) = canonical_existing_directory(resolved)
        .filter(|candidate| candidate.strip_prefix(&canonical_root).is_ok())
    else {
        return vec![DestinationCrumb {
            label: root_name(),
            target: canonical_root.clone(),
            kind: DestinationCrumbKind::Scope,
        }];
    };
    let Ok(relative) = canonical.strip_prefix(&canonical_root) else {
        return vec![DestinationCrumb {
            label: root_name(),
            target: canonical_root,
            kind: DestinationCrumbKind::Current,
        }];
    };
    let mut crumbs = vec![DestinationCrumb {
        label: root_name(),
        target: canonical_root.clone(),
        kind: DestinationCrumbKind::Ancestor,
    }];
    let mut prefix = canonical_root;
    for component in relative.components() {
        prefix.push(component);
        crumbs.push(DestinationCrumb {
            label: component.as_os_str().to_string_lossy().into_owned(),
            target: prefix.clone(),
            kind: DestinationCrumbKind::Ancestor,
        });
    }
    if let Some(last) = crumbs.last_mut() {
        last.kind = DestinationCrumbKind::Current;
    }
    crumbs
}

fn lexical_destination_crumbs(resolved: &Path, home: &Path) -> Vec<DestinationCrumb> {
    let mut chain: Vec<PathBuf> = resolved.ancestors().map(Path::to_path_buf).collect();
    chain.reverse();
    if let Some(home_index) = chain.iter().position(|ancestor| ancestor.as_path() == home) {
        chain.drain(..home_index);
    }
    let last = chain.len().saturating_sub(1);
    chain
        .into_iter()
        .enumerate()
        .map(|(index, target)| DestinationCrumb {
            label: path_crumb_label(&target, home),
            target,
            kind: if index == last {
                DestinationCrumbKind::Current
            } else {
                DestinationCrumbKind::Ancestor
            },
        })
        .collect()
}

fn destination_crumbs(
    input: &str,
    base: &Path,
    search_root: &Path,
    root_limit: Option<&Path>,
    canonical_root: Option<&Path>,
    root_label: Option<&str>,
    home: &Path,
) -> Vec<DestinationCrumb> {
    if !looks_like_path(input) {
        return vec![DestinationCrumb {
            label: scope_crumb_label(search_root, root_limit, root_label, home),
            target: search_root.to_path_buf(),
            kind: DestinationCrumbKind::Scope,
        }];
    }
    let resolved = resolve_destination_path(input, base, home);
    if let Some(root) = root_limit {
        confined_destination_crumbs(&resolved, root, canonical_root, root_label)
    } else {
        lexical_destination_crumbs(&resolved, home)
    }
}

pub(super) struct DestinationLocationBar {
    stack: gtk::Stack,
    crumbs: gtk::Box,
    crumb_scroll: gtk::ScrolledWindow,
    field: gtk::Entry,
    base: PathBuf,
    search_root: PathBuf,
    root_limit: Option<PathBuf>,
    canonical_root: Option<PathBuf>,
    root_label: Option<String>,
    edit_start_text: RefCell<String>,
    last_navigable: RefCell<Option<gtk::Button>>,
    history: RefCell<DestinationHistory>,
    last_directory: RefCell<PathBuf>,
    navigation: RefCell<Option<DestinationNavigation>>,
}

struct DestinationNavigation {
    back: gtk::Button,
    forward: gtk::Button,
    up: gtk::Button,
    places: Vec<(PathBuf, gtk::Button)>,
    suggestions: glib::WeakRef<gtk::Box>,
}

const BROWSE_CHILD: &str = "browse";
const EDIT_CHILD: &str = "edit";

impl DestinationLocationBar {
    pub(super) fn wrap(
        field: gtk::Entry,
        base: PathBuf,
        search_root: PathBuf,
        root_limit: Option<PathBuf>,
        root_label: Option<String>,
    ) -> Rc<Self> {
        let crumbs = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        crumbs.add_css_class("breadcrumbs");
        let crumb_scroll = gtk::ScrolledWindow::builder()
            .child(&crumbs)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .build();
        let stack = gtk::Stack::new();
        stack.add_css_class("destination-location-stack");
        stack.add_css_class("destination-location-bar");
        stack.add_named(&crumb_scroll, Some(BROWSE_CHILD));
        stack.add_named(&field, Some(EDIT_CHILD));
        stack.set_visible_child_name(BROWSE_CHILD);
        let canonical_root = root_limit.as_deref().and_then(canonical_existing_directory);
        let bar = Rc::new(Self {
            stack,
            crumbs,
            crumb_scroll,
            field,
            search_root,
            root_limit,
            canonical_root,
            root_label,
            edit_start_text: RefCell::new(String::new()),
            last_navigable: RefCell::new(None),
            history: RefCell::default(),
            last_directory: RefCell::new(base.clone()),
            navigation: RefCell::new(None),
            base,
        });
        {
            let changed_bar = bar.clone();
            bar.field.connect_changed(move |_| changed_bar.rebuild());
        }
        bar.crumb_scroll.set_cursor_from_name(Some("text"));
        {
            let editor = bar.clone();
            let edit_location = gtk::GestureClick::new();
            edit_location.connect_released(move |gesture, _, x, y| {
                let clicked_button = gesture
                    .widget()
                    .and_then(|widget| widget.pick(x, y, gtk::PickFlags::DEFAULT))
                    .is_some_and(is_breadcrumb_button_target);
                if !clicked_button {
                    editor.begin_edit();
                }
            });
            bar.crumb_scroll.add_controller(edit_location);
        }
        {
            let keys_bar = bar.clone();
            let keys = gtk::EventControllerKey::new();
            keys.connect_key_pressed(move |_, key, _, state| keys_bar.handle_key(key, state));
            bar.field.add_controller(keys);
        }
        {
            let error_bar = bar.clone();
            bar.field.connect_css_classes_notify(move |field| {
                if field.has_css_class("error") {
                    error_bar.stack.add_css_class("error");
                } else {
                    error_bar.stack.remove_css_class("error");
                }
            });
            if bar.field.has_css_class("error") {
                bar.stack.add_css_class("error");
            }
        }
        bar.rebuild();
        bar
    }

    pub(super) fn widget(&self) -> gtk::Widget {
        self.stack.clone().upcast()
    }

    pub(super) fn is_editing(&self) -> bool {
        self.stack
            .visible_child_name()
            .is_some_and(|name| name.as_str() == EDIT_CHILD)
    }

    pub(super) fn begin_edit(&self) {
        self.edit_start_text.replace(self.field.text().to_string());
        self.stack.set_visible_child_name(EDIT_CHILD);
        self.field.grab_focus();
        self.field.select_region(0, -1);
    }

    pub(super) fn cancel_edit(&self) {
        self.field.set_text(&self.edit_start_text.borrow().clone());
        self.show_browse();
        self.focus_browse();
    }

    pub(super) fn show_browse(&self) {
        self.stack.set_visible_child_name(BROWSE_CHILD);
    }

    pub(super) fn select_directory(&self, path: &Path) {
        self.navigate(path);
        self.focus_browse();
    }

    fn home_target(&self) -> Option<PathBuf> {
        if self.root_limit.is_some() {
            self.canonical_root.clone()
        } else {
            Some(glib::home_dir())
        }
    }

    fn parent_target(&self) -> Option<PathBuf> {
        parent_destination(
            &self.field.text(),
            &self.base,
            self.root_limit.as_deref(),
            self.canonical_root.as_deref(),
            &glib::home_dir(),
        )
    }

    // Typed text that is not yet a folder falls back to the last folder shown.
    fn current_directory(&self) -> PathBuf {
        let text = self.field.text();
        if looks_like_path(&text) {
            let resolved = resolve_destination_path(&text, &self.base, &glib::home_dir());
            if resolved.is_dir() {
                return resolved;
            }
        }
        self.last_directory.borrow().clone()
    }

    fn confined(&self, path: &Path) -> Option<PathBuf> {
        match &self.root_limit {
            Some(_) => canonical_directory_within(self.canonical_root.as_deref()?, path),
            None => Some(path.to_path_buf()),
        }
    }

    pub(super) fn navigate(&self, path: &Path) {
        let Some(path) = self.confined(path) else {
            return;
        };
        let from = self.current_directory();
        self.history.borrow_mut().visit(&from, &path);
        self.show_directory(&path);
    }

    fn show_directory(&self, path: &Path) {
        self.last_directory.replace(path.to_path_buf());
        set_destination_entry(&self.field, path);
        self.show_browse();
        let focus_kept = self
            .stack
            .root()
            .and_then(|root| root.focus())
            .is_some_and(|focus| {
                focus.is_mapped()
                    && focus.is_sensitive()
                    && focus != self.field
                    && !focus.is_ancestor(&self.field)
            });
        if !focus_kept {
            self.focus_browse();
        }
    }

    pub(super) fn go_back(&self) {
        let from = self.current_directory();
        let target = self.history.borrow_mut().go_back(&from);
        if let Some(target) = target {
            self.show_directory(&target);
        }
    }

    pub(super) fn go_forward(&self) {
        let from = self.current_directory();
        let target = self.history.borrow_mut().go_forward(&from);
        if let Some(target) = target {
            self.show_directory(&target);
        }
    }

    pub(super) fn go_up(&self) {
        if let Some(parent) = self.parent_target() {
            self.navigate(&parent);
        }
    }

    pub(super) fn go_home(&self) {
        if let Some(home) = self.home_target() {
            self.navigate(&home);
        }
    }

    fn focus_first_suggestion(&self) -> bool {
        let Some(suggestions) = self
            .navigation
            .borrow()
            .as_ref()
            .and_then(|navigation| navigation.suggestions.upgrade())
        else {
            return false;
        };
        let mut child = suggestions.first_child();
        while let Some(widget) = child {
            if widget.is::<gtk::Button>() {
                let moved = widget.grab_focus();
                if moved && let Some(window) = widget.root().and_downcast::<gtk::Window>() {
                    window.set_focus_visible(true);
                }
                return moved;
            }
            child = widget.next_sibling();
        }
        false
    }

    fn sync_navigation(&self) {
        let navigation = self.navigation.borrow();
        let Some(navigation) = navigation.as_ref() else {
            return;
        };
        let history = self.history.borrow();
        navigation.back.set_sensitive(!history.back.is_empty());
        navigation
            .forward
            .set_sensitive(!history.forward.is_empty());
        navigation.up.set_sensitive(self.parent_target().is_some());
        let text = self.field.text();
        let current = looks_like_path(&text)
            .then(|| resolve_destination_path(&text, &self.base, &glib::home_dir()));
        for (path, row) in &navigation.places {
            if current.as_deref() == Some(path.as_path()) {
                row.add_css_class("active");
            } else {
                row.remove_css_class("active");
            }
        }
    }

    pub(super) fn focus_browse(&self) {
        if let Some(button) = self.last_navigable.borrow().as_ref() {
            button.grab_focus();
        }
    }

    pub(super) fn handle_key(
        &self,
        key: gtk::gdk::Key,
        modifiers: gtk::gdk::ModifierType,
    ) -> glib::Propagation {
        if !self.is_editing() {
            return glib::Propagation::Proceed;
        }
        let unmodified = !modifiers.intersects(
            gtk::gdk::ModifierType::CONTROL_MASK
                | gtk::gdk::ModifierType::ALT_MASK
                | gtk::gdk::ModifierType::SHIFT_MASK,
        );
        if key == gtk::gdk::Key::Escape {
            self.cancel_edit();
            glib::Propagation::Stop
        } else if matches!(key, gtk::gdk::Key::Down | gtk::gdk::Key::KP_Down)
            && unmodified
            && self.focus_first_suggestion()
        {
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    }

    fn rebuild(self: &Rc<Self>) {
        while let Some(child) = self.crumbs.first_child() {
            self.crumbs.remove(&child);
        }
        self.last_navigable.borrow_mut().take();
        let home = glib::home_dir();
        let crumbs = destination_crumbs(
            &self.field.text(),
            &self.base,
            &self.search_root,
            self.root_limit.as_deref(),
            self.canonical_root.as_deref(),
            self.root_label.as_deref(),
            &home,
        );
        let starts_at_fs_root = crumbs
            .first()
            .is_some_and(|crumb| crumb.target.as_path() == Path::new("/"));
        for (index, crumb) in crumbs.iter().enumerate() {
            if index > 0 && !(starts_at_fs_root && index == 1) {
                let separator = gtk::Label::new(Some("/"));
                separator.add_css_class("breadcrumb-separator");
                self.crumbs.append(&separator);
            }
            match crumb.kind {
                DestinationCrumbKind::Current => {
                    let current = gtk::Box::new(gtk::Orientation::Horizontal, 2);
                    current.add_css_class("current-breadcrumb");
                    let view = gtk::Button::with_label(&crumb.label);
                    if let Some(label) = view.child().and_downcast::<gtk::Label>() {
                        ellipsize_destination_crumb(&label);
                    }
                    view.add_css_class("breadcrumb");
                    view.add_css_class("current");
                    crate::ui::accessibility::set_description(
                        &view,
                        Some(&crumb.target.to_string_lossy()),
                    );
                    view.set_has_frame(false);
                    view.set_cursor_from_name(Some("pointer"));
                    let editor = self.clone();
                    view.connect_clicked(move |_| editor.begin_edit());
                    current.append(&view);
                    self.crumbs.append(&current);
                }
                DestinationCrumbKind::Ancestor | DestinationCrumbKind::Scope => {
                    let button = gtk::Button::with_label(&crumb.label);
                    if let Some(label) = button.child().and_downcast::<gtk::Label>() {
                        ellipsize_destination_crumb(&label);
                    }
                    button.add_css_class("breadcrumb");
                    if crumb.target.as_path() == Path::new("/") {
                        button.add_css_class("breadcrumb-root");
                    }
                    if crumb.kind == DestinationCrumbKind::Scope {
                        crate::ui::accessibility::set_description(
                            &button,
                            Some(&format!("Search scope: {} — click to browse", crumb.label)),
                        );
                    } else {
                        crate::ui::accessibility::set_description(
                            &button,
                            Some(&crumb.target.to_string_lossy()),
                        );
                    }
                    button.set_has_frame(false);
                    button.set_cursor_from_name(Some("pointer"));
                    let target = crumb.target.clone();
                    let bar = Rc::downgrade(self);
                    button.connect_clicked(move |_| {
                        if let Some(bar) = bar.upgrade() {
                            bar.navigate(&target);
                        }
                    });
                    self.crumbs.append(&button);
                    self.last_navigable.borrow_mut().replace(button);
                }
            }
        }
        if let Some(last) = self.crumbs.last_child() {
            let last = last.downgrade();
            self.crumb_scroll.add_tick_callback(move |scroller, _| {
                let Some(last) = last.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                // The adjustment's upper bound is stale until the new crumbs are allocated.
                if last.width() <= 0 {
                    return glib::ControlFlow::Continue;
                }
                let adjustment = scroller.hadjustment();
                adjustment.set_value(adjustment.upper() - adjustment.page_size());
                glib::ControlFlow::Break
            });
        }
        self.sync_navigation();
    }
}

pub(super) struct DestinationBrowserOptions {
    pub(super) base: PathBuf,
    pub(super) search_root: PathBuf,
    pub(super) root_limit: Option<PathBuf>,
    pub(super) root_label: Option<String>,
    pub(super) show_hidden: bool,
    pub(super) places: Vec<PlaceShortcut>,
}

/// The folder picker shared by Copy to, Move to, Send to, and Extract to.
pub(super) struct DestinationBrowser {
    pub(super) field: gtk::Entry,
    pub(super) bar: Rc<DestinationLocationBar>,
    pub(super) error: gtk::Label,
    root: gtk::Box,
}

impl DestinationBrowser {
    pub(super) fn new(
        options: DestinationBrowserOptions,
        on_changed: impl Fn(&gtk::Entry) + 'static,
    ) -> Self {
        let DestinationBrowserOptions {
            base,
            search_root,
            root_limit,
            root_label,
            show_hidden,
            places,
        } = options;
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.append(&form_label("Destination folder"));

        let field = form_entry();
        field.set_hexpand(true);
        field.set_placeholder_text(Some("Search for a folder…"));
        field.set_text(&folder_input_path(&base));
        field.set_position(-1);
        let bar = DestinationLocationBar::wrap(
            field.clone(),
            base.clone(),
            search_root.clone(),
            root_limit.clone(),
            root_label.clone(),
        );

        let back = navigation_button(crate::assets::icons::ARROW_LEFT, "Back (Alt+Left)");
        let forward = navigation_button(crate::assets::icons::ARROW_RIGHT, "Forward (Alt+Right)");
        let up = navigation_button(crate::assets::icons::ARROW_UP, "Parent folder (Alt+Up)");
        back.add_css_class("destination-back");
        forward.add_css_class("destination-forward");
        up.add_css_class("destination-up");
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        buttons.add_css_class("list-navigation");
        buttons.set_valign(gtk::Align::Center);
        for (button, action) in [
            (
                &back,
                DestinationLocationBar::go_back as fn(&DestinationLocationBar),
            ),
            (&forward, DestinationLocationBar::go_forward),
            (&up, DestinationLocationBar::go_up),
        ] {
            let weak_bar = Rc::downgrade(&bar);
            button.connect_clicked(move |_| {
                if let Some(bar) = weak_bar.upgrade() {
                    action(&bar);
                }
            });
            buttons.append(button);
        }
        let navigation_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        navigation_row.append(&buttons);
        let location = bar.widget();
        location.set_hexpand(true);
        navigation_row.append(&location);
        root.append(&navigation_row);

        let error = gtk::Label::new(None);
        error.add_css_class("form-message");
        error.add_css_class("error");
        error.set_wrap(true);
        error.set_xalign(0.0);
        error.set_visible(false);

        let browse_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let mut place_rows = Vec::new();
        if !places.is_empty() {
            let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
            column.add_css_class("destination-places");
            let mut group = PlaceGroup::Standard;
            for place in places {
                if place.group != group {
                    group = place.group;
                    let heading = gtk::Label::new(Some(match group {
                        PlaceGroup::Standard => "PLACES",
                        PlaceGroup::Pinned => "PINNED",
                        PlaceGroup::Device => "DEVICES",
                    }));
                    heading.add_css_class("sidebar-heading");
                    heading.set_xalign(0.0);
                    column.append(&heading);
                }
                let row = sidebar_button(place.icon, &place.name);
                crate::ui::accessibility::set_description(
                    &row,
                    Some(&place.path.to_string_lossy()),
                );
                let weak_bar = Rc::downgrade(&bar);
                let place_error = error.clone();
                let path = place.path.clone();
                row.connect_clicked(move |_| {
                    let Some(bar) = weak_bar.upgrade() else {
                        return;
                    };
                    if path.is_dir() {
                        bar.navigate(&path);
                    } else {
                        place_error.remove_css_class("warning");
                        place_error.add_css_class("error");
                        place_error.set_text("That location is no longer available.");
                        place_error.set_visible(true);
                    }
                });
                column.append(&row);
                place_rows.push((place.path, row));
            }
            let place_scroll = gtk::ScrolledWindow::builder()
                .child(&column)
                .hscrollbar_policy(gtk::PolicyType::Never)
                .vscrollbar_policy(gtk::PolicyType::Automatic)
                // Row labels expand; keep that from widening the column.
                .hexpand(false)
                .build();
            place_scroll.add_css_class("destination-place-scroll");
            browse_row.append(&place_scroll);
        }

        let suggestions = gtk::Box::new(gtk::Orientation::Vertical, 2);
        suggestions.add_css_class("transfer-suggestions");
        let suggestion_scroll = gtk::ScrolledWindow::builder()
            .child(&suggestions)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .min_content_height(150)
            .max_content_height(220)
            .propagate_natural_height(true)
            .hexpand(true)
            .build();
        suggestion_scroll.add_css_class("transfer-suggestion-scroll");
        browse_row.append(&suggestion_scroll);
        root.append(&browse_row);
        root.append(&error);

        bar.navigation.replace(Some(DestinationNavigation {
            back,
            forward,
            up,
            places: place_rows,
            suggestions: suggestions.downgrade(),
        }));
        bar.sync_navigation();

        let select_bar = Rc::downgrade(&bar);
        let changed_error = error.clone();
        setup_transfer_search(
            &field,
            &suggestions,
            &Rc::new(Cell::new(0_u64)),
            TransferSearchScope {
                base,
                search_root,
                root_limit,
                show_hidden,
            },
            Rc::new(move |path: &Path| {
                if let Some(bar) = select_bar.upgrade() {
                    bar.select_directory(path);
                }
            }),
            move |field| {
                field.remove_css_class("error");
                changed_error.set_visible(false);
                changed_error.remove_css_class("warning");
                changed_error.add_css_class("error");
                on_changed(field);
            },
        );

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let keys_bar = Rc::downgrade(&bar);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let Some(bar) = keys_bar.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if !modifiers.contains(gtk::gdk::ModifierType::ALT_MASK)
                || modifiers.intersects(
                    gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK,
                )
            {
                return glib::Propagation::Proceed;
            }
            match key {
                gtk::gdk::Key::Left | gtk::gdk::Key::KP_Left => bar.go_back(),
                gtk::gdk::Key::Right | gtk::gdk::Key::KP_Right => bar.go_forward(),
                gtk::gdk::Key::Up | gtk::gdk::Key::KP_Up => bar.go_up(),
                gtk::gdk::Key::Home | gtk::gdk::Key::KP_Home => bar.go_home(),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        root.add_controller(keys);

        Self {
            field,
            bar,
            error,
            root,
        }
    }

    pub(super) fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    /// Lists the starting folder; call once the picker is in its dialog.
    pub(super) fn activate(&self) {
        self.field.emit_by_name::<()>("changed", &[]);
        self.bar.focus_browse();
    }
}

#[cfg(test)]
mod tests;
