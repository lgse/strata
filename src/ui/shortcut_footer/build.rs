// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{gdk, glib, prelude::*};

use super::{
    ShortcutFooter,
    catalog::{ReferenceQuery, hint_note_attributes, reference_note, render_reference},
    indicators::{ChordIndicator, CurrentHit},
    prompt::PromptBar,
    reference::{ReferenceArea, ReferenceLayout, ReferenceSession},
    status::watch_status_widget,
};
use crate::ui::{
    browser_modes::BrowserMode,
    keys::{Keys, bindings},
    shortcut_reference::{ReferenceScope, with_keys},
};

/// The footer's status row and the prompt that can replace it.
struct StatusRow {
    root: gtk::Stack,
    status: gtk::Box,
    count: gtk::Label,
    paste: gtk::Label,
    tag: gtk::Label,
    chord: gtk::Label,
    chords: ChordIndicator,
    visual: gtk::Label,
    filter: gtk::Label,
    current: CurrentHit,
    prompt: PromptBar,
    feedback: gtk::Label,
}

/// The shortcut reference panel, before it opens over a window.
struct ReferencePanel {
    content: gtk::Box,
    header: gtk::Box,
    search: gtk::Entry,
    body: gtk::Box,
    categories: gtk::Box,
    sidebar: gtk::ScrolledWindow,
    reference: gtk::Box,
    scroll: gtk::ScrolledWindow,
    note: gtk::Label,
    selected_category: Rc<RefCell<String>>,
    active_area: Rc<Cell<ReferenceArea>>,
    reference_scope: Rc<Cell<ReferenceScope>>,
}

fn status_label(class: &str) -> gtk::Label {
    let label = gtk::Label::new(None);
    label.add_css_class(class);
    label.set_visible(false);
    label
}

fn paste_label() -> gtk::Label {
    let paste = gtk::Label::new(Some(&crate::i18n::tr("Files on clipboard")));
    paste.add_css_class("shortcut-footer-paste");
    crate::ui::shortcut_reference::follow_keys(&paste, |paste| {
        crate::ui::accessibility::set_description(
            paste,
            Some(&with_keys(
                "Press %{keys} to paste into a supported directory.",
                Keys::One(bindings::PASTE.shown()),
            )),
        );
    });
    paste.set_visible(false);
    paste
}

fn tenxer_tag() -> gtk::Label {
    let tag = gtk::Label::new(Some(crate::ui::tenxer_mode::TAG_TEXT));
    tag.add_css_class("tenxer-tag");
    let tag_name = crate::i18n::tr(crate::ui::tenxer_mode::TAG_NAME);
    crate::ui::accessibility::set_description(&tag, Some(&tag_name));
    crate::ui::accessibility::set_label(&tag, &tag_name);
    tag.set_visible(false);
    tag
}

impl StatusRow {
    fn build() -> Self {
        let root = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(120)
            .build();
        root.add_css_class("shortcut-footer");
        let status = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let count = status_label("shortcut-footer-count");
        let paste = paste_label();
        let tag = tenxer_tag();
        let chord = status_label("shortcut-footer-chord-pill");
        let chords = ChordIndicator::attach(&chord);
        let visual = status_label("shortcut-footer-chord");
        let filter = gtk::Label::new(None);
        filter.add_css_class("shortcut-footer-chord");
        filter.set_ellipsize(gtk::pango::EllipsizeMode::End);
        filter.set_max_width_chars(40);
        filter.set_visible(false);
        let current = CurrentHit::new();
        let prompt = PromptBar::new();
        let feedback = status_label("shortcut-footer-feedback");
        for widget in [&paste, &filter, &visual, &feedback, &count, &chord, &tag] {
            status.append(widget);
        }
        root.add_child(&status);
        root.add_child(&prompt.bar);
        Self {
            root,
            status,
            count,
            paste,
            tag,
            chord,
            chords,
            visual,
            filter,
            current,
            prompt,
            feedback,
        }
    }

    /// Places the reference toggle first, then the search hit and a spacer.
    fn insert_reference_toggle(&self, more: &gtk::ToggleButton) {
        self.status.prepend(more);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        self.status
            .insert_child_after(&self.current.root, Some(more));
        self.status
            .insert_child_after(&spacer, Some(&self.current.root));
    }

