// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use gtk::{gdk, glib, prelude::*};

use super::{
    browser::FilterStatus,
    browser_modes::BrowserMode,
    tenxer_mode::{Chord, Prompt},
};

type Shortcut = (&'static str, &'static str);
type ChordListener = Box<dyn Fn(Option<Chord>)>;
/// `None` while the view is busy rebuilding; the footer then retries on idle.
type FilterSource = Rc<RefCell<Option<Rc<dyn Fn() -> Option<Option<FilterStatus>>>>>>;

#[derive(Clone)]
pub(super) struct ShortcutFooter {
    root: gtk::Stack,
    status: gtk::Box,
    paste: gtk::Label,
    count: gtk::Label,
    show_hints: Rc<Cell<bool>>,
    pending_popup: Rc<Cell<bool>>,
    more: gtk::MenuButton,
    popover: gtk::Popover,
    reference: gtk::Box,
    categories: gtk::Box,
    sidebar: gtk::ScrolledWindow,
    search: gtk::Entry,
    selected_category: Rc<RefCell<String>>,
    active_area: Rc<Cell<ReferenceArea>>,
    scroll: gtk::ScrolledWindow,
    focus_before: Rc<RefCell<Option<glib::WeakRef<gtk::Widget>>>>,
    status_widgets: Rc<RefCell<Vec<gtk::Widget>>>,
    tag: gtk::Label,
    feedback: gtk::Label,
    feedback_epoch: Rc<Cell<u64>>,
    prompt: PromptBar,
    chords: ChordIndicator,
    visual: gtk::Label,
    filter: gtk::Label,
    current: CurrentHit,
    filter_source: FilterSource,
    filter_retry: Rc<Cell<bool>>,
    observed: Rc<RefCell<std::rc::Weak<crate::app::Browser>>>,
    view_mode: Rc<Cell<BrowserMode>>,
}

/// The search hit under the cursor, as its path below the searched folder.
/// The folder part gives way to an ellipsis before the name does.
#[derive(Clone)]
struct CurrentHit {
    root: gtk::Box,
    folder: gtk::Label,
    name: gtk::Label,
}

impl CurrentHit {
    const NAME_MIN_CHARS: usize = 32;

    fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.set_visible(false);
        let folder = gtk::Label::new(None);
        folder.set_ellipsize(gtk::pango::EllipsizeMode::End);
        folder.set_max_width_chars(60);
        let name = gtk::Label::new(None);
        name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        for label in [&folder, &name] {
            label.add_css_class("shortcut-footer-chord-hint");
            root.append(label);
        }
        Self { root, folder, name }
    }

    fn set(&self, path: Option<&std::path::Path>) {
        let Some(path) = path.filter(|path| path.file_name().is_some()) else {
            self.root.set_visible(false);
            return;
        };
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let folder = path
            .parent()
            .map(|folder| folder.to_string_lossy())
            .unwrap_or_default();
        self.folder.set_text(&folder);
        self.folder.set_visible(!folder.is_empty());
        let name = if folder.is_empty() {
            name.into_owned()
        } else {
            format!("{}{name}", std::path::MAIN_SEPARATOR)
        };
        self.name.set_text(&name);
        let chars = name.chars().count();
        self.name
            .set_width_chars(chars.min(Self::NAME_MIN_CHARS) as i32);
        let full = path.to_string_lossy();
        self.root.set_tooltip_text(Some(&full));
        super::accessibility::set_label(&self.root, &full);
        self.root.set_visible(true);
    }
}

/// The armed chord and its footer mark. The chord is armed exactly while the
/// mark is showing.
#[derive(Clone)]
struct ChordIndicator {
    armed: Rc<Cell<Option<Chord>>>,
    mark: glib::WeakRef<gtk::Label>,
    hint: glib::WeakRef<gtk::Label>,
    listeners: Rc<RefCell<Vec<ChordListener>>>,
}

