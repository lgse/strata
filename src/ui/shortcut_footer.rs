// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{gdk, glib, prelude::*};

use super::browser_modes::BrowserMode;

type Shortcut = (&'static str, &'static str);

const FILES: &[Shortcut] = &[
    ("Enter", "Open the current item"),
    ("Space", "Toggle quick preview"),
    ("Ctrl+C / Ctrl+X", "Copy / cut selected items"),
    ("Ctrl+V", "Paste into the indicated directory"),
    ("Ctrl+D", "Duplicate selected items"),
    ("Delete", "Move selected items to Trash, when supported"),
    ("Shift+Delete", "Permanently delete selected items"),
    ("Ctrl+Z", "Undo the last file operation"),
    ("F2", "Rename"),
    ("Ctrl+Shift+N", "Create a folder"),
    ("Ctrl+A", "Select all items in the focused pane"),
    ("Shift+↑ / ↓", "Extend selection"),
    ("Alt+Enter", "Show item properties"),
    ("y / p", "Copy path / pin a folder (type-to-search off)"),
];

const TOOLS: &[Shortcut] = &[
    ("Ctrl+F", "Filter the current pane"),
    ("Ctrl+K", "Open global search"),
    ("Ctrl+L", "Edit the location"),
    ("Ctrl+T", "Open a terminal"),
    ("F5 / Ctrl+R", "Refresh"),
    ("Ctrl+H / Ctrl+.", "Show or hide hidden files"),
    ("Ctrl+B", "Show or hide the sidebar"),
    ("Ctrl+Shift+B", "Switch focus between sidebar and browser"),
    ("Ctrl+,", "Open Settings"),
    ("Escape", "Close preview or cancel the current interaction"),
    ("F1", "Show or hide this reference"),
];

#[derive(Clone)]
pub(super) struct ShortcutFooter {
    root: gtk::Box,
    summary: gtk::Label,
    paste: gtk::Label,
    show_hints: Rc<Cell<bool>>,
    pending_popup: Rc<Cell<bool>>,
    more: gtk::MenuButton,
    popover: gtk::Popover,
    reference: gtk::Box,
    focus_before: Rc<RefCell<Option<glib::WeakRef<gtk::Widget>>>>,
}