    fn widgets(&self, more: &gtk::ToggleButton) -> Rc<RefCell<Vec<gtk::Widget>>> {
        Rc::new(RefCell::new(vec![
            self.paste.clone().upcast::<gtk::Widget>(),
            self.count.clone().upcast(),
            more.clone().upcast(),
            self.tag.clone().upcast(),
            self.chord.clone().upcast(),
            self.visual.clone().upcast(),
            self.filter.clone().upcast(),
            self.current.root.clone().upcast(),
            self.prompt.bar.clone().upcast(),
            self.feedback.clone().upcast(),
        ]))
    }
}

fn reference_toggle() -> gtk::ToggleButton {
    let more = gtk::ToggleButton::new();
    let more_text = gtk::Label::new(None);
    more.set_child(Some(&more_text));
    more.add_css_class("shortcut-footer-button");
    let more_text = more_text.downgrade();
    crate::ui::shortcut_reference::follow_keys(&more, move |more| {
        let reference = Keys::One(bindings::REFERENCE.alternate(0).shown());
        let more_label = with_keys("%{keys} Shortcuts", reference);
        if let Some(text) = more_text.upgrade() {
            text.set_text(&more_label);
        }
        crate::ui::accessibility::set_label(more, &more_label);
        crate::ui::accessibility::set_description(
            more,
            Some(&with_keys(
                "Show all file-view shortcuts (%{keys})",
                reference,
            )),
        );
    });
    more
}

fn reference_header() -> (gtk::Box, gtk::Entry) {
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    header.add_css_class("shortcut-reference-header");
    let title = gtk::Label::new(Some(&crate::i18n::tr("Keyboard shortcuts")));
    title.add_css_class("shortcut-reference-title");
    title.set_hexpand(true);
    title.set_xalign(0.0);
    header.append(&title);
    let search = gtk::Entry::new();
    search.add_css_class("form-control");
    search.set_placeholder_text(Some(&crate::i18n::tr("Search actions or keys…")));
    search.set_hexpand(true);
    search.add_css_class("shortcut-reference-search");
    crate::ui::accessibility::set_label(&search, &crate::i18n::tr("Search actions or keys…"));
    header.append(&search);
    (header, search)
}

fn category_sidebar() -> (gtk::Box, gtk::ScrolledWindow) {
    let categories = gtk::Box::new(gtk::Orientation::Vertical, 0);
    categories.add_css_class("shortcut-reference-categories");
    categories.set_valign(gtk::Align::Start);
    let sidebar = gtk::ScrolledWindow::builder()
        .child(&categories)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_height(true)
        .build();
    sidebar.add_css_class("shortcut-reference-sidebar");
    (categories, sidebar)
}

fn results_scroll(reference: &gtk::Box) -> gtk::ScrolledWindow {
    let scroll = gtk::ScrolledWindow::builder()
        .child(reference)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .height_request(440)
        .focusable(true)
        .build();
    scroll.add_css_class("shortcut-reference-scroll");
    scroll
}

/// The panel is outside the window until it opens, so the footer anchors the
/// binding and a closed window releases it.
fn reference_footer_note(root: &gtk::Stack) -> (gtk::Box, gtk::Label) {
    let footer_note = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    footer_note.add_css_class("shortcut-reference-footer");
    let note = gtk::Label::new(None);
    let noted = note.downgrade();
    crate::ui::shortcut_reference::follow_keys(root, move |_| {
        if let Some(note) = noted.upgrade() {
            let note_text = reference_note();
            note.set_text(&note_text);
            note.set_attributes(Some(&hint_note_attributes(&note_text)));
        }
    });
    note.set_wrap(true);
    note.add_css_class("shortcut-reference-note");
    note.set_hexpand(true);
    note.set_xalign(0.0);
    footer_note.append(&note);
    (footer_note, note)
}

impl ReferencePanel {
    fn build(root: &gtk::Stack, mode: BrowserMode) -> Self {
        let content = crate::ui::accessibility::dialog_box(&crate::i18n::tr("Keyboard shortcuts"));
        content.add_css_class("shortcut-reference-panel");
        content.set_halign(gtk::Align::Center);
        content.set_valign(gtk::Align::Center);
        content.set_overflow(gtk::Overflow::Hidden);
        let (header, search) = reference_header();
        content.append(&header);
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        body.add_css_class("shortcut-reference-body");
        let (categories, sidebar) = category_sidebar();
        body.append(&sidebar);
        let reference = gtk::Box::new(gtk::Orientation::Vertical, 24);
        reference.add_css_class("shortcut-reference-results");
        let scroll = results_scroll(&reference);
        body.append(&scroll);
        content.append(&body);
        let (footer_note, note) = reference_footer_note(root);
        content.append(&footer_note);
        let panel = Self {
            content,
            header,
            search,
            body,
            categories,
            sidebar,
            reference,
            scroll,
            note,
            selected_category: Rc::new(RefCell::new(String::from("All"))),
            active_area: Rc::new(Cell::new(ReferenceArea::Search)),
            reference_scope: Rc::new(Cell::new(ReferenceScope::window(mode))),
        };
        panel.render_on_search();
        panel
    }