impl ChordIndicator {
    fn set(&self, chord: Option<Chord>) {
        if let Some(mark) = self.mark.upgrade() {
            mark.set_text(chord.map_or("", Chord::mark));
            mark.set_visible(chord.is_some());
        }
        if let Some(hint) = self.hint.upgrade() {
            hint.set_text(chord.map_or("", Chord::hint));
            hint.set_visible(chord.is_some());
        }
        if self.armed.replace(chord) != chord {
            for listener in self.listeners.borrow().iter() {
                listener(chord);
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReferenceArea {
    Search,
    Categories,
    Results,
}

#[derive(Clone)]
struct ReferenceLayout {
    header: glib::WeakRef<gtk::Box>,
    body: glib::WeakRef<gtk::Box>,
    sidebar: glib::WeakRef<gtk::ScrolledWindow>,
    categories: glib::WeakRef<gtk::Box>,
    scroll: glib::WeakRef<gtk::ScrolledWindow>,
    reference: glib::WeakRef<gtk::Box>,
    search: glib::WeakRef<gtk::Entry>,
    footer: glib::WeakRef<gtk::Label>,
    more: glib::WeakRef<gtk::MenuButton>,
    mode: Rc<Cell<BrowserMode>>,
    selected: Rc<RefCell<String>>,
}

impl ReferenceLayout {
    fn update(&self, window: &gtk::Window, popover: &gtk::Popover) {
        let (
            Some(header),
            Some(body),
            Some(sidebar),
            Some(categories),
            Some(scroll),
            Some(reference),
            Some(search),
            Some(more),
        ) = (
            self.header.upgrade(),
            self.body.upgrade(),
            self.sidebar.upgrade(),
            self.categories.upgrade(),
            self.scroll.upgrade(),
            self.reference.upgrade(),
            self.search.upgrade(),
            self.more.upgrade(),
        )
        else {
            return;
        };
        if let Some(footer) = self.footer.upgrade() {
            footer.set_max_width_chars(((window.width() - 80) / 9).max(10));
        }
        let compact = window.width() < 1480;
        let changed = reference.has_css_class("compact") != compact;
        header.set_orientation(if compact {
            gtk::Orientation::Vertical
        } else {
            gtk::Orientation::Horizontal
        });
        body.set_orientation(if compact {
            gtk::Orientation::Vertical
        } else {
            gtk::Orientation::Horizontal
        });
        if compact {
            body.add_css_class("compact");
            reference.add_css_class("compact");
            categories.add_css_class("compact");
        } else {
            body.remove_css_class("compact");
            reference.remove_css_class("compact");
            categories.remove_css_class("compact");
        }
        categories.set_spacing(if compact { 8 } else { 0 });
        if changed {
            for button in category_buttons(&categories) {
                if let Some(line) = button.child().and_downcast::<gtk::Box>() {
                    if let Some(label) = line.first_child().and_downcast::<gtk::Label>() {
                        label.set_hexpand(!compact);
                    }
                    if let Some(count) = line.last_child().and_downcast::<gtk::Label>() {
                        count.set_visible(!compact);
                    }
                }
                button.set_hexpand(!compact);
                button.set_halign(if compact {
                    gtk::Align::Start
                } else {
                    gtk::Align::Fill
                });
                if compact {
                    button.add_css_class("action-library-category");
                } else {
                    button.remove_css_class("action-library-category");
                }
            }
        }
        if compact || changed {
            reflow_categories(&categories, compact, (window.width() - 104).max(1));
        }
        sidebar.set_height_request(-1);
        sidebar.set_propagate_natural_height(compact);
        sidebar.set_max_content_height((window.height() / 4).clamp(60, 160));
        sidebar.set_vexpand(!compact);
        scroll.set_width_request(if compact {
            (window.width() - 80).max(1)
        } else {
            (window.width() - 340).clamp(1, 1200)
        });
        scroll
            .set_height_request((window.height() - if compact { 300 } else { 160 }).clamp(1, 620));
        if changed {
            render_reference(
                &reference,
                self.mode.get(),
                &self.selected.borrow(),
                &search.text(),
                compact,
            );
        }
        let panel_height = popover
            .child()
            .map(|child| child.measure(gtk::Orientation::Vertical, -1).1)
            .unwrap_or(0)
            .min(window.height().saturating_sub(40));
        let anchor = gtk::graphene::Point::new(
            window.width() as f32 / 2.0,
            (window.height() + panel_height) as f32 / 2.0,
        );
        if let Some(point) = window.compute_point(&more, &anchor) {
            popover.set_pointing_to(Some(&gdk::Rectangle::new(
                point.x().round() as i32,
                point.y().round() as i32,
                1,
                1,
            )));
        }
    }
}

/// The open footer prompt. Its bar covers the whole footer so typing never
/// shares the row with status marks.
#[derive(Clone)]
struct PromptBar {
    bar: gtk::Box,
    label: gtk::Label,
    entry: gtk::Entry,
    kind: Rc<Cell<Option<Prompt>>>,
}

#[derive(Clone)]
struct WeakPromptBar {
    bar: glib::WeakRef<gtk::Box>,
    label: glib::WeakRef<gtk::Label>,
    entry: glib::WeakRef<gtk::Entry>,
    kind: Rc<Cell<Option<Prompt>>>,
}

impl PromptBar {
    fn new() -> Self {
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        bar.add_css_class("shortcut-footer-prompt-bar");
        bar.set_visible(false);
        let label = gtk::Label::new(None);
        label.add_css_class("shortcut-footer-prompt-label");
        let entry = gtk::Entry::new();
        entry.add_css_class("form-control");
        entry.add_css_class("shortcut-footer-prompt");
        entry.set_hexpand(true);
        bar.append(&label);
        bar.append(&entry);
        Self {
            bar,
            label,
            entry,
            kind: Rc::new(Cell::new(None)),
        }
    }

    fn downgrade(&self) -> WeakPromptBar {
        WeakPromptBar {
            bar: self.bar.downgrade(),
            label: self.label.downgrade(),
            entry: self.entry.downgrade(),
            kind: self.kind.clone(),
        }
    }

    fn has_focus(&self) -> bool {
        self.bar.is_visible()
            && self
                .bar
                .root()
                .and_then(|root| root.focus())
                .is_some_and(|focus| {
                    focus == *self.entry.upcast_ref::<gtk::Widget>()
                        || focus.is_ancestor(&self.entry)
                })
    }

    fn open(&self, stack: &gtk::Stack, kind: Prompt, text: &str) -> bool {
        // Replacing an open prompt must not hand its text to the new kind.
        self.kind.set(None);
        self.entry.set_text("");
        self.kind.set(Some(kind));
        self.label.set_text(kind.label());
        super::accessibility::set_label(&self.entry, kind.name());
        self.entry.set_text(text);
        self.bar.set_visible(true);
        stack.set_visible_child(&self.bar);
        // A pre-filled query stays unselected so typing extends it.
        let focused = self.entry.grab_focus_without_selecting();
        self.entry.set_position(-1);
        focused
    }

    fn close(&self) {
        self.kind.set(None);
        self.entry.set_text("");
        self.bar.set_visible(false);
    }
}

impl WeakPromptBar {
    fn upgrade(&self) -> Option<PromptBar> {
        Some(PromptBar {
            bar: self.bar.upgrade()?,
            label: self.label.upgrade()?,
            entry: self.entry.upgrade()?,
            kind: self.kind.clone(),
        })
    }
}

impl ShortcutFooter {
    pub fn new(mode: BrowserMode) -> Self {
        let root = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(120)
            .build();
        root.add_css_class("shortcut-footer");
        let status = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let count = gtk::Label::new(None);
        count.add_css_class("shortcut-footer-count");
        count.set_visible(false);
        let paste = gtk::Label::new(Some("Files on clipboard"));
        paste.add_css_class("shortcut-footer-paste");
        paste.set_tooltip_text(Some("Press Ctrl+V to paste into a supported directory."));
        paste.set_visible(false);
        let tag = gtk::Label::new(Some(crate::ui::tenxer_mode::TAG_TEXT));
        tag.add_css_class("tenxer-tag");
        tag.set_tooltip_text(Some(crate::ui::tenxer_mode::TAG_NAME));
        super::accessibility::set_label(&tag, crate::ui::tenxer_mode::TAG_NAME);
        tag.set_visible(false);
        let chord = gtk::Label::new(None);
        chord.add_css_class("shortcut-footer-chord");
        chord.set_visible(false);
        let chord_hint = gtk::Label::new(None);
        chord_hint.add_css_class("shortcut-footer-chord-hint");
        chord_hint.set_ellipsize(gtk::pango::EllipsizeMode::End);
        chord_hint.set_visible(false);
        let chords = ChordIndicator {
            armed: Rc::new(Cell::new(None)),
            mark: chord.downgrade(),
            hint: chord_hint.downgrade(),
            listeners: Rc::new(RefCell::new(Vec::new())),
        };
        let visual = gtk::Label::new(None);
        visual.add_css_class("shortcut-footer-chord");
        visual.set_visible(false);
        let filter = gtk::Label::new(None);
        filter.add_css_class("shortcut-footer-chord");
        filter.set_ellipsize(gtk::pango::EllipsizeMode::End);
        filter.set_max_width_chars(40);
        filter.set_visible(false);
        let current = CurrentHit::new();
        let prompt = PromptBar::new();
        let feedback = gtk::Label::new(None);
        feedback.add_css_class("shortcut-footer-feedback");
        feedback.set_visible(false);
        status.append(&paste);
        status.append(&filter);
        status.append(&visual);
        status.append(&chord);
        status.append(&chord_hint);
        // Transient marks grow leftward so the pill stays put.
        status.append(&tag);
        status.append(&feedback);
        status.append(&count);
        root.add_child(&status);
        root.add_child(&prompt.bar);
        let show_hints = Rc::new(Cell::new(true));
        let pending_popup = Rc::new(Cell::new(false));

        let more = gtk::MenuButton::new();
        more.set_child(Some(&gtk::Label::new(Some("F1  Shortcuts"))));
        more.add_css_class("shortcut-footer-button");
        more.set_tooltip_text(Some("Show all file-view shortcuts (F1)"));
        status.prepend(&more);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        status.insert_child_after(&current.root, Some(&more));
        status.insert_child_after(&spacer, Some(&current.root));
        let popover = gtk::Popover::builder()
            .position(gtk::PositionType::Top)
            .halign(gtk::Align::Center)
            .has_arrow(false)
            .build();
        popover.add_css_class("shortcut-popover");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        header.add_css_class("shortcut-reference-header");
        let title = gtk::Label::new(Some("Keyboard shortcuts"));
        title.add_css_class("shortcut-reference-title");
        title.set_hexpand(true);
        title.set_xalign(0.0);
        header.append(&title);
        let search = gtk::Entry::new();
        search.add_css_class("form-control");
        search.set_placeholder_text(Some("Search actions or keys…"));
        search.set_hexpand(true);
        search.add_css_class("shortcut-reference-search");
        header.append(&search);
        content.append(&header);
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        body.add_css_class("shortcut-reference-body");
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
        body.append(&sidebar);
        let reference = gtk::Box::new(gtk::Orientation::Vertical, 24);
        reference.add_css_class("shortcut-reference-results");
        let selected_category = Rc::new(RefCell::new(String::from("All")));
        let active_area = Rc::new(Cell::new(ReferenceArea::Search));
        let scroll = gtk::ScrolledWindow::builder()
            .child(&reference)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .height_request(440)
            .focusable(true)
            .build();
        scroll.add_css_class("shortcut-reference-scroll");
        body.append(&scroll);
        content.append(&body);
        let footer_note = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        footer_note.add_css_class("shortcut-reference-footer");
        let note = gtk::Label::new(Some(
            "Ctrl+B categories · Ctrl+F search · Ctrl+L list · arrows/hjkl move · Tab cycle · Esc close",
        ));
        note.set_wrap(true);
        note.add_css_class("shortcut-reference-note");
        note.set_hexpand(true);
        note.set_xalign(0.0);
        footer_note.append(&note);
        content.append(&footer_note);
        popover.set_child(Some(&content));
        let search_reference = reference.downgrade();
        let view_mode = Rc::new(Cell::new(mode));
        let search_mode = view_mode.clone();
        let search_category = selected_category.clone();
        search.connect_changed(move |entry| {
            if let Some(reference) = search_reference.upgrade() {
                render_reference(
                    &reference,
                    search_mode.get(),
                    &search_category.borrow(),
                    &entry.text(),
                    reference.has_css_class("compact"),
                );
            }
        });
        let layout = ReferenceLayout {
            header: header.downgrade(),
            body: body.downgrade(),
            sidebar: sidebar.downgrade(),
            categories: categories.downgrade(),
            scroll: scroll.downgrade(),
            reference: reference.downgrade(),
            search: search.downgrade(),
            footer: note.downgrade(),
            more: more.downgrade(),
            mode: view_mode.clone(),
            selected: selected_category.clone(),
        };
        let weak_popover = popover.downgrade();
        let weak_search = search.downgrade();
        let weak_more_for_position = more.downgrade();
        let backdrop = Rc::new(RefCell::new(None::<(gtk::Overlay, gtk::Box)>));
        let open_backdrop = backdrop.clone();
        let focus_trap = Rc::new(RefCell::new(None::<(gtk::Window, glib::SignalHandlerId)>));
        let open_focus_trap = focus_trap.clone();
        let trap_search = search.downgrade();
        let trap_sidebar = sidebar.downgrade();
        let trap_scroll = scroll.downgrade();
        let trap_area = active_area.clone();
        let trap_popover = popover.downgrade();
        popover.connect_show(move |popover| {
            trap_area.set(ReferenceArea::Search);
            if let Some(window) = popover.root().and_downcast::<gtk::Window>() {
                if let Some(overlay) = window.child().and_downcast::<gtk::Overlay>() {
                    if let Some(root) = overlay.child().and_downcast::<super::blur::BlurBin>() {
                        root.set_blurred(true);
                    }
                    let layer = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    layer.add_css_class("search-backdrop");
                    layer.set_halign(gtk::Align::Fill);
                    layer.set_valign(gtk::Align::Fill);
                    layer.set_hexpand(true);
                    layer.set_vexpand(true);
                    let click = gtk::GestureClick::new();
                    let dismiss = weak_more_for_position.clone();
                    click.connect_released(move |_, _, _, _| {
                        if let Some(more) = dismiss.upgrade() {
                            more.popdown();
                        }
                    });
                    layer.add_controller(click);
                    overlay.add_overlay(&layer);
                    *open_backdrop.borrow_mut() = Some((overlay, layer));
                }
                let search = trap_search.clone();
                let sidebar = trap_sidebar.clone();
                let scroll = trap_scroll.clone();
                let area = trap_area.clone();
                let trapped_popover = trap_popover.clone();
                let id = window.connect_focus_widget_notify(move |window| {
                    let Some(popover) = trapped_popover
                        .upgrade()
                        .filter(|popover| popover.is_visible())
                    else {
                        return;
                    };
                    let Some(focus) = gtk::prelude::RootExt::focus(window) else {
                        return;
                    };
                    if focus != *popover.upcast_ref::<gtk::Widget>() && !focus.is_ancestor(&popover)
                    {
                        if let Some(search) = search.upgrade() {
                            search.grab_focus();
                        }
                        return;
                    }
                    if let (Some(search), Some(sidebar), Some(scroll)) =
                        (search.upgrade(), sidebar.upgrade(), scroll.upgrade())
                    {
                        if focus == *search.upcast_ref::<gtk::Widget>()
                            || focus.is_ancestor(&search)
                        {
                            area.set(ReferenceArea::Search);
                        } else if focus == *sidebar.upcast_ref::<gtk::Widget>()
                            || focus.is_ancestor(&sidebar)
                        {
                            area.set(ReferenceArea::Categories);
                        } else if focus == *scroll.upcast_ref::<gtk::Widget>()
                            || focus.is_ancestor(&scroll)
                        {
                            area.set(ReferenceArea::Results);
                        }
                    }
                });
                *open_focus_trap.borrow_mut() = Some((window.clone(), id));
                if let Some(scroll) = layout.scroll.upgrade() {
                    scroll.vadjustment().set_value(scroll.vadjustment().lower());
                }
                layout.update(&window, popover);
                let last_size = Cell::new((window.width(), window.height()));
                let window = window.downgrade();
                let layout = layout.clone();
                popover.add_tick_callback(move |popover, _| {
                    let Some(window) = window.upgrade() else {
                        return glib::ControlFlow::Break;
                    };
                    if !popover.is_visible() {
                        return glib::ControlFlow::Break;
                    }
                    let size = (window.width(), window.height());
                    if last_size.replace(size) != size {
                        layout.update(&window, popover);
                    }
                    glib::ControlFlow::Continue
                });
            }
            let search = weak_search.clone();
            let popover = weak_popover.clone();
            // Show runs before the popover can take focus; grab it once mapped.
            glib::idle_add_local_once(move || {
                if popover
                    .upgrade()
                    .is_some_and(|popover| popover.is_visible())
                    && let Some(search) = search.upgrade()
                {
                    search.grab_focus();
                }
            });
        });
        more.set_popover(Some(&popover));
        let focus_before: Rc<RefCell<Option<glib::WeakRef<gtk::Widget>>>> =
            Rc::new(RefCell::new(None));
        let restored_focus = focus_before.clone();
        let close_backdrop = backdrop;
        let close_focus_trap = focus_trap;
        let weak_more = more.downgrade();
        let closed_hints = show_hints.clone();
        let closed_pending = pending_popup.clone();
        let weak_popover = popover.downgrade();
        popover.connect_closed(move |_| {
            if let Some((window, id)) = close_focus_trap.borrow_mut().take() {
                window.disconnect(id);
            }
            if let Some((overlay, layer)) = close_backdrop.borrow_mut().take() {
                overlay.remove_overlay(&layer);
                if let Some(root) = overlay.child().and_downcast::<super::blur::BlurBin>() {
                    root.set_blurred(false);
                }
            }
            let restored_focus = restored_focus.clone();
            let closed_pending = closed_pending.clone();
            let weak_more = weak_more.clone();
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
                more.set_visible(closed_hints.get());
            });
        });
        let status_widgets: Rc<RefCell<Vec<gtk::Widget>>> = Rc::new(RefCell::new(vec![
            paste.clone().upcast::<gtk::Widget>(),
            count.clone().upcast(),
            more.clone().upcast(),
            tag.clone().upcast(),
            chord.clone().upcast(),
            chord_hint.clone().upcast(),
            visual.clone().upcast(),
            filter.clone().upcast(),
            current.root.clone().upcast(),
            prompt.bar.clone().upcast(),
            feedback.clone().upcast(),
        ]));
        for widget in status_widgets.borrow().iter() {
            watch_status_widget(widget, &status_widgets, &root);
        }
        let footer = Self {
            root,
            status,
            paste,
            count,
            show_hints,
            pending_popup,
            more,
            popover,
            reference,
            categories,
            sidebar,
            search,
            selected_category,
            active_area,
            scroll,
            focus_before,
            status_widgets,
            tag,
            feedback,
            feedback_epoch: Rc::new(Cell::new(0)),
            prompt,
            chords,
            visual,
            filter,
            current,
            filter_source: Rc::new(RefCell::new(None)),
            filter_retry: Rc::new(Cell::new(false)),
            observed: Rc::new(RefCell::new(std::rc::Weak::new())),
            view_mode,
        };
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let shortcuts = footer.clone();
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            shortcuts
                .handle_key(key, modifiers)
                .unwrap_or(glib::Propagation::Proceed)
        });
        footer.popover.add_controller(keys);
        footer.close_prompt_on_focus_loss();
        footer.set_mode(mode);
        footer
    }

