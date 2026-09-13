// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    fs,
    path::{Path, PathBuf},
    rc::Rc,
};

use gtk::{gdk, glib, prelude::*};

use crate::app::Browser;

const MAX_COMPLETION_CANDIDATES: usize = 50;
const MAX_SCANNED_ENTRIES: usize = 10_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CompletionCandidate {
    pub(crate) display_name: String,
    pub(crate) replacement: String,
    pub(crate) parent_hint: String,
    pub(crate) match_len: usize,
}

pub(crate) struct PathCompletion {
    popover: gtk::Popover,
    scroll: gtk::ScrolledWindow,
    list: gtk::ListBox,
    candidates: Rc<RefCell<Vec<CompletionCandidate>>>,
    selected_index: Rc<Cell<Option<usize>>>,
    pending_reveal: Cell<bool>,
    suppress_refresh: Cell<bool>,
    is_active: Box<dyn Fn() -> bool>,
}

impl Drop for PathCompletion {
    fn drop(&mut self) {
        if self.popover.parent().is_some() {
            self.popover.unparent();
        }
    }
}

fn format_highlighted_markup(display_name: &str, match_len: usize) -> String {
    if match_len == 0 {
        return glib::markup_escape_text(display_name).to_string();
    }
    let mut byte_split = 0;
    for (char_count, (i, c)) in display_name.char_indices().enumerate() {
        if char_count == match_len {
            byte_split = i;
            break;
        }
        byte_split = i + c.len_utf8();
    }
    let (matched, remainder) = display_name.split_at(byte_split);
    format!(
        "<b>{}</b>{}",
        glib::markup_escape_text(matched),
        glib::markup_escape_text(remainder)
    )
}

fn compact_hint(path: &Path, home: &Path) -> String {
    if path == home {
        return "~".to_owned();
    }
    if let Ok(suffix) = path.strip_prefix(home) {
        return format!("~/{}", suffix.to_string_lossy());
    }
    path.to_string_lossy().into_owned()
}

fn directory_prefix(path: &Path) -> String {
    let mut prefix = path.to_string_lossy().into_owned();
    if !prefix.ends_with(std::path::MAIN_SEPARATOR) {
        prefix.push(std::path::MAIN_SEPARATOR);
    }
    prefix
}

