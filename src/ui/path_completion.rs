// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    fs,
    path::{Path, PathBuf},
    rc::Rc,
};

use gtk::{gdk, glib, prelude::*};

use crate::app::Browser;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CompletionCandidate {
    pub(crate) display_name: String,
    pub(crate) replacement: String,
    pub(crate) parent_hint: String,
    pub(crate) match_len: usize,
    pub(crate) is_dir: bool,
}

pub(crate) struct PathCompletion {
    popover: gtk::Popover,
    scroll: gtk::ScrolledWindow,
    list: gtk::ListBox,
    candidates: Rc<RefCell<Vec<CompletionCandidate>>>,
    selected_index: Rc<Cell<Option<usize>>>,
}

impl Drop for PathCompletion {
    fn drop(&mut self) {
        if self.popover.parent().is_some() {
            self.popover.unparent();
        }
    }
}

fn candidate_icon(name: &str, is_dir: bool) -> &'static str {
    if is_dir {
        return crate::assets::icons::FOLDER;
    }
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("sh" | "bash" | "zsh" | "fish") => crate::assets::icons::TERMINAL,
        Some(
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "avif" | "tif" | "tiff"
            | "dng" | "raw",
        ) => crate::assets::icons::PICTURES,
        Some("mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v") => crate::assets::icons::VIDEOS,
        Some("zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" | "zst") => {
            crate::assets::icons::FILE_ARCHIVE
        }
        Some(
            "rs" | "c" | "h" | "cpp" | "go" | "py" | "rb" | "java" | "js" | "jsx" | "ts" | "tsx"
            | "lua" | "php" | "html" | "css" | "scss" | "json" | "toml" | "yaml" | "yml",
        ) => crate::assets::icons::FILE_CODE,
        _ => crate::assets::icons::DOCUMENTS,
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

