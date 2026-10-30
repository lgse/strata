// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
};

use gtk::{
    gdk::{self, ModifierType as Modifiers},
    glib,
    prelude::*,
};

use crate::ui::keys::{
    self, Keys,
    bindings::{
        COMPLETE, DOWN, ENTER, ENTER_ANY_SHIFT, ESCAPE_ANY_SHIFT, PAGE_DOWN_KEY, PAGE_UP_KEY, UP,
    },
};
use crate::{adapters::probes, app::Browser};

const MAX_COMPLETION_CANDIDATES: usize = 50;
const MAX_SCANNED_ENTRIES: usize = 10_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CompletionCandidate {
    pub(crate) display_name: String,
    pub(crate) replacement: String,
    pub(crate) parent_hint: String,
    pub(crate) match_len: usize,
}

#[derive(Clone, Copy)]
enum Command {
    Step(i32),
    Down,
    Complete,
    Activate,
}

struct Request {
    generation: u64,
    text: String,
    deferred: Vec<Command>,
    task: glib::JoinHandle<()>,
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
    on_activate: Rc<dyn Fn()>,
    generation: Cell<u64>,
    request: RefCell<Option<Request>>,
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
        crate::ui::shortcut_reference::follow_keys(&shortcuts, |shortcuts| {
            shortcuts.set_markup(&crate::ui::shortcut_reference::legend_markup(&[
                (Keys::One(COMPLETE.shown()), "completion.complete"),
                (
                    Keys::Any(crate::ui::keys::shown![UP.alternate(0), DOWN.alternate(0)]),
                    "Choose",
                ),
                (Keys::One(ENTER.shown()), "Open"),
            ]));
        });
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
            on_activate,
            generation: Cell::new(0),
            request: RefCell::new(None),
        });

        let weak_completion = Rc::downgrade(&completion);
        let row_entry = entry.downgrade();
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
                completion.activate_candidate(&entry, &candidate);
            }
        });

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak_completion = Rc::downgrade(&completion);
        let key_entry = entry.downgrade();
        let key_browser = browser.clone();
        keys::on_press(&keys, move |_, press| {
            const SHIFT: Modifiers = Modifiers::SHIFT_MASK;
            let Some(completion) = weak_completion.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let Some(entry) = key_entry.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let command = if press.is(COMPLETE.ignoring(SHIFT)) {
                Command::Complete
            } else if press.is(DOWN.ignoring(SHIFT)) {
                Command::Down
            } else if press.is(UP.ignoring(SHIFT)) {
                Command::Step(-1)
            } else if press.is(PAGE_DOWN_KEY.ignoring(SHIFT)) {
                Command::Step(5)
            } else if press.is(PAGE_UP_KEY.ignoring(SHIFT)) {
                Command::Step(-5)
            } else if press.is(ENTER_ANY_SHIFT) {
                Command::Activate
            } else if press.is(ESCAPE_ANY_SHIFT) && completion.popover.is_visible() {
                completion.dismiss();
                return glib::Propagation::Stop;
            } else {
                return glib::Propagation::Proceed;
            };
            completion.press(&entry, &key_browser, command, true)
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

    pub(crate) fn refresh(self: &Rc<Self>, entry: &gtk::Entry, browser: &Rc<Browser>) {
        if !(self.is_active)() || entry.root().is_none() {
            self.cancel_request();
            self.show(entry, Vec::new());
            return;
        }

        let text = entry.text().to_string();
        let current_dir = browser
            .active_location()
            .and_then(|location| location.native_path().map(Path::to_path_buf));
        let show_hidden = browser.preferences().show_hidden;
        match plan_completions(
            &text,
            current_dir.as_deref(),
            &glib::home_dir(),
            show_hidden,
        ) {
            Plan::Ready(candidates) => {
                self.cancel_request();
                self.show(entry, candidates);
            }
            Plan::List(listing) => self.request(entry, browser, text, listing),
        }
    }

    fn request(
        self: &Rc<Self>,
        entry: &gtk::Entry,
        browser: &Rc<Browser>,
        text: String,
        listing: Listing,
    ) {
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        self.selected_index.set(None);
        let weak_completion = Rc::downgrade(self);
        let weak_entry = entry.downgrade();
        let weak_browser = Rc::downgrade(browser);
        let task = glib::MainContext::default().spawn_local(async move {
            let candidates = listing.candidates().await;
            if let (Some(completion), Some(entry), Some(browser)) = (
                weak_completion.upgrade(),
                weak_entry.upgrade(),
                weak_browser.upgrade(),
            ) {
                completion.arrive(&entry, &browser, generation, candidates);
            }
        });
        let previous = self.request.replace(Some(Request {
            generation,
            text,
            deferred: Vec::new(),
            task,
        }));
        if let Some(previous) = previous {
            previous.task.abort();
        }
    }

    fn arrive(
        self: &Rc<Self>,
        entry: &gtk::Entry,
        browser: &Rc<Browser>,
        generation: u64,
        candidates: Vec<CompletionCandidate>,
    ) {
        let current = self
            .request
            .borrow_mut()
            .take_if(|request| request.generation == generation);
        let Some(request) = current else {
            return;
        };
        if entry.text() != request.text.as_str() {
            return;
        }
        self.show(entry, candidates);
        let mut commands = request.deferred.into_iter();
        while let Some(command) = commands.next() {
            let relist = entry.text() != request.text.as_str();
            let proceeds =
                self.press(entry, browser, command, relist) == glib::Propagation::Proceed;
            if proceeds && matches!(command, Command::Activate) {
                (self.on_activate)();
            }
            if let Some(next) = self.request.borrow_mut().as_mut() {
                next.deferred.extend(commands);
                break;
            }
        }
    }

    fn press(
        self: &Rc<Self>,
        entry: &gtk::Entry,
        browser: &Rc<Browser>,
        command: Command,
        relist: bool,
    ) -> glib::Propagation {
        if self.defer(command) {
            return glib::Propagation::Stop;
        }
        let visible = self.popover.is_visible();
        match command {
            Command::Complete => self.handle_tab(entry, browser, relist),
            Command::Down if visible => self.move_selection(1),
            Command::Down => {
                if relist && self.request.borrow().is_none() {
                    self.refresh(entry, browser);
                }
            }
            Command::Step(delta) if visible => self.move_selection(delta),
            Command::Step(_) => return glib::Propagation::Proceed,
            Command::Activate => match self.selected_candidate().filter(|_| visible) {
                Some(candidate) => self.activate_candidate(entry, &candidate),
                None => {
                    self.dismiss();
                    return glib::Propagation::Proceed;
                }
            },
        }
        glib::Propagation::Stop
    }

    fn cancel_request(&self) {
        if let Some(request) = self.request.take() {
            request.task.abort();
        }
    }

    fn defer(&self, command: Command) -> bool {
        let visible = self.popover.is_visible();
        let mut request = self.request.borrow_mut();
        let Some(request) = request.as_mut() else {
            return false;
        };
        let waits = !request.deferred.is_empty()
            || match command {
                Command::Complete => true,
                Command::Down | Command::Step(_) => visible,
                Command::Activate => false,
            };
        if waits {
            request.deferred.push(command);
        }
        waits
    }

    fn selected_candidate(&self) -> Option<CompletionCandidate> {
        let index = self.selected_index.get()?;
        self.candidates.borrow().get(index).cloned()
    }

    fn show(self: &Rc<Self>, entry: &gtk::Entry, candidates: Vec<CompletionCandidate>) {
        self.selected_index.set(None);
        if candidates.is_empty() {
            self.candidates.borrow_mut().clear();
            self.hide();
            return;
        }

        self.render_candidates(&candidates);
        self.candidates.replace(candidates);
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
        if !(self.is_active)() {
            self.dismiss();
            return;
        }
        if self.candidates.borrow().is_empty() {
            self.hide();
            return;
        }
        if entry.width() <= 0 {
            return;
        }
        // Include the entry's CSS padding and border, not just its text area.
        let Some(bounds) = entry.compute_bounds(entry) else {
            return;
        };
        let width = bounds.width().ceil() as i32;
        if width <= 0 {
            return;
        }
        self.popover.set_size_request(width, -1);
        self.popover.set_pointing_to(Some(&gdk::Rectangle::new(
            bounds.x().floor() as i32,
            bounds.y().floor() as i32,
            width,
            (bounds.height().ceil() as i32).max(32),
        )));
        self.popover.set_offset(0, 6);
        self.pending_reveal.set(false);
        if !self.popover.is_visible() {
            self.popover.popup();
        }
    }

    pub(crate) fn dismiss(&self) {
        self.cancel_request();
        self.hide();
    }

    fn hide(&self) {
        self.pending_reveal.set(false);
        self.popover.popdown();
    }

    fn activate_candidate(&self, entry: &gtk::Entry, candidate: &CompletionCandidate) {
        self.dismiss();
        self.suppress_refresh.set(true);
        entry.set_text(&candidate.replacement);
        entry.set_position(-1);
        self.suppress_refresh.set(false);
        (self.on_activate)();
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

    fn complete_text(&self, entry: &gtk::Entry, replacement: &str) {
        self.suppress_refresh.set(true);
        entry.set_text(replacement);
        entry.set_position(-1);
        self.suppress_refresh.set(false);
        self.dismiss();
    }

    fn handle_tab(self: &Rc<Self>, entry: &gtk::Entry, browser: &Rc<Browser>, relist: bool) {
        if relist && !self.popover.is_visible() {
            self.refresh(entry, browser);
            if self.defer(Command::Complete) {
                return;
            }
        }
        self.complete(entry);
    }

    fn complete(&self, entry: &gtk::Entry) {
        let candidates = self.candidates.borrow().clone();
        if candidates.is_empty() {
            return;
        }

        if let Some(candidate) = self.selected_candidate() {
            self.complete_text(entry, &candidate.replacement);
            return;
        }

        if candidates.len() == 1 {
            let candidate = &candidates[0];
            self.complete_text(entry, &candidate.replacement);
            return;
        }

        let replacements: Vec<_> = candidates.iter().map(|c| c.replacement.clone()).collect();
        if let Some(common) = longest_common_prefix(&replacements) {
            let current_text = entry.text().to_string();
            if common.len() > current_text.len() {
                self.complete_text(entry, &common);
                return;
            }
        }

        self.move_selection(1);
    }
}

enum Plan {
    Ready(Vec<CompletionCandidate>),
    List(Listing),
}

struct Folder {
    path: PathBuf,
    prepend: String,
    parent_hint: String,
}

impl Folder {
    fn new(path: PathBuf, prepend: String, home: &Path) -> Self {
        let parent_hint = compact_hint(&path, home);
        Self {
            path,
            prepend,
            parent_hint,
        }
    }

    async fn candidates(
        &self,
        leaf_prefix: &str,
        include_hidden: bool,
        match_len: usize,
    ) -> Vec<CompletionCandidate> {
        let names =
            probes::child_folders(&self.path, leaf_prefix, include_hidden, MAX_SCANNED_ENTRIES)
                .await;
        let mut dirs: Vec<_> = names
            .into_iter()
            .map(|name| CompletionCandidate {
                display_name: format!("{name}/"),
                replacement: format!("{}{name}/", self.prepend),
                parent_hint: self.parent_hint.clone(),
                match_len,
            })
            .collect();
        dirs.sort_by_cached_key(|candidate| candidate.display_name.to_lowercase());
        dirs.truncate(MAX_COMPLETION_CANDIDATES);
        dirs
    }
}

struct Listing {
    folder: Folder,
    leaf_prefix: String,
    show_hidden: bool,
    inside: Option<Folder>,
}

impl Listing {
    async fn candidates(self) -> Vec<CompletionCandidate> {
        let include_hidden = self.leaf_prefix.starts_with('.') || self.show_hidden;
        let match_len = self.leaf_prefix.chars().count();
        let candidates = self
            .folder
            .candidates(&self.leaf_prefix, include_hidden, match_len)
            .await;
        if candidates.len() <= 1
            && let Some(inside) = &self.inside
        {
            let children = inside.candidates("", self.show_hidden, 0).await;
            if !children.is_empty() {
                return children;
            }
        }
        candidates
    }
}

fn plan_completions(
    input: &str,
    current_dir: Option<&Path>,
    home: &Path,
    show_hidden: bool,
) -> Plan {
    let input = input.trim();
    let list = |folder: Folder, leaf_prefix: &str, inside: Option<Folder>| {
        Plan::List(Listing {
            folder,
            leaf_prefix: leaf_prefix.to_owned(),
            show_hidden,
            inside: inside.filter(|_| !input.ends_with('/')),
        })
    };
    if input.is_empty() {
        return match current_dir {
            Some(current) => list(
                Folder::new(current.to_path_buf(), String::new(), home),
                "",
                None,
            ),
            None => Plan::Ready(Vec::new()),
        };
    }

    if input == "~" {
        return Plan::Ready(vec![CompletionCandidate {
            display_name: "~/".to_owned(),
            replacement: "~/".to_owned(),
            parent_hint: "home".to_owned(),
            match_len: 1,
        }]);
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
        let rel_trimmed = relative.trim_start_matches('/');
        let inside = (!rel_trimmed.is_empty()).then(|| {
            Folder::new(
                home.join(rel_trimmed),
                format!("~/{}/", rel_trimmed.trim_end_matches('/')),
                home,
            )
        });
        return list(Folder::new(base_dir, prepend, home), leaf_prefix, inside);
    }

    if input.starts_with('~') {
        return Plan::Ready(Vec::new());
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
        let inside = (input != "/").then(|| {
            Folder::new(
                PathBuf::from(input),
                format!("{}/", input.trim_end_matches('/')),
                home,
            )
        });
        return list(Folder::new(base_dir, prepend, home), leaf_prefix, inside);
    }

    let Some(current) = current_dir else {
        return Plan::Ready(Vec::new());
    };
    let (base_dir, leaf_prefix) = if input.ends_with('/') {
        (current.join(input), "")
    } else if let Some((parent_part, leaf)) = input.rsplit_once('/') {
        (current.join(parent_part), leaf)
    } else {
        (current.to_path_buf(), input)
    };
    let input_path = current.join(input);
    let inside = Folder::new(input_path.clone(), directory_prefix(&input_path), home);
    list(
        Folder::new(base_dir.clone(), directory_prefix(&base_dir), home),
        leaf_prefix,
        Some(inside),
    )
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