impl PathCompletion {
    pub(crate) fn attach(
        entry: &gtk::Entry,
        browser: Rc<Browser>,
        is_active: impl Fn() -> bool + 'static,
        on_activate: impl Fn() + 'static,
    ) -> Rc<Self> {
        let content_box = gtk::Box::new(gtk::Orientation::Vertical, 0);

        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .hexpand(true)
            .can_focus(false)
            .focusable(false)
            .build();
        list.add_css_class("path-completion-list");

        let scroll = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .min_content_height(36)
            .max_content_height(280)
            .propagate_natural_height(true)
            .propagate_natural_width(false)
            .hexpand(true)
            .can_focus(false)
            .focusable(false)
            .build();
        scroll.add_css_class("path-completion-scroll");
        content_box.append(&scroll);

        let shortcuts = gtk::Label::new(None);
        shortcuts.set_markup("<b>Tab</b> Complete   <b>↑↓</b> Choose   <b>↵</b> Open");
        shortcuts.set_xalign(0.0);
        shortcuts.add_css_class("path-completion-shortcuts");
        content_box.append(&shortcuts);

        let popover = gtk::Popover::builder()
            .has_arrow(false)
            .autohide(false)
            .position(gtk::PositionType::Bottom)
            .halign(gtk::Align::Fill)
            .can_focus(false)
            .focusable(false)
            .child(&content_box)
            .build();
        popover.add_css_class("path-completion-popover");
        popover.set_parent(entry);

        let popover_for_destroy = popover.clone();
        entry.connect_destroy(move |_| {
            if popover_for_destroy.parent().is_some() {
                popover_for_destroy.unparent();
            }
        });

        let candidates = Rc::new(RefCell::new(Vec::new()));
        let selected_index = Rc::new(Cell::new(None));
        let on_activate: Rc<dyn Fn()> = Rc::new(on_activate);

        let completion = Rc::new(Self {
            popover: popover.clone(),
            scroll: scroll.clone(),
            list: list.clone(),
            candidates: candidates.clone(),
            selected_index: selected_index.clone(),
            pending_reveal: Cell::new(false),
            suppress_refresh: Cell::new(false),
            is_active: Box::new(is_active),
        });

        let weak_completion = Rc::downgrade(&completion);
        let row_entry = entry.downgrade();
        let row_activate = on_activate.clone();
        list.connect_row_activated(move |_, row| {
            let Some(completion) = weak_completion.upgrade() else {
                return;
            };
            let Some(entry) = row_entry.upgrade() else {
                return;
            };
            let index = row.index() as usize;
            let candidate = completion.candidates.borrow().get(index).cloned();
            if let Some(candidate) = candidate {
                completion.activate_candidate(&entry, &candidate, row_activate.as_ref());
            }
        });

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak_completion = Rc::downgrade(&completion);
        let key_entry = entry.downgrade();
        let key_browser = browser.clone();
        let key_activate = on_activate;
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let Some(completion) = weak_completion.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let Some(entry) = key_entry.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if modifiers.intersects(
                gdk::ModifierType::CONTROL_MASK
                    | gdk::ModifierType::ALT_MASK
                    | gdk::ModifierType::SUPER_MASK,
            ) {
                return glib::Propagation::Proceed;
            }
            match key {
                gdk::Key::Tab | gdk::Key::ISO_Left_Tab => {
                    completion.handle_tab(&entry, &key_browser);
                    glib::Propagation::Stop
                }
                gdk::Key::Down => {
                    if completion.popover.is_visible() {
                        completion.move_selection(1);
                        glib::Propagation::Stop
                    } else {
                        completion.refresh(&entry, &key_browser);
                        glib::Propagation::Stop
                    }
                }
                gdk::Key::Up => {
                    if completion.popover.is_visible() {
                        completion.move_selection(-1);
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
                gdk::Key::Page_Down => {
                    if completion.popover.is_visible() {
                        completion.move_selection(5);
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
                gdk::Key::Page_Up => {
                    if completion.popover.is_visible() {
                        completion.move_selection(-5);
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
                gdk::Key::Escape => {
                    if completion.popover.is_visible() {
                        completion.dismiss();
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
                gdk::Key::Return | gdk::Key::KP_Enter => {
                    if completion.popover.is_visible() {
                        if let Some(index) = completion.selected_index.get() {
                            let candidate = completion.candidates.borrow().get(index).cloned();
                            if let Some(candidate) = candidate {
                                completion.activate_candidate(
                                    &entry,
                                    &candidate,
                                    key_activate.as_ref(),
                                );
                                return glib::Propagation::Stop;
                            }
                        }
                        completion.dismiss();
                    }
                    glib::Propagation::Proceed
                }
                _ => glib::Propagation::Proceed,
            }
        });
        entry.add_controller(keys);

        let weak_completion = Rc::downgrade(&completion);
        let change_browser = browser;
        entry.connect_changed(move |entry| {
            if let Some(completion) = weak_completion.upgrade()
                && !completion.suppress_refresh.get()
            {
                completion.refresh(entry, &change_browser);
            }
        });

        completion
    }

    pub(crate) fn refresh(self: &Rc<Self>, entry: &gtk::Entry, browser: &Browser) {
        if !(self.is_active)() || entry.root().is_none() {
            self.candidates.borrow_mut().clear();
            self.selected_index.set(None);
            self.dismiss();
            return;
        }

        let text = entry.text().to_string();
        let current_dir = browser
            .active_location()
            .and_then(|location| location.native_path().map(Path::to_path_buf));
        let show_hidden = browser.preferences().show_hidden;
        let candidates = suggest_completions(
            &text,
            current_dir.as_deref(),
            &glib::home_dir(),
            show_hidden,
        );

        if candidates.is_empty() {
            self.candidates.borrow_mut().clear();
            self.selected_index.set(None);
            self.dismiss();
            return;
        }

        self.candidates.replace(candidates.clone());
        self.selected_index.set(None);
        self.render_candidates(&candidates);
        let already_pending = self.pending_reveal.replace(true);
        self.present_for_entry(entry);
        if self.pending_reveal.get() && !already_pending {
            let weak_completion = Rc::downgrade(self);
            let weak_entry = entry.downgrade();
            entry.add_tick_callback(move |_, _| {
                let (Some(completion), Some(entry)) =
                    (weak_completion.upgrade(), weak_entry.upgrade())
                else {
                    return glib::ControlFlow::Break;
                };
                if !completion.pending_reveal.get() {
                    return glib::ControlFlow::Break;
                }
                completion.present_for_entry(&entry);
                if completion.pending_reveal.get() {
                    glib::ControlFlow::Continue
                } else {
                    glib::ControlFlow::Break
                }
            });
        }
    }

    fn present_for_entry(&self, entry: &gtk::Entry) {
        if !(self.is_active)() || self.candidates.borrow().is_empty() {
            self.dismiss();
            return;
        }
        let width = entry.width();
        if width <= 0 {
            return;
        }
        self.popover.set_size_request(width, -1);
        self.popover.set_pointing_to(Some(&gdk::Rectangle::new(
            0,
            0,
            width,
            entry.height().max(32),
        )));
        self.popover.set_offset(0, 6);
        self.pending_reveal.set(false);
        if !self.popover.is_visible() {
            self.popover.popup();
        }
    }

    pub(crate) fn dismiss(&self) {
        self.pending_reveal.set(false);
        self.popover.popdown();
    }

    fn activate_candidate(
        &self,
        entry: &gtk::Entry,
        candidate: &CompletionCandidate,
        on_activate: &dyn Fn(),
    ) {
        self.dismiss();
        self.suppress_refresh.set(true);
        entry.set_text(&candidate.replacement);
        entry.set_position(-1);
        self.suppress_refresh.set(false);
        on_activate();
    }

    fn render_candidates(&self, candidates: &[CompletionCandidate]) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        for candidate in candidates {
            let list_row = gtk::ListBoxRow::builder()
                .focusable(false)
                .selectable(true)
                .activatable(true)
                .tooltip_text(&candidate.replacement)
                .build();
            list_row.add_css_class("path-completion-row");

            let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            content.add_css_class("path-completion-row-content");

            let icon = crate::assets::primary_icon(crate::assets::icons::FOLDER, 16);
            icon.set_valign(gtk::Align::Center);
            content.append(&icon);

            let label = gtk::Label::new(None);
            label.set_markup(&format_highlighted_markup(
                &candidate.display_name,
                candidate.match_len,
            ));
            label.set_xalign(0.0);
            label.set_hexpand(true);
            label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            content.append(&label);

            if !candidate.parent_hint.is_empty() {
                let parent_label = gtk::Label::new(Some(&candidate.parent_hint));
                parent_label.add_css_class("path-completion-parent");
                parent_label.set_xalign(1.0);
                parent_label.set_ellipsize(gtk::pango::EllipsizeMode::Start);
                parent_label.set_max_width_chars(18);
                content.append(&parent_label);
            }

            list_row.set_child(Some(&content));
            self.list.append(&list_row);
        }
    }

    fn move_selection(&self, delta: i32) {
        let count = self.candidates.borrow().len();
        if count == 0 {
            return;
        }
        let next = match self.selected_index.get() {
            Some(current) => {
                if delta > 0 {
                    (current + delta as usize) % count
                } else {
                    (current + count - (-delta as usize % count)) % count
                }
            }
            None => {
                if delta > 0 {
                    0
                } else {
                    count.saturating_sub(1)
                }
            }
        };
        self.selected_index.set(Some(next));
        if let Some(row) = self.list.row_at_index(next as i32) {
            self.list.select_row(Some(&row));
            let vadjustment = self.scroll.vadjustment();
            if let Some(bounds) = row.compute_bounds(&self.list) {
                let row_top = bounds.top_left().y() as f64;
                let row_bottom = bounds.bottom_left().y() as f64;
                let current_val = vadjustment.value();
                let page_size = vadjustment.page_size();
                if row_top < current_val {
                    vadjustment.set_value(row_top);
                } else if row_bottom > current_val + page_size {
                    vadjustment.set_value(row_bottom - page_size);
                }
            } else if next == 0 {
                vadjustment.set_value(vadjustment.lower());
            }
        }
    }

    fn handle_tab(self: &Rc<Self>, entry: &gtk::Entry, browser: &Browser) {
        if !self.popover.is_visible() {
            self.refresh(entry, browser);
        }
        let candidates = self.candidates.borrow().clone();
        if candidates.is_empty() {
            return;
        }

        if let Some(index) = self.selected_index.get()
            && let Some(candidate) = candidates.get(index)
        {
            entry.set_text(&candidate.replacement);
            entry.set_position(-1);
            return;
        }

        if candidates.len() == 1 {
            let candidate = &candidates[0];
            entry.set_text(&candidate.replacement);
            entry.set_position(-1);
            return;
        }

        let replacements: Vec<_> = candidates.iter().map(|c| c.replacement.clone()).collect();
        if let Some(common) = longest_common_prefix(&replacements) {
            let current_text = entry.text().to_string();
            if common.len() > current_text.len() {
                entry.set_text(&common);
                entry.set_position(-1);
                return;
            }
        }

        self.move_selection(1);
    }
}

pub(crate) fn suggest_completions(
    input: &str,
    current_dir: Option<&Path>,
    home: &Path,
    show_hidden_pref: bool,
) -> Vec<CompletionCandidate> {
    let input = input.trim();
    if input.is_empty() {
        if let Some(current) = current_dir {
            let hint = compact_hint(current, home);
            return list_directory_candidates(current, "", "", &hint, 0, show_hidden_pref);
        }
        return Vec::new();
    }

    if input == "~" {
        return vec![CompletionCandidate {
            display_name: "~/".to_owned(),
            replacement: "~/".to_owned(),
            parent_hint: "home".to_owned(),
            match_len: 1,
        }];
    }

    if let Some(relative) = input.strip_prefix("~/") {
        let (base_dir, leaf_prefix, prepend) = if relative.ends_with('/') {
            (
                home.join(relative.trim_start_matches('/')),
                "",
                format!("~/{relative}"),
            )
        } else if let Some((parent_rel, leaf)) = relative.rsplit_once('/') {
            (
                home.join(parent_rel.trim_start_matches('/')),
                leaf,
                format!("~/{parent_rel}/"),
            )
        } else {
            (home.to_path_buf(), relative, "~/".to_owned())
        };
        let hint = compact_hint(&base_dir, home);
        let candidates = list_directory_candidates(
            &base_dir,
            leaf_prefix,
            &prepend,
            &hint,
            leaf_prefix.chars().count(),
            show_hidden_pref,
        );
        let rel_trimmed = relative.trim_start_matches('/');
        let full_path = home.join(rel_trimmed);
        if candidates.len() <= 1 && full_path.is_dir() && !rel_trimmed.is_empty() {
            let child_hint = compact_hint(&full_path, home);
            let child_prepend = format!("~/{}/", rel_trimmed.trim_end_matches('/'));
            let children = list_directory_candidates(
                &full_path,
                "",
                &child_prepend,
                &child_hint,
                0,
                show_hidden_pref,
            );
            if !children.is_empty() {
                return children;
            }
        }
        return candidates;
    }

    if input.starts_with('~') {
        return Vec::new();
    }

    if let Some(stripped) = input.strip_prefix('/') {
        let (base_dir, leaf_prefix, prepend) = if input == "/" {
            (PathBuf::from("/"), "", "/".to_owned())
        } else if input.ends_with('/') {
            (PathBuf::from(input), "", input.to_owned())
        } else if let Some((parent_part, leaf)) = input.rsplit_once('/') {
            let base = if parent_part.is_empty() {
                PathBuf::from("/")
            } else {
                PathBuf::from(parent_part)
            };
            let prepend = if parent_part.is_empty() {
                "/".to_owned()
            } else {
                format!("{parent_part}/")
            };
            (base, leaf, prepend)
        } else {
            (PathBuf::from("/"), stripped, "/".to_owned())
        };
        let hint = compact_hint(&base_dir, home);
        let candidates = list_directory_candidates(
            &base_dir,
            leaf_prefix,
            &prepend,
            &hint,
            leaf_prefix.chars().count(),
            show_hidden_pref,
        );
        let input_path = PathBuf::from(input);
        if candidates.len() <= 1 && input_path.is_dir() && input != "/" {
            let child_hint = compact_hint(&input_path, home);
            let child_prepend = format!("{}/", input.trim_end_matches('/'));
            let children = list_directory_candidates(
                &input_path,
                "",
                &child_prepend,
                &child_hint,
                0,
                show_hidden_pref,
            );
            if !children.is_empty() {
                return children;
            }
        }
        return candidates;
    }

    if let Some(current) = current_dir {
        let (base_dir, leaf_prefix) = if input.ends_with('/') {
            (current.join(input), "")
        } else if let Some((parent_part, leaf)) = input.rsplit_once('/') {
            (current.join(parent_part), leaf)
        } else {
            (current.to_path_buf(), input)
        };
        let prepend = directory_prefix(&base_dir);
        let hint = compact_hint(&base_dir, home);
        let candidates = list_directory_candidates(
            &base_dir,
            leaf_prefix,
            &prepend,
            &hint,
            leaf_prefix.chars().count(),
            show_hidden_pref,
        );
        let input_path = current.join(input);
        if candidates.len() <= 1 && input_path.is_dir() && !input.ends_with('/') {
            let child_hint = compact_hint(&input_path, home);
            let child_prepend = directory_prefix(&input_path);
            let children = list_directory_candidates(
                &input_path,
                "",
                &child_prepend,
                &child_hint,
                0,
                show_hidden_pref,
            );
            if !children.is_empty() {
                return children;
            }
        }
        return candidates;
    }

    Vec::new()
}

fn list_directory_candidates(
    base_dir: &Path,
    leaf_prefix: &str,
    prepend: &str,
    parent_hint: &str,
    match_len: usize,
    show_hidden_pref: bool,
) -> Vec<CompletionCandidate> {
    let Ok(entries) = fs::read_dir(base_dir) else {
        return Vec::new();
    };

    let leaf_lower = leaf_prefix.to_lowercase();
    let include_hidden = leaf_prefix.starts_with('.') || show_hidden_pref;

    let mut dirs = Vec::new();

    for entry in entries.flatten().take(MAX_SCANNED_ENTRIES) {
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_hidden = name.starts_with('.');
        if !include_hidden && is_hidden {
            continue;
        }
        if !name.to_lowercase().starts_with(&leaf_lower) {
            continue;
        }

        let is_dir = entry
            .file_type()
            .is_ok_and(|ft| ft.is_dir() || (ft.is_symlink() && entry.path().is_dir()));

        if !is_dir {
            continue;
        }
        dirs.push(CompletionCandidate {
            display_name: format!("{name}/"),
            replacement: format!("{prepend}{name}/"),
            parent_hint: parent_hint.to_owned(),
            match_len,
        });
    }

    dirs.sort_by(|a, b| {
        a.display_name
            .to_lowercase()
            .cmp(&b.display_name.to_lowercase())
    });
    dirs.truncate(MAX_COMPLETION_CANDIDATES);
    dirs
}

pub(crate) fn longest_common_prefix(strings: &[String]) -> Option<String> {
    if strings.is_empty() {
        return None;
    }
    let first = &strings[0];
    let mut len = first.len();
    for string in &strings[1..] {
        len = len.min(string.len());
        while len > 0 && (!first.is_char_boundary(len) || !string.starts_with(&first[..len])) {
            len -= 1;
        }
        if len == 0 {
            return None;
        }
    }
    Some(first[..len].to_owned())
}

#[cfg(test)]
mod tests;