impl PathCompletion {
    pub(crate) fn attach(
        entry: &gtk::Entry,
        browser: Rc<Browser>,
        on_activate: impl Fn() + 'static,
    ) -> Rc<Self> {
        let content_box = gtk::Box::new(gtk::Orientation::Vertical, 4);

        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .can_focus(false)
            .focusable(false)
            .build();
        list.add_css_class("path-completion-list");

        let scroll = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .min_content_height(36)
            .max_content_height(260)
            .min_content_width(460)
            .propagate_natural_height(true)
            .propagate_natural_width(true)
            .can_focus(false)
            .focusable(false)
            .build();
        scroll.add_css_class("path-completion-scroll");
        content_box.append(&scroll);

        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        footer.add_css_class("path-completion-footer");
        let hints = gtk::Label::new(None);
        hints.set_markup("<span alpha='65%'><b>Tab</b> complete</span>  •  <span alpha='65%'><b>↑↓</b> select</span>  •  <span alpha='65%'><b>↵</b> navigate</span>  •  <span alpha='65%'><b>Esc</b> close</span>");
        hints.set_xalign(0.0);
        hints.set_hexpand(true);
        footer.append(&hints);
        content_box.append(&footer);

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
        let on_activate = Rc::new(on_activate);

        let completion = Rc::new(Self {
            popover: popover.clone(),
            scroll: scroll.clone(),
            list: list.clone(),
            candidates: candidates.clone(),
            selected_index: selected_index.clone(),
        });

        let weak_completion = Rc::downgrade(&completion);
        let row_entry = entry.downgrade();
        let row_browser = browser.clone();
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
                entry.set_text(&candidate.replacement);
                entry.set_position(-1);
                entry.grab_focus();
                if candidate.is_dir {
                    completion.refresh(&entry, &row_browser);
                } else {
                    completion.dismiss();
                    row_activate();
                }
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
                                entry.set_text(&candidate.replacement);
                                entry.set_position(-1);
                                completion.dismiss();
                                key_activate();
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
            if let Some(completion) = weak_completion.upgrade() {
                completion.refresh(entry, &change_browser);
            }
        });

        completion
    }

    pub(crate) fn refresh(&self, entry: &gtk::Entry, browser: &Browser) {
        if entry.root().is_none() {
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
        let width = entry.width();
        if width > 0 {
            self.scroll.set_min_content_width(width);
            self.popover
                .set_pointing_to(Some(&gdk::Rectangle::new(0, entry.height(), width, 1)));
        }
        if !self.popover.is_visible() {
            self.popover.popup();
        }
    }

    pub(crate) fn dismiss(&self) {
        self.popover.popdown();
    }

    fn render_candidates(&self, candidates: &[CompletionCandidate]) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        for candidate in candidates {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            row.add_css_class("path-completion-row");

            let icon_name = candidate_icon(&candidate.display_name, candidate.is_dir);
            let icon = crate::assets::primary_icon(icon_name, 16);
            icon.set_valign(gtk::Align::Center);
            row.append(&icon);

            let label = gtk::Label::new(None);
            label.set_markup(&format_highlighted_markup(
                &candidate.display_name,
                candidate.match_len,
            ));
            label.set_xalign(0.0);
            label.set_hexpand(true);
            label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            row.append(&label);

            if !candidate.parent_hint.is_empty() {
                let parent_label = gtk::Label::new(Some(&candidate.parent_hint));
                parent_label.add_css_class("path-completion-parent");
                parent_label.set_xalign(1.0);
                parent_label.set_ellipsize(gtk::pango::EllipsizeMode::Start);
                row.append(&parent_label);
            }

            self.list.append(&row);
        }
    }

    fn move_selection(&self, delta: i32) {
        let count = self.candidates.borrow().len();
        if count == 0 {
            return;
        }
        let current = self.selected_index.get();
        let next = match current {
            Some(index) => (index as i32 + delta).rem_euclid(count as i32) as usize,
            None => {
                if delta >= 0 {
                    0
                } else {
                    count.saturating_sub(1)
                }
            }
        };
        self.selected_index.set(Some(next));
        if let Some(row) = self.list.row_at_index(next as i32) {
            self.list.select_row(Some(&row));
        }
    }

    fn handle_tab(&self, entry: &gtk::Entry, browser: &Browser) {
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
            self.refresh(entry, browser);
            return;
        }

        if candidates.len() == 1 {
            let candidate = &candidates[0];
            entry.set_text(&candidate.replacement);
            entry.set_position(-1);
            self.refresh(entry, browser);
            return;
        }

        let replacements: Vec<_> = candidates.iter().map(|c| c.replacement.clone()).collect();
        if let Some(common) = longest_common_prefix(&replacements) {
            let current_text = entry.text().to_string();
            if common.len() > current_text.len() {
                entry.set_text(&common);
                entry.set_position(-1);
                self.refresh(entry, browser);
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
            is_dir: true,
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
        return list_directory_candidates(
            &base_dir,
            leaf_prefix,
            &prepend,
            &hint,
            leaf_prefix.chars().count(),
            show_hidden_pref,
        );
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
        return list_directory_candidates(
            &base_dir,
            leaf_prefix,
            &prepend,
            &hint,
            leaf_prefix.chars().count(),
            show_hidden_pref,
        );
    }

    if let Some(current) = current_dir {
        let (base_dir, leaf_prefix, prepend) = if input.ends_with('/') {
            (current.join(input), "", input.to_owned())
        } else if let Some((parent_part, leaf)) = input.rsplit_once('/') {
            (current.join(parent_part), leaf, format!("{parent_part}/"))
        } else {
            (current.to_path_buf(), input, "".to_owned())
        };
        let hint = compact_hint(&base_dir, home);
        return list_directory_candidates(
            &base_dir,
            leaf_prefix,
            &prepend,
            &hint,
            leaf_prefix.chars().count(),
            show_hidden_pref,
        );
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
    let mut files = Vec::new();

    for entry in entries.flatten() {
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

        if is_dir {
            dirs.push(CompletionCandidate {
                display_name: format!("{name}/"),
                replacement: format!("{prepend}{name}/"),
                parent_hint: parent_hint.to_owned(),
                match_len,
                is_dir: true,
            });
        } else {
            files.push(CompletionCandidate {
                display_name: name.clone(),
                replacement: format!("{prepend}{name}"),
                parent_hint: parent_hint.to_owned(),
                match_len,
                is_dir: false,
            });
        }
    }

    dirs.sort_by(|a, b| {
        a.display_name
            .to_lowercase()
            .cmp(&b.display_name.to_lowercase())
    });
    files.sort_by(|a, b| {
        a.display_name
            .to_lowercase()
            .cmp(&b.display_name.to_lowercase())
    });

    dirs.extend(files);
    dirs.truncate(50);
    dirs
}

pub(crate) fn longest_common_prefix(strings: &[String]) -> Option<String> {
    if strings.is_empty() {
        return None;
    }
    let first = &strings[0];
    let mut len = first.len();
    for s in &strings[1..] {
        len = len.min(s.len());
        while !s.starts_with(&first[..len]) {
            len = len.saturating_sub(1);
            if len == 0 {
                return None;
            }
        }
    }
    Some(first[..len].to_string())
}

#[cfg(test)]
mod tests;