    /// Focus moving anywhere but the shortcut reference (a clicked row, another
    /// pane) discards the prompt, so its text never reaches a browsing command.
    fn close_prompt_on_focus_loss(&self) {
        let focus = gtk::EventControllerFocus::new();
        let prompt = self.prompt.downgrade();
        let popover = self.popover.downgrade();
        let pending_popup = self.pending_popup.clone();
        focus.connect_leave(move |_| {
            let prompt = prompt.clone();
            let popover = popover.clone();
            let pending_popup = pending_popup.clone();
            // Focus has not settled on its new owner while ::leave runs.
            glib::idle_add_local_once(move || {
                let reference_open = pending_popup.get()
                    || popover
                        .upgrade()
                        .is_some_and(|popover| popover.is_visible());
                if let Some(prompt) = prompt.upgrade()
                    && prompt.bar.is_visible()
                    && !prompt.has_focus()
                    && !reference_open
                {
                    prompt.close();
                }
            });
        });
        self.prompt.entry.add_controller(focus);
    }

    pub fn widget(&self) -> &gtk::Stack {
        &self.root
    }

    pub fn set_activity(&self, widget: &impl IsA<gtk::Widget>) {
        let widget = widget.as_ref().clone();
        self.status.insert_child_after(&widget, Some(&self.count));
        self.status_widgets.borrow_mut().push(widget.clone());
        watch_status_widget(&widget, &self.status_widgets, &self.root);
    }