impl ShortcutFooter {
    pub fn new(mode: BrowserMode) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        root.add_css_class("shortcut-footer");
        let summary = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .single_line_mode(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        summary.add_css_class("shortcut-footer-summary");
        root.append(&summary);
        let paste = gtk::Label::new(Some("Ctrl+V  Paste available"));
        paste.add_css_class("shortcut-footer-paste");
        paste.set_tooltip_text(Some("Files are on the clipboard. Press Ctrl+V to paste."));
        paste.set_visible(false);
        root.append(&paste);
        let show_hints = Rc::new(Cell::new(true));
        let pending_popup = Rc::new(Cell::new(false));

        let more = gtk::MenuButton::new();
        more.set_child(Some(&gtk::Label::new(Some("F1  Shortcuts"))));
        more.add_css_class("shortcut-footer-button");
        more.set_tooltip_text(Some("Show all file-view shortcuts (F1)"));
        root.append(&more);
        let popover = gtk::Popover::builder()
            .position(gtk::PositionType::Top)
            .halign(gtk::Align::End)
            .has_arrow(false)
            .build();
        popover.add_css_class("shortcut-popover");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let title = gtk::Label::builder()
            .label("Keyboard shortcuts")
            .xalign(0.0)
            .hexpand(true)
            .build();
        title.add_css_class("shortcut-reference-title");
        let close = gtk::Button::with_label("Close");
        close.add_css_class("shortcut-reference-close");
        let weak = popover.downgrade();
        close.connect_clicked(move |_| {
            if let Some(popover) = weak.upgrade() {
                popover.popdown();
            }
        });
        header.append(&title);
        header.append(&close);
        content.append(&header);
        let note = gtk::Label::builder()
            .label("File-view shortcuts. Text fields, dialogs, and media previews use their own controls.")
            .xalign(0.0).wrap(true).build();
        note.add_css_class("shortcut-reference-note");
        content.append(&note);
        let reference = gtk::Box::new(gtk::Orientation::Vertical, 16);
        let scroll = gtk::ScrolledWindow::builder()
            .child(&reference)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .overlay_scrolling(false)
            .propagate_natural_height(true)
            .max_content_height(440)
            .min_content_width(420)
            .focusable(true)
            .build();
        scroll.add_css_class("fixed-scrollbar");
        content.append(&scroll);
        popover.set_child(Some(&content));
        let weak_scroll = scroll.downgrade();
        popover.connect_show(move |popover| {
            if let Some(scroll) = weak_scroll.upgrade()
                && let Some(window) = popover.root().and_downcast::<gtk::Window>()
            {
                scroll.vadjustment().set_value(scroll.vadjustment().lower());
                scroll.set_max_content_height((window.height() - 150).clamp(100, 440));
                scroll.set_min_content_width((window.width() - 60).clamp(260, 420));
            }
        });
        more.set_popover(Some(&popover));
        let focus_before: Rc<RefCell<Option<glib::WeakRef<gtk::Widget>>>> =
            Rc::new(RefCell::new(None));
        let restored_focus = focus_before.clone();
        let weak_more = more.downgrade();
        let weak_root = root.downgrade();
        let closed_hints = show_hints.clone();
        let closed_pending = pending_popup.clone();
        let weak_popover = popover.downgrade();
        popover.connect_closed(move |_| {
            let restored_focus = restored_focus.clone();
            let closed_pending = closed_pending.clone();
            let weak_more = weak_more.clone();
            let weak_root = weak_root.clone();
            let closed_hints = closed_hints.clone();
            let weak_popover = weak_popover.clone();
            // MenuButton restores its own focus after ::closed; wait without overriding a newer focus move.
            glib::idle_add_local_once(move || {
                if closed_pending.get()
                    || weak_popover
                        .upgrade()
                        .is_some_and(|popover| popover.is_visible())
                {
                    return;
                }
                let previous = restored_focus.borrow_mut().take();
                let Some(more) = weak_more.upgrade() else {
                    return;
                };
                let still_on_button =
                    more.root()
                        .and_then(|root| root.focus())
                        .is_some_and(|focused| {
                            focused == *more.upcast_ref::<gtk::Widget>()
                                || focused.is_ancestor(&more)
                        });
                if still_on_button
                    && let Some(previous) = previous.and_then(|previous| previous.upgrade())
                    && previous.is_mapped()
                {
                    previous.grab_focus();
                }
                if let Some(root) = weak_root.upgrade() {
                    root.set_visible(closed_hints.get());
                }
            });
        });
        let footer = Self {
            root,
            summary,
            paste,
            show_hints,
            pending_popup,
            more,
            popover,
            reference,
            focus_before,
        };
        footer.set_mode(mode);
        footer
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn bind_preferences(&self, manager: &super::theme::ThemeManager) {
        let show_hints = self.show_hints.clone();
        let pending = self.pending_popup.clone();
        let weak_popover = self.popover.downgrade();
        manager.on_keybinding_hints_changed(&self.root, move |root, enabled| {
            show_hints.set(enabled);
            if !enabled {
                pending.set(false);
            }
            root.set_visible(
                enabled
                    || weak_popover
                        .upgrade()
                        .is_some_and(|popover| popover.is_visible()),
            );
        });
    }

    pub fn connect_clipboard(&self, clipboard: &gdk::Clipboard) -> glib::SignalHandlerId {
        let label = self.paste.downgrade();
        let generation = Rc::new(Cell::new(0));
        refresh_paste_availability(clipboard, &label, &generation);
        clipboard.connect_changed(move |clipboard| {
            refresh_paste_availability(clipboard, &label, &generation);
        })
    }

    pub fn set_mode(&self, mode: BrowserMode) {
        let shortcuts = summary_shortcuts(mode);
        self.summary.set_markup(
            &shortcuts
                .iter()
                .map(|(key, action)| {
                    format!(
                        "<b>{}</b>  {}",
                        glib::markup_escape_text(key),
                        glib::markup_escape_text(action)
                    )
                })
                .collect::<Vec<_>>()
                .join("     "),
        );
        self.summary.set_tooltip_text(Some(
            &shortcuts
                .iter()
                .map(|(key, action)| format!("{key}  {action}"))
                .collect::<Vec<_>>()
                .join("  ·  "),
        ));
        while let Some(child) = self.reference.first_child() {
            self.reference.remove(&child);
        }
        append_section(
            &self.reference,
            match mode {
                BrowserMode::Columns => "Columns navigation",
                BrowserMode::Grid => "Grid navigation",
                BrowserMode::Explorer => "Explorer navigation",
            },
            &navigation_shortcuts(mode),
        );
        append_section(&self.reference, "Files and selection", FILES);
        append_section(&self.reference, "Search and tools", TOOLS);
    }

    pub fn handle_key(
        &self,
        key: gdk::Key,
        modifiers: gdk::ModifierType,
    ) -> Option<glib::Propagation> {
        let command_modifiers = modifiers.intersects(
            gdk::ModifierType::CONTROL_MASK
                | gdk::ModifierType::ALT_MASK
                | gdk::ModifierType::SUPER_MASK,
        );
        if key == gdk::Key::F1
            && !command_modifiers
            && !modifiers.contains(gdk::ModifierType::SHIFT_MASK)
        {
            if self.popover.is_visible() || self.pending_popup.replace(false) {
                if self.popover.is_visible() {
                    self.more.popdown();
                } else {
                    self.focus_before.take();
                    self.root.set_visible(self.show_hints.get());
                }
            } else {
                if self.focus_before.borrow().is_none() {
                    self.focus_before.replace(
                        self.root
                            .root()
                            .and_then(|root| root.focus())
                            .map(|widget| widget.downgrade()),
                    );
                }
                if self.root.is_visible() && self.more.width() > 0 {
                    self.more.popup();
                } else {
                    self.pending_popup.set(true);
                    self.root.set_visible(true);
                    let pending = self.pending_popup.clone();
                    let weak_more = self.more.downgrade();
                    // A hidden footer needs an allocation before its popover can be positioned.
                    self.root.add_tick_callback(move |_, _| {
                        let Some(more) = weak_more.upgrade() else {
                            return glib::ControlFlow::Break;
                        };
                        if !pending.get() {
                            return glib::ControlFlow::Break;
                        }
                        if !more.is_mapped() || more.width() == 0 {
                            return glib::ControlFlow::Continue;
                        }
                        pending.set(false);
                        more.popup();
                        glib::ControlFlow::Break
                    });
                }
            }
            return Some(glib::Propagation::Stop);
        }
        if self.pending_popup.get() {
            if key == gdk::Key::Escape {
                self.pending_popup.set(false);
                self.focus_before.take();
                self.root.set_visible(self.show_hints.get());
            }
            return Some(glib::Propagation::Stop);
        }
        if !self.popover.is_visible() {
            return None;
        }
        if key == gdk::Key::Escape {
            self.more.popdown();
            return Some(glib::Propagation::Stop);
        }
        // The reference is read-only: never let a shortcut operate on files behind it.
        Some(
            if !command_modifiers
                && matches!(
                    key,
                    gdk::Key::Tab
                        | gdk::Key::ISO_Left_Tab
                        | gdk::Key::Up
                        | gdk::Key::Down
                        | gdk::Key::Left
                        | gdk::Key::Right
                        | gdk::Key::Page_Up
                        | gdk::Key::Page_Down
                        | gdk::Key::Home
                        | gdk::Key::End
                        | gdk::Key::Return
                        | gdk::Key::KP_Enter
                        | gdk::Key::space
                )
            {
                glib::Propagation::Proceed
            } else {
                glib::Propagation::Stop
            },
        )
    }
}

fn refresh_paste_availability(
    clipboard: &gdk::Clipboard,
    label: &glib::WeakRef<gtk::Label>,
    generation: &Rc<Cell<u64>>,
) {
    let revision = generation.get().wrapping_add(1);
    generation.set(revision);
    let Some(paste) = label.upgrade() else {
        return;
    };
    paste.set_visible(false);
    let formats = clipboard.formats();
    if !formats.contains_type(gdk::FileList::static_type())
        && !formats.contain_mime_type("text/uri-list")
    {
        return;
    }
    let clipboard = clipboard.clone();
    let label = label.clone();
    let generation = generation.clone();
    glib::MainContext::default().spawn_local(async move {
        let available = clipboard
            .read_value_future(gdk::FileList::static_type(), glib::Priority::DEFAULT)
            .await
            .ok()
            .and_then(|value| value.get::<gdk::FileList>().ok())
            .is_some_and(|files| !files.files().is_empty());
        if revision == generation.get()
            && let Some(label) = label.upgrade()
        {
            label.set_visible(available);
        }
    });
}

fn summary_shortcuts(mode: BrowserMode) -> Vec<Shortcut> {
    let mut shortcuts = vec![match mode {
        BrowserMode::Columns => ("↑↓ ←→", "Navigate"),
        BrowserMode::Grid => ("↑↓←→", "Move"),
        BrowserMode::Explorer => ("↑↓", "Move"),
    }];
    shortcuts.push(match mode {
        BrowserMode::Columns => ("← at first pane", "Sidebar"),
        BrowserMode::Grid => ("← at edge", "Sidebar"),
        BrowserMode::Explorer => ("←", "Sidebar"),
    });
    shortcuts.extend_from_slice(&[
        ("↑ at top", "Header"),
        ("Enter", "Open"),
        ("Space", "Preview"),
        ("Ctrl+F", "Filter"),
        ("Ctrl+C / X", "Copy / cut"),
        ("Del", "Trash"),
    ]);
    shortcuts
}

fn navigation_shortcuts(mode: BrowserMode) -> Vec<Shortcut> {
    let mut shortcuts = match mode {
        BrowserMode::Columns => vec![
            ("↑ / ↓", "Move between items"),
            ("← / →", "Parent pane / open or enter the current item"),
            ("← at first pane", "Focus the visible sidebar"),
            (
                "Backspace",
                "Close the current pane or go to the parent folder",
            ),
            (
                "h / j / k / l",
                "Vim-style column navigation (type-to-search off)",
            ),
        ],
        BrowserMode::Grid => vec![
            ("↑ ↓ ← →", "Move spatially between tiles"),
            ("← at left edge", "Focus the visible sidebar"),
            ("Backspace", "Go to the parent folder"),
            ("h / l", "Parent folder / open item (type-to-search off)"),
            ("j / k", "Next / previous item (type-to-search off)"),
        ],
        BrowserMode::Explorer => vec![
            ("↑ / ↓", "Move between file rows"),
            ("←", "Focus the visible sidebar"),
            ("Backspace", "Go to the parent folder"),
            ("h / l", "Parent folder / open item (type-to-search off)"),
            ("j / k", "Next / previous item (type-to-search off)"),
        ],
    };
    shortcuts.extend_from_slice(&[
        ("↑ at top", "Focus the navigation header"),
        ("← / → in header", "Move between header controls"),
        ("↓ in header", "Return to the files"),
        ("→ in sidebar", "Return to the browser"),
        ("Alt+← / Alt+→", "Back / forward in history"),
        ("Alt+↑", "Go to the parent folder"),
        ("Alt+Home", "Go to Home"),
        ("Home / End", "First / last item"),
        ("PgUp / PgDn", "Move one page"),
        ("Tab / Shift+Tab", "Next / previous interface control"),
    ]);
    shortcuts
}

fn append_section(parent: &gtk::Box, title: &str, shortcuts: &[Shortcut]) {
    let section = gtk::Box::new(gtk::Orientation::Vertical, 7);
    let heading = gtk::Label::builder().label(title).xalign(0.0).build();
    heading.add_css_class("shortcut-reference-heading");
    section.append(&heading);
    for (key, action) in shortcuts {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        let key = gtk::Label::builder()
            .label(*key)
            .xalign(0.0)
            .width_chars(17)
            .build();
        key.add_css_class("shortcut-reference-key");
        let action = gtk::Label::builder()
            .label(*action)
            .xalign(0.0)
            .hexpand(true)
            .wrap(true)
            .build();
        action.add_css_class("shortcut-reference-description");
        row.append(&key);
        row.append(&action);
        section.append(&row);
    }
    parent.append(&section);
}

#[cfg(test)]
mod tests;