    fn render_on_search(&self) {
        let search_reference = self.reference.downgrade();
        let search_scope = self.reference_scope.clone();
        let search_category = self.selected_category.clone();
        self.search.connect_changed(move |entry| {
            if let Some(reference) = search_reference.upgrade() {
                render_reference(
                    &reference,
                    &ReferenceQuery {
                        scope: search_scope.get(),
                        selected: &search_category.borrow(),
                        query: &entry.text(),
                        compact: reference.has_css_class("compact"),
                    },
                );
            }
        });
    }

    fn session(&self, more: &gtk::ToggleButton, show_hints: &Rc<Cell<bool>>) -> ReferenceSession {
        let layout = ReferenceLayout {
            header: self.header.downgrade(),
            body: self.body.downgrade(),
            sidebar: self.sidebar.downgrade(),
            categories: self.categories.downgrade(),
            scroll: self.scroll.downgrade(),
            reference: self.reference.downgrade(),
            search: self.search.downgrade(),
            footer: self.note.downgrade(),
            scope: self.reference_scope.clone(),
            selected: self.selected_category.clone(),
        };
        ReferenceSession {
            panel: self.content.downgrade(),
            more: more.downgrade(),
            search: self.search.downgrade(),
            sidebar: self.sidebar.downgrade(),
            scroll: self.scroll.downgrade(),
            active_area: self.active_area.clone(),
            layout,
            show_hints: show_hints.clone(),
            focus_before: Rc::default(),
            attached: Rc::default(),
        }
    }
}

fn toggle_reference_on_click(more: &gtk::ToggleButton, session: &ReferenceSession) {
    let toggled = session.clone();
    more.connect_clicked(move |more| {
        if toggled.is_open() {
            toggled.close();
        } else if !toggled.open(more) {
            more.set_active(false);
        }
    });
}

impl ShortcutFooter {
    pub fn new(mode: BrowserMode) -> Self {
        let row = StatusRow::build();
        let show_hints = Rc::new(Cell::new(true));
        let more = reference_toggle();
        row.insert_reference_toggle(&more);
        let panel = ReferencePanel::build(&row.root, mode);
        let session = panel.session(&more, &show_hints);
        toggle_reference_on_click(&more, &session);
        let status_widgets = row.widgets(&more);
        for widget in status_widgets.borrow().iter() {
            watch_status_widget(widget, &status_widgets, &row.root);
        }
        let footer = Self {
            root: row.root,
            status: row.status,
            paste: row.paste,
            count: row.count,
            show_hints,
            more,
            panel: panel.content,
            session,
            reference: panel.reference,
            categories: panel.categories,
            sidebar: panel.sidebar,
            search: panel.search,
            selected_category: panel.selected_category,
            active_area: panel.active_area,
            scroll: panel.scroll,
            status_widgets,
            tag: row.tag,
            feedback: row.feedback,
            feedback_epoch: Rc::new(Cell::new(0)),
            prompt: row.prompt,
            chords: row.chords,
            visual: row.visual,
            filter: row.filter,
            current: row.current,
            filter_source: Rc::new(RefCell::new(None)),
            filter_retry: Rc::new(Cell::new(false)),
            disposed: Rc::new(Cell::new(false)),
            observed: Rc::new(RefCell::new(std::rc::Weak::new())),
            reference_scope: panel.reference_scope,
        };
        footer.install_reference_keys();
        footer.close_prompt_on_focus_loss();
        footer.set_mode(mode);
        footer
    }

    fn install_reference_keys(&self) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let shortcuts = self.clone();
        crate::ui::keys::on_press(&keys, move |_, press| {
            shortcuts
                .handle_key(press)
                .unwrap_or(glib::Propagation::Proceed)
        });
        self.panel.add_controller(keys);
        let unused = gtk::EventControllerKey::new();
        crate::ui::keys::on_press(&unused, |_, press| {
            if press.modifiers().intersects(
                gdk::ModifierType::CONTROL_MASK
                    | gdk::ModifierType::ALT_MASK
                    | gdk::ModifierType::SUPER_MASK,
            ) {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        self.search.add_controller(unused);
    }
}