    pub fn bind_preferences(&self, manager: &super::preferences::PreferenceManager) {
        let tag = self.tag.downgrade();
        let reference = self.reference.downgrade();
        let categories = self.categories.downgrade();
        let search = self.search.downgrade();
        let selected = self.selected_category.clone();
        let scroll = self.scroll.downgrade();
        let popover = self.popover.downgrade();
        let view_mode = self.view_mode.clone();
        let feedback = self.feedback.downgrade();
        let prompt = self.prompt.downgrade();
        let chords = self.chords.clone();
        let primed = Rc::new(Cell::new(false));
        manager.bind_preference(
            &self.root,
            super::preferences::PreferenceManager::tenxer_mode,
            move |_, enabled| {
                let Some(tag) = tag.upgrade() else {
                    return;
                };
                let Some(reference) = reference.upgrade() else {
                    return;
                };
                let starting = !primed.replace(true);
                apply_experimental_label(&tag, enabled);
                if let Some(popover) = popover.upgrade() {
                    if enabled {
                        popover.add_css_class("tenxer-active");
                    } else {
                        popover.remove_css_class("tenxer-active");
                    }
                }
                if !starting
                    && !enabled
                    && let Some(feedback) = feedback.upgrade()
                {
                    feedback.set_text("");
                    feedback.set_visible(false);
                    if let Some(prompt) = prompt.upgrade() {
                        prompt.close();
                    }
                    chords.set(None);
                }
                if let (Some(categories), Some(search), Some(scroll)) =
                    (categories.upgrade(), search.upgrade(), scroll.upgrade())
                {
                    rebuild_reference(
                        &reference,
                        &categories,
                        &search,
                        &scroll,
                        &selected,
                        &view_mode,
                    );
                }
            },
        );
        let show_hints = self.show_hints.clone();
        let pending = self.pending_popup.clone();
        let weak_popover = self.popover.downgrade();
        let more = self.more.downgrade();
        manager.on_keybinding_hints_changed(&self.root, move |_, enabled| {
            show_hints.set(enabled);
            if !enabled {
                pending.set(false);
            }
            if let Some(more) = more.upgrade() {
                more.set_visible(
                    enabled
                        || weak_popover
                            .upgrade()
                            .is_some_and(|popover| popover.is_visible()),
                );
            }
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

    pub fn observe_browser(&self, browser: &Rc<crate::app::Browser>) {
        self.observed.replace(Rc::downgrade(browser));
        self.refresh_filter();
        let weak_browser = Rc::downgrade(browser);
        let footer = self.clone();
        browser.observe(move |event| {
            if matches!(
                event,
                crate::app::BrowserEvent::NavigationStarting
                    | crate::app::BrowserEvent::SelectionSetChanged { .. }
            ) {
                footer.clear_feedback();
            }
            if weak_browser.upgrade().is_some() {
                footer.refresh_filter();
            }
        });
    }

    /// Refreshes once on idle, after focus settles on the cursor row.
    pub(in crate::ui) fn schedule_filter_refresh(&self) {
        if self.filter_retry.replace(true) {
            return;
        }
        let footer = self.clone();
        glib::idle_add_local_once(move || {
            footer.filter_retry.set(false);
            footer.refresh_filter();
        });
    }

    /// Reports `source`'s filter in place of the directory count while one is
    /// active. Call [`Self::refresh_filter`] when it changes.
    pub(in crate::ui) fn observe_filter(
        &self,
        source: impl Fn() -> Option<Option<FilterStatus>> + 'static,
    ) {
        self.filter_source.replace(Some(Rc::new(source)));
        self.refresh_filter();
    }

    pub(in crate::ui) fn refresh_filter(&self) {
        let source = self.filter_source.borrow().clone();
        let status = match source {
            Some(source) => match source() {
                Some(status) => status,
                None => {
                    self.schedule_filter_refresh();
                    return;
                }
            },
            None => None,
        };
        match status.as_ref() {
            Some(status) => {
                let label = if status.search { "search" } else { "filter" };
                self.filter.set_text(&format!("{label}: {}", status.query));
                self.filter.set_tooltip_text(Some(&status.query));
                self.filter.set_visible(true);
            }
            None => self.filter.set_visible(false),
        }
        self.current
            .set(status.as_ref().and_then(|status| status.current.as_deref()));
        let browser = self.observed.borrow().upgrade();
        if let Some(browser) = browser {
            update_item_count(&self.count, &browser, status.as_ref());
            // A range over results is theirs; the hidden directory's never shows.
            let visual = match status.as_ref() {
                Some(status) if status.visual.is_some() => status.visual,
                _ => browser.visual_kind(),
            };
            update_visual_mode(&self.visual, visual);
        }
    }

    #[cfg(test)]
    pub(in crate::ui) fn tag_visible(&self) -> bool {
        self.tag.is_visible()
    }

    pub fn set_mode(&self, mode: BrowserMode) {
        self.view_mode.set(mode);
        rebuild_reference(
            &self.reference,
            &self.categories,
            &self.search,
            &self.scroll,
            &self.selected_category,
            &self.view_mode,
        );
    }

    pub(in crate::ui) fn show_feedback(&self, text: &str) {
        let epoch = self.feedback_epoch.get().wrapping_add(1);
        self.feedback_epoch.set(epoch);
        self.feedback.set_text(text);
        let visible = !text.is_empty();
        self.feedback.set_visible(visible);
        if !visible {
            return;
        }
        let epochs = self.feedback_epoch.clone();
        let feedback = self.feedback.downgrade();
        glib::timeout_add_local_once(FEEDBACK_FLASH, move || {
            if epochs.get() != epoch {
                return;
            }
            if let Some(feedback) = feedback.upgrade() {
                feedback.set_text("");
                feedback.set_visible(false);
            }
        });
    }

    pub(in crate::ui) fn clear_feedback(&self) {
        self.feedback_epoch
            .set(self.feedback_epoch.get().wrapping_add(1));
        self.feedback.set_text("");
        self.feedback.set_visible(false);
    }

    #[cfg(test)]
    pub(in crate::ui) fn feedback_text(&self) -> String {
        self.feedback.text().to_string()
    }

    #[cfg(test)]
    pub(in crate::ui) fn dismiss_feedback(&self) {
        self.clear_feedback();
    }

    pub(in crate::ui) fn open_prompt(&self, kind: Prompt) -> bool {
        self.open_prompt_with(kind, "")
    }

    pub(in crate::ui) fn open_prompt_with(&self, kind: Prompt, text: &str) -> bool {
        // An armed chord must stay visible, and the prompt covers its mark.
        self.chords.set(None);
        self.prompt.open(&self.root, kind, text)
    }

    /// Runs `listener` as the open prompt's text changes, not when a prompt
    /// opens empty or closes.
    pub(in crate::ui) fn connect_prompt_changed(
        &self,
        listener: impl Fn(Prompt, String) + 'static,
    ) {
        let kind = self.prompt.kind.clone();
        self.prompt.entry.connect_changed(move |entry| {
            if let Some(kind) = kind.get() {
                listener(kind, entry.text().to_string());
            }
        });
    }

    #[cfg(test)]
    pub(in crate::ui) fn filter_mark(&self) -> Option<String> {
        self.filter
            .is_visible()
            .then(|| self.filter.text().to_string())
    }

    #[cfg(test)]
    pub(in crate::ui) fn current_hit(&self) -> Option<(String, String)> {
        let current = &self.current;
        current.root.is_visible().then(|| {
            let folder = if current.folder.is_visible() {
                current.folder.text().to_string()
            } else {
                String::new()
            };
            (
                format!("{folder}{}", current.name.text()),
                current.root.tooltip_text().unwrap_or_default().to_string(),
            )
        })
    }

    #[cfg(test)]
    pub(in crate::ui) fn count_text(&self) -> (String, String) {
        (
            self.count.text().to_string(),
            self.count.tooltip_text().unwrap_or_default().to_string(),
        )
    }

    #[cfg(test)]
    pub(in crate::ui) fn prompt(&self) -> &gtk::Entry {
        &self.prompt.entry
    }

    pub(in crate::ui) fn open_prompt_kind(&self) -> Option<Prompt> {
        self.prompt.kind.get()
    }

    pub(in crate::ui) fn prompt_text(&self) -> String {
        self.prompt.entry.text().to_string()
    }

    #[cfg(test)]
    pub(in crate::ui) fn prompt_label(&self) -> Option<String> {
        (self.root.visible_child().as_ref() == Some(self.prompt.bar.upcast_ref()))
            .then(|| self.prompt.label.text().to_string())
    }

    pub(in crate::ui) fn dismiss_prompt(&self) {
        self.prompt.close();
    }

    pub(in crate::ui) fn arm_chord(&self, chord: Chord) {
        self.chords.set(Some(chord));
    }

    pub(in crate::ui) fn armed_chord(&self) -> Option<Chord> {
        self.chords.armed.get()
    }

    pub(in crate::ui) fn cancel_chord(&self) {
        self.chords.set(None);
    }

    pub(in crate::ui) fn connect_chord_changed(&self, listener: impl Fn(Option<Chord>) + 'static) {
        self.chords.listeners.borrow_mut().push(Box::new(listener));
    }

    #[cfg(test)]
    pub(in crate::ui) fn chord(&self) -> gtk::Label {
        self.chords.mark.upgrade().expect("chord mark")
    }

    #[cfg(test)]
    pub(in crate::ui) fn chord_hint(&self) -> Option<String> {
        let hint = self.chords.hint.upgrade()?;
        hint.is_visible().then(|| hint.text().to_string())
    }

    pub(in crate::ui) fn prompt_is_visible(&self) -> bool {
        self.prompt.bar.is_visible()
    }

    pub(in crate::ui) fn prompt_has_focus(&self) -> bool {
        self.prompt.has_focus()
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
        let reference_open = self.popover.is_visible() || self.pending_popup.get();
        let f1 = key == gdk::Key::F1
            && !command_modifiers
            && !modifiers.contains(gdk::ModifierType::SHIFT_MASK);
        let tilde = self.tilde_toggles(key, modifiers, reference_open);
        if self.prompt_has_focus() && !f1 && !tilde && !reference_open {
            return None;
        }
        if f1 || tilde {
            if self.popover.is_visible() || self.pending_popup.replace(false) {
                if self.popover.is_visible() {
                    self.more.popdown();
                } else {
                    self.focus_before.take();
                    self.more.set_visible(self.show_hints.get());
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
                if self.more.is_mapped() && self.more.width() > 0 {
                    self.more.popup();
                } else {
                    self.pending_popup.set(true);
                    self.more.set_visible(true);
                    let pending = self.pending_popup.clone();
                    let weak_more = self.more.downgrade();
                    // A hidden shortcut button needs an allocation before positioning the popover.
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
                self.more.set_visible(self.show_hints.get());
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
        let keys = modifiers
            & (gdk::ModifierType::CONTROL_MASK
                | gdk::ModifierType::ALT_MASK
                | gdk::ModifierType::SUPER_MASK
                | gdk::ModifierType::SHIFT_MASK);
        if keys == gdk::ModifierType::CONTROL_MASK {
            let focused = match key {
                gdk::Key::b => self.focus_category(),
                gdk::Key::f => self.focus_search(),
                gdk::Key::l => self.focus_results(),
                _ => false,
            };
            if focused {
                return Some(glib::Propagation::Stop);
            }
        }
        let focus = self.search.root().and_then(|root| root.focus());
        let search_focused = focus.as_ref().is_some_and(|focus| {
            focus == self.search.upcast_ref::<gtk::Widget>() || focus.is_ancestor(&self.search)
        });
        let category_focused = focus.as_ref().is_some_and(|focus| {
            focus == self.sidebar.upcast_ref::<gtk::Widget>() || focus.is_ancestor(&self.sidebar)
        });
        let list_focused = focus.as_ref().is_some_and(|focus| {
            focus == self.scroll.upcast_ref::<gtk::Widget>() || focus.is_ancestor(&self.scroll)
        });
        if matches!(key, gdk::Key::Tab | gdk::Key::ISO_Left_Tab) && !command_modifiers {
            let backwards =
                keys.contains(gdk::ModifierType::SHIFT_MASK) || key == gdk::Key::ISO_Left_Tab;
            let current = if search_focused {
                ReferenceArea::Search
            } else if category_focused {
                ReferenceArea::Categories
            } else if list_focused {
                ReferenceArea::Results
            } else {
                self.active_area.get()
            };
            match (current, backwards) {
                (ReferenceArea::Search, false) | (ReferenceArea::Results, true) => {
                    self.focus_category();
                }
                (ReferenceArea::Categories, false) | (ReferenceArea::Search, true) => {
                    self.focus_results();
                }
                (ReferenceArea::Results, false) | (ReferenceArea::Categories, true) => {
                    self.focus_search();
                }
            }
            return Some(glib::Propagation::Stop);
        }
        if !command_modifiers && category_focused {
            let direction = match key {
                gdk::Key::Up
                | gdk::Key::KP_Up
                | gdk::Key::Left
                | gdk::Key::KP_Left
                | gdk::Key::k
                | gdk::Key::h => Some(-1),
                gdk::Key::Down
                | gdk::Key::KP_Down
                | gdk::Key::Right
                | gdk::Key::KP_Right
                | gdk::Key::j
                | gdk::Key::l => Some(1),
                gdk::Key::Home => Some(-100),
                gdk::Key::End => Some(100),
                _ => None,
            };
            if let Some(direction) = direction {
                self.move_category(direction);
                return Some(glib::Propagation::Stop);
            }
            if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space) {
                return Some(glib::Propagation::Proceed);
            }
        }
        if !command_modifiers
            && ((list_focused)
                || (search_focused
                    && matches!(
                        key,
                        gdk::Key::Up | gdk::Key::Down | gdk::Key::Page_Up | gdk::Key::Page_Down
                    )))
            && self.scroll_reference(key)
        {
            return Some(glib::Propagation::Stop);
        }
        if search_focused
            && (!command_modifiers
                || (keys == gdk::ModifierType::CONTROL_MASK
                    && matches!(key, gdk::Key::a | gdk::Key::c | gdk::Key::v | gdk::Key::x)))
        {
            return Some(glib::Propagation::Proceed);
        }
        // No browsing shortcut may operate on files while the reference is open.
        Some(glib::Propagation::Stop)
    }

    fn category_buttons(&self) -> Vec<gtk::Button> {
        category_buttons(&self.categories)
    }

    fn focus_search(&self) -> bool {
        let focused = self.search.grab_focus();
        if focused {
            self.active_area.set(ReferenceArea::Search);
        }
        focused
    }

    fn focus_results(&self) -> bool {
        let focused = self.scroll.grab_focus();
        if focused {
            self.active_area.set(ReferenceArea::Results);
        }
        focused
    }

    fn focus_category(&self) -> bool {
        let focused = self
            .category_buttons()
            .into_iter()
            .find(|button| button.has_css_class("selected"))
            .is_some_and(|button| button.grab_focus());
        if focused {
            self.active_area.set(ReferenceArea::Categories);
        }
        focused
    }

    fn move_category(&self, delta: isize) {
        let buttons = self.category_buttons();
        let Some(index) = buttons
            .iter()
            .position(|button| button.has_focus())
            .or_else(|| {
                buttons
                    .iter()
                    .position(|button| button.has_css_class("selected"))
            })
        else {
            return;
        };
        let next = (index as isize + delta).clamp(0, buttons.len() as isize - 1) as usize;
        buttons[next].grab_focus();
        buttons[next].emit_clicked();
    }

    fn scroll_reference(&self, key: gdk::Key) -> bool {
        let adjustment = self.scroll.vadjustment();
        let page = adjustment.page_size().max(1.0);
        let step = if adjustment.step_increment() >= 1.0 {
            adjustment.step_increment()
        } else {
            page / 10.0
        };
        let page_step = if adjustment.page_increment() >= 1.0 {
            adjustment.page_increment()
        } else {
            page
        };
        let delta = match key {
            gdk::Key::Up | gdk::Key::KP_Up | gdk::Key::Left | gdk::Key::KP_Left | gdk::Key::k => {
                -step
            }
            gdk::Key::Down
            | gdk::Key::KP_Down
            | gdk::Key::Right
            | gdk::Key::KP_Right
            | gdk::Key::j => step,
            gdk::Key::Page_Up | gdk::Key::KP_Page_Up => -page_step,
            gdk::Key::Page_Down | gdk::Key::KP_Page_Down => page_step,
            gdk::Key::Home => {
                adjustment.set_value(adjustment.lower());
                return true;
            }
            gdk::Key::End => {
                adjustment.set_value((adjustment.upper() - page).max(adjustment.lower()));
                return true;
            }
            _ => return false,
        };
        let limit = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
        adjustment.set_value((adjustment.value() + delta).clamp(adjustment.lower(), limit));
        true
    }

    fn tilde_toggles(
        &self,
        key: gdk::Key,
        modifiers: gdk::ModifierType,
        reference_open: bool,
    ) -> bool {
        if !super::preferences::PreferenceManager::shared().tenxer_mode() {
            return false;
        }
        if modifiers.intersects(
            gdk::ModifierType::CONTROL_MASK
                | gdk::ModifierType::ALT_MASK
                | gdk::ModifierType::SUPER_MASK,
        ) {
            return false;
        }
        let tilde = key == gdk::Key::asciitilde
            || (key == gdk::Key::grave && modifiers.contains(gdk::ModifierType::SHIFT_MASK));
        tilde && (reference_open || !self.prompt_has_focus())
    }
}

fn category_buttons(categories: &gtk::Box) -> Vec<gtk::Button> {
    let mut buttons = Vec::new();
    let mut child = categories.first_child();
    while let Some(row) = child {
        child = row.next_sibling();
        let mut item = row.first_child();
        while let Some(widget) = item {
            item = widget.next_sibling();
            if let Ok(button) = widget.downcast::<gtk::Button>() {
                buttons.push(button);
            }
        }
    }
    buttons
}

fn reflow_categories(categories: &gtk::Box, compact: bool, width: i32) {
    let buttons = category_buttons(categories);
    let focused = buttons.iter().position(|button| button.has_focus());
    for button in &buttons {
        if let Some(row) = button.parent().and_downcast::<gtk::Box>() {
            row.remove(button);
        }
    }
    while let Some(row) = categories.first_child() {
        categories.remove(&row);
    }
    let mut row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_vexpand(false);
    categories.append(&row);
    let mut occupied = 0;
    for button in &buttons {
        let natural = button.measure(gtk::Orientation::Horizontal, -1).1;
        let needed = if occupied == 0 {
            natural
        } else {
            occupied + 8 + natural
        };
        if occupied > 0 && (!compact || needed > width) {
            row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.set_vexpand(false);
            categories.append(&row);
            occupied = 0;
        } else {
            occupied = if occupied == 0 { natural } else { needed };
        }
        row.append(button);
        if occupied == 0 {
            occupied = natural;
        }
    }
    if let Some(index) = focused {
        buttons[index].grab_focus();
    }
}

fn rebuild_reference(
    reference: &gtk::Box,
    categories: &gtk::Box,
    search: &gtk::Entry,
    scroll: &gtk::ScrolledWindow,
    selected: &Rc<RefCell<String>>,
    mode: &Rc<Cell<BrowserMode>>,
) {
    while let Some(child) = categories.first_child() {
        categories.remove(&child);
    }
    let sections = super::shortcut_reference::reference_sections(mode.get());
    if !sections
        .iter()
        .any(|section| section.title == selected.borrow().as_str())
    {
        *selected.borrow_mut() = String::from("All");
    }
    let total: usize = sections.iter().map(|section| section.rows.len()).sum();
    for (name, count) in std::iter::once(("All", total)).chain(
        sections
            .iter()
            .map(|section| (section.title, section.rows.len())),
    ) {
        let button = gtk::Button::new();
        button.add_css_class("shortcut-reference-category");
        let compact = categories.has_css_class("compact");
        if compact {
            button.add_css_class("action-library-category");
        }
        let is_selected = *selected.borrow() == name;
        if is_selected {
            button.add_css_class("selected");
        }
        button.update_state(&[gtk::accessible::State::Selected(Some(is_selected))]);
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let label = gtk::Label::new(Some(name));
        label.set_hexpand(!compact);
        label.set_xalign(0.0);
        line.append(&label);
        let amount = gtk::Label::new(Some(&count.to_string()));
        amount.add_css_class("shortcut-reference-count");
        amount.set_visible(!compact);
        line.append(&amount);
        button.set_child(Some(&line));
        button.set_hexpand(!compact);
        button.set_halign(if compact {
            gtk::Align::Start
        } else {
            gtk::Align::Fill
        });
        let reference = reference.downgrade();
        let weak_categories = categories.downgrade();
        let search = search.downgrade();
        let scroll = scroll.downgrade();
        let selected = selected.clone();
        let mode = mode.clone();
        button.connect_clicked(move |button| {
            *selected.borrow_mut() = name.to_owned();
            if let (Some(reference), Some(categories), Some(search), Some(scroll)) = (
                reference.upgrade(),
                weak_categories.upgrade(),
                search.upgrade(),
                scroll.upgrade(),
            ) {
                for category in category_buttons(&categories) {
                    category.remove_css_class("selected");
                    category.update_state(&[gtk::accessible::State::Selected(Some(false))]);
                }
                button.add_css_class("selected");
                button.update_state(&[gtk::accessible::State::Selected(Some(true))]);
                render_reference(
                    &reference,
                    mode.get(),
                    name,
                    &search.text(),
                    reference.has_css_class("compact"),
                );
                scroll.vadjustment().set_value(0.0);
            }
        });
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.set_vexpand(false);
        row.append(&button);
        categories.append(&row);
    }
    if categories.has_css_class("compact") {
        let width = categories
            .root()
            .and_downcast::<gtk::Window>()
            .map_or(400, |window| (window.width() - 104).max(1));
        reflow_categories(categories, true, width);
    }
    render_reference(
        reference,
        mode.get(),
        &selected.borrow(),
        &search.text(),
        reference.has_css_class("compact"),
    );
}

fn render_reference(
    reference: &gtk::Box,
    mode: BrowserMode,
    selected: &str,
    query: &str,
    compact: bool,
) {
    while let Some(child) = reference.first_child() {
        reference.remove(&child);
    }
    let query = query.trim().to_lowercase();
    let mut found = false;
    for section in super::shortcut_reference::reference_sections(mode) {
        if selected != "All" && selected != section.title {
            continue;
        }
        let rows: Vec<_> = section
            .rows
            .into_iter()
            .filter(|(key, action)| {
                query.is_empty()
                    || key.to_lowercase().contains(&query)
                    || action.to_lowercase().contains(&query)
            })
            .collect();
        if !rows.is_empty() {
            found = true;
            append_section(reference, section.title, &rows, compact);
        }
    }
    if !found {
        let empty = gtk::Label::new(Some("No matching shortcuts"));
        empty.add_css_class("shortcut-reference-note");
        reference.append(&empty);
    }
}

/// Keep the experimental caveat in the pill's tooltip and accessible description.
fn apply_experimental_label(tag: &gtk::Label, enabled: bool) {
    tag.set_text(crate::ui::tenxer_mode::TAG_TEXT);
    tag.set_visible(enabled);
    let phrase = super::shortcut_reference::EXPERIMENTAL_LABEL;
    let announced = if enabled {
        format!("{} {phrase}", crate::ui::tenxer_mode::TAG_NAME)
    } else {
        crate::ui::tenxer_mode::TAG_NAME.to_owned()
    };
    tag.set_tooltip_text(Some(&announced));
    tag.update_property(&[
        gtk::accessible::Property::Label(&announced),
        gtk::accessible::Property::Description(if enabled { phrase } else { "" }),
    ]);
}

const FEEDBACK_FLASH: Duration = Duration::from_millis(2_000);

fn update_visual_mode(label: &gtk::Label, visual: Option<crate::app::VisualKind>) {
    let (text, name) = match visual {
        Some(crate::app::VisualKind::Select) => ("VISUAL", "Visual select"),
        Some(crate::app::VisualKind::Unset) => ("UNSET", "Visual unset"),
        None => ("", ""),
    };
    if label.text() != text {
        label.set_text(text);
        super::accessibility::set_label(label, name);
    }
    label.set_visible(!text.is_empty());
}

#[cfg(test)]
impl ShortcutFooter {
    pub(in crate::ui) fn visual_text(&self) -> Option<String> {
        self.visual
            .is_visible()
            .then(|| self.visual.text().to_string())
    }
}

fn update_item_count(
    label: &gtk::Label,
    browser: &Rc<crate::app::Browser>,
    filter: Option<&FilterStatus>,
) {
    if let Some(filter) = filter {
        let noun = if filter.total() == 1 { "item" } else { "items" };
        label.set_label(&format!("{} {noun}", filter.total()));
        label.set_tooltip_text(Some(&file_folder_breakdown(filter.files, filter.folders)));
        label.set_visible(true);
        return;
    }
    let Some(depth) = browser.active_depth() else {
        label.set_visible(false);
        return;
    };
    let counts = browser.column_entry_counts(depth).unwrap_or_default();
    let selected = browser.selected_entries();
    for position in browser.selected_positions(depth) {
        if let Some(entry) = browser.entry_at(depth, position)
            && !entry.is_directory()
            && entry.size == crate::model::MetadataValue::Unknown
        {
            browser.request_metadata_fill(depth, position, entry.location, false);
        }
    }
    let noun = if counts.total == 1 { "item" } else { "items" };
    if !selected.is_empty() {
        label.set_label(&selection_details(&selected));
        label.set_tooltip_text(Some(&format!(
            "{} of {} {noun} selected. Size includes selected files only; folder contents are not counted.",
            selected.len(), counts.total
        )));
    } else {
        label.set_label(&format!("{} {noun}", counts.total));
        label.set_tooltip_text(Some(&file_folder_breakdown(counts.files, counts.folders)));
    }
    label.set_visible(true);
}

fn file_folder_breakdown(files: usize, folders: usize) -> String {
    let file_noun = if files == 1 { "file" } else { "files" };
    let folder_noun = if folders == 1 { "folder" } else { "folders" };
    format!("{files} {file_noun}, {folders} {folder_noun}")
}

fn selection_details(entries: &[crate::model::FileEntry]) -> String {
    let folders = entries.iter().filter(|entry| entry.is_directory()).count();
    let files = entries.len() - folders;
    let mut parts = Vec::new();
    if folders > 0 {
        let noun = if folders == 1 { "folder" } else { "folders" };
        parts.push(format!("{folders} {noun}"));
    }
    if files > 0 {
        let noun = if files == 1 { "file" } else { "files" };
        parts.push(format!("{files} {noun}"));
    }
    let mut text = format!("{} selected", parts.join(", "));
    if files > 0 {
        let mut bytes = 0u64;
        let mut known = 0;
        for entry in entries.iter().filter(|entry| !entry.is_directory()) {
            if let crate::model::MetadataValue::Known(size) = entry.size {
                bytes = bytes.saturating_add(size);
                known += 1;
            }
        }
        let size = super::browser::format_file_size(bytes);
        if known == files {
            text.push_str(&format!(" ({size})"));
        } else if known > 0 {
            text.push_str(&format!(" ({size} known; size incomplete)"));
        } else {
            text.push_str(" (size unavailable)");
        }
    }
    text
}

fn watch_status_widget(
    widget: &gtk::Widget,
    status_widgets: &Rc<RefCell<Vec<gtk::Widget>>>,
    root: &gtk::Stack,
) {
    let root = root.downgrade();
    let statuses = status_widgets.clone();
    widget.connect_visible_notify(move |_| {
        // Ignore ancestor visibility so a hidden footer can reveal itself.
        if let Some(root) = root.upgrade() {
            root.set_visible(
                statuses
                    .borrow()
                    .iter()
                    .any(gtk::prelude::WidgetExt::get_visible),
            );
        }
    });
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

fn append_section(parent: &gtk::Box, title: &str, shortcuts: &[Shortcut], compact: bool) {
    let section = gtk::Box::new(gtk::Orientation::Vertical, 16);
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let label = gtk::Label::new(Some(&title.to_uppercase()));
    label.add_css_class("shortcut-reference-heading");
    heading.append(&label);
    let count = gtk::Label::new(Some(&shortcuts.len().to_string()));
    count.add_css_class("shortcut-reference-count");
    heading.append(&count);
    let divider = gtk::Separator::new(gtk::Orientation::Horizontal);
    divider.set_hexpand(true);
    divider.set_valign(gtk::Align::Center);
    heading.append(&divider);
    section.append(&heading);
    let grid = gtk::Grid::new();
    grid.set_column_spacing(48);
    grid.set_row_spacing(12);
    for (index, (key, action)) in shortcuts.iter().enumerate() {
        let row = gtk::Box::new(
            if compact {
                gtk::Orientation::Vertical
            } else {
                gtk::Orientation::Horizontal
            },
            6,
        );
        row.add_css_class("shortcut-reference-row");
        let action = gtk::Label::builder()
            .label(*action)
            .xalign(0.0)
            .hexpand(true)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .build();
        action.add_css_class("shortcut-reference-description");
        if compact {
            action.set_max_width_chars(22);
        }
        row.append(&action);
        let keys = gtk::Label::new(Some(key));
        keys.add_css_class("shortcut-reference-key");
        keys.set_halign(gtk::Align::Start);
        keys.set_wrap(compact);
        keys.set_max_width_chars(if compact { 22 } else { 32 });
        row.append(&keys);
        grid.attach(
            &row,
            if compact { 0 } else { (index % 2) as i32 },
            if compact {
                index as i32
            } else {
                (index / 2) as i32
            },
            1,
            1,
        );
    }
    section.append(&grid);
    parent.append(&section);
}

#[cfg(test)]
mod tests;
