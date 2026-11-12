// SPDX-License-Identifier: MIT

//! A tab's optional second pane, Nemo's extra pane: two browser views side by
//! side, one of them active. The tab's chrome acts on the active pane through
//! its `PaneTarget`, and each pane keys its own footer and dispatcher.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{glib, prelude::*};

use crate::{
    app::Presence,
    model::{FileEntry, Location},
    ui::browser::{BrowserView, OtherPane, PaneTarget, paths::is_trash_location},
};

use super::{super::keyboard::PaneInput, layout::FooterBinding};

#[cfg(test)]
mod tests;

/// One of a tab's two panes. The main pane always exists; the extra pane is
/// created the first time it opens and hidden when it closes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum Side {
    #[default]
    Main,
    Extra,
}

impl Side {
    fn other(self) -> Self {
        match self {
            Self::Main => Self::Extra,
            Self::Extra => Self::Main,
        }
    }
}

/// Whether the extra pane shows and which pane is active.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct PaneLayout {
    split: bool,
    active: Side,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Toggle {
    /// The extra pane opens on the active pane's folder and becomes active.
    Opened,
    /// The extra pane closes. When it was active, the main pane takes over
    /// its folder, so the folder being worked in stays.
    Closed { adopt: bool },
}

impl PaneLayout {
    pub(super) fn split(self) -> bool {
        self.split
    }

    pub(super) fn active(self) -> Side {
        self.active
    }

    /// The pane that is not active, while both show.
    pub(super) fn other(self) -> Option<Side> {
        self.split.then(|| self.active.other())
    }

    pub(super) fn toggle(&mut self) -> Toggle {
        if self.split {
            let adopt = self.active == Side::Extra;
            *self = Self::default();
            Toggle::Closed { adopt }
        } else {
            *self = Self {
                split: true,
                active: Side::Extra,
            };
            Toggle::Opened
        }
    }

    /// Makes `side` active; false when it already is or does not show.
    pub(super) fn activate(&mut self, side: Side) -> bool {
        if self.active == side || (side == Side::Extra && !self.split) {
            return false;
        }
        self.active = side;
        true
    }

    pub(super) fn focus_other(&mut self) -> bool {
        self.other().is_some_and(|other| self.activate(other))
    }
}

/// Where Copy and Move to Other Pane put the selection: the other pane's
/// folder, unless no other pane shows or its folder cannot take files.
pub(super) fn transfer_destination(
    split: bool,
    other: Option<Location>,
) -> Result<Location, &'static str> {
    if !split {
        return Err("Open the other pane first");
    }
    let destination = other.ok_or("The other pane has no folder")?;
    if is_trash_location(&destination) || destination.is_recent_location() {
        return Err("Can\u{2019}t copy or move items into this folder");
    }
    Ok(destination)
}

pub(super) struct Pane {
    pub(super) view: BrowserView,
    pub(super) footer: FooterBinding,
    pub(super) input: PaneInput,
    pub(super) terminal: Rc<super::terminal::PaneTerminal>,
    shell: gtk::Box,
    location_bar: gtk::Box,
}

impl Pane {
    pub(super) fn new(view: BrowserView, footer: FooterBinding) -> Self {
        let shell = gtk::Box::new(gtk::Orientation::Vertical, 0);
        shell.add_css_class("browser-pane");
        shell.set_hexpand(true);
        shell.set_vexpand(true);
        let location_bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        location_bar.add_css_class("pane-location");
        location_bar.set_visible(false);
        let terminal = super::terminal::PaneTerminal::new(&view);
        location_bar.append(&terminal.button);
        shell.append(&location_bar);
        shell.append(&terminal.split(&view.widget()));
        Self {
            view,
            footer,
            input: Rc::new(Cell::new(true)),
            terminal,
            shell,
            location_bar,
        }
    }

    fn sync_location_bar(&self, split: bool, locations: &gtk::Stack) {
        let widget = self.view.location_widget();
        if split && widget.parent().as_ref() == Some(locations.upcast_ref()) {
            locations.remove(&widget);
            self.location_bar.prepend(&widget);
        } else if !split && widget.parent().as_ref() == Some(self.location_bar.upcast_ref()) {
            self.location_bar.remove(&widget);
            locations.add_child(&widget);
        }
        self.location_bar.set_visible(split);
    }

    fn location(&self) -> Option<Location> {
        self.view.browser().active_location()
    }

    fn focus_listing(&self) {
        self.view.browser().focus_active();
    }

    fn contains_focus(&self) -> bool {
        self.shell
            .root()
            .and_then(|root| root.focus())
            .is_some_and(|focus| focus.is_ancestor(&self.shell))
    }
}

type LocationListener = Rc<dyn Fn(Option<&Location>)>;

pub(super) struct Panes {
    layout: Cell<PaneLayout>,
    main: Rc<Pane>,
    extra: RefCell<Option<Rc<Pane>>>,
    paned: gtk::Paned,
    locations: gtk::Stack,
    footers: gtk::Stack,
    target: PaneTarget,
    presence: Cell<(Presence, bool)>,
    location_listeners: RefCell<Vec<LocationListener>>,
}

impl Panes {
    pub(super) fn new(
        main: Pane,
        target: PaneTarget,
        locations: &gtk::Stack,
        footers: &gtk::Stack,
    ) -> Rc<Self> {
        let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
        paned.add_css_class("dual-pane");
        paned.set_wide_handle(true);
        paned.set_resize_start_child(true);
        paned.set_resize_end_child(true);
        paned.set_shrink_start_child(false);
        paned.set_shrink_end_child(false);
        paned.set_hexpand(true);
        paned.set_vexpand(true);
        paned.set_start_child(Some(&main.shell));
        super::super::sidebar_rail::configure_paned_handle(&paned, "col-resize");
        let panes = Rc::new(Self {
            layout: Cell::new(PaneLayout::default()),
            main: Rc::new(main),
            extra: RefCell::new(None),
            paned,
            locations: locations.clone(),
            footers: footers.clone(),
            target,
            presence: Cell::new((Presence::default(), false)),
            location_listeners: RefCell::new(Vec::new()),
        });
        panes.install_activation(&panes.main, Side::Main);
        panes.follow_location(&panes.main, Side::Main);
        panes
    }

    pub(super) fn widget(&self) -> gtk::Widget {
        self.paned.clone().upcast()
    }

    #[cfg(test)]
    pub(super) fn layout(&self) -> PaneLayout {
        self.layout.get()
    }

    pub(super) fn main(&self) -> &Rc<Pane> {
        &self.main
    }

    pub(super) fn footers(&self) -> gtk::Stack {
        self.footers.clone()
    }

    pub(super) fn is_split(&self) -> bool {
        self.layout.get().split()
    }

    pub(super) fn has_extra(&self) -> bool {
        self.extra.borrow().is_some()
    }

    fn pane(&self, side: Side) -> Option<Rc<Pane>> {
        match side {
            Side::Main => Some(self.main.clone()),
            Side::Extra => self.extra.borrow().clone(),
        }
    }

    pub(super) fn active(&self) -> Rc<Pane> {
        self.pane(self.layout.get().active())
            .unwrap_or_else(|| self.main.clone())
    }

    pub(super) fn active_view(&self) -> BrowserView {
        self.active().view.clone()
    }

    /// Every pane, shown or not.
    pub(super) fn all(&self) -> Vec<Rc<Pane>> {
        std::iter::once(self.main.clone())
            .chain(self.extra.borrow().clone())
            .collect()
    }

    pub(super) fn cancel_chords(&self) {
        for pane in self.all() {
            pane.footer.shortcuts.cancel_chord();
        }
    }

    pub(super) fn has_background_operations(&self) -> bool {
        self.all()
            .iter()
            .any(|pane| pane.view.browser().has_background_operations())
    }

    /// Takes the extra pane, built with its own footer and key dispatcher.
    pub(super) fn attach_extra(self: &Rc<Self>, pane: Pane) {
        let pane = Rc::new(pane);
        pane.shell.set_visible(false);
        pane.input.set(false);
        self.paned.set_end_child(Some(&pane.shell));
        self.locations.add_child(&pane.view.location_widget());
        self.install_activation(&pane, Side::Extra);
        self.follow_location(&pane, Side::Extra);
        self.target.add(&pane.view);
        for (side, view) in [(Side::Main, &self.main.view), (Side::Extra, &pane.view)] {
            view.set_other_pane(Some(self.other_pane_hooks(side)));
        }
        self.extra.replace(Some(pane));
    }

    fn other_pane_hooks(self: &Rc<Self>, side: Side) -> OtherPane {
        let located = Rc::downgrade(self);
        let transferring = Rc::downgrade(self);
        OtherPane {
            location: Rc::new(move || {
                let panes = located.upgrade()?;
                if !panes.layout.get().split() {
                    return None;
                }
                panes.pane(side.other())?.location()
            }),
            transfer: Rc::new(move |entries, moving| {
                if let Some(panes) = transferring.upgrade() {
                    panes.transfer_from(side, Some(entries), moving);
                }
            }),
        }
    }

    pub(super) fn toggle(&self) {
        let Some(extra) = self.extra.borrow().clone() else {
            return;
        };
        let mut layout = self.layout.get();
        match layout.toggle() {
            Toggle::Opened => self.open(&extra, layout),
            Toggle::Closed { adopt } => self.close(&extra, layout, adopt),
        }
    }

    fn open(&self, extra: &Pane, layout: PaneLayout) {
        let source = self.main.view.clone();
        extra.view.set_view_mode(source.view_mode());
        if let Some(location) = source.browser().active_location() {
            extra.view.navigate_location(location);
        }
        for view in [&source, &extra.view] {
            view.set_split_pane(true);
        }
        extra.shell.set_visible(true);
        self.layout.set(layout);
        self.apply();
        center_divider(&self.paned);
        extra.focus_listing();
    }

    fn close(&self, extra: &Pane, layout: PaneLayout, adopt: bool) {
        if adopt {
            self.adopt(extra);
        }
        for view in [&self.main.view, &extra.view] {
            view.set_split_pane(false);
        }
        extra.shell.set_visible(false);
        self.layout.set(layout);
        self.apply();
        self.main.focus_listing();
    }

    /// The main pane takes over the closing extra pane's folder, view and selection.
    fn adopt(&self, extra: &Pane) {
        let main = &self.main.view;
        main.set_view_mode(extra.view.view_mode());
        let Some(location) = extra.location() else {
            return;
        };
        let selected: Vec<_> = extra
            .view
            .transfer_targets(false)
            .unwrap_or_default()
            .into_iter()
            .map(|entry| entry.location)
            .filter(|selected| selected.parent().as_ref() == Some(&location))
            .collect();
        if selected.is_empty() {
            main.navigate_location(location);
        } else {
            main.reveal_locations(location, selected, false);
        }
    }

    /// Activates the pane focus or a click entered. Keys go to it at once; the
    /// rest waits for an idle, since GTK reports focus while views still
    /// hold their state.
    fn activate_from_input(self: &Rc<Self>, side: Side) {
        let mut layout = self.layout.get();
        if !layout.activate(side) {
            return;
        }
        self.layout.set(layout);
        let active = self.active();
        for pane in self.all() {
            pane.input.set(Rc::ptr_eq(&pane, &active));
        }
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(panes) = weak.upgrade() {
                panes.apply();
            }
        });
    }

    pub(super) fn focus_other(&self) {
        let mut layout = self.layout.get();
        if layout.focus_other() {
            self.layout.set(layout);
            self.apply();
            self.active().focus_listing();
        }
    }

    pub(super) fn toggle_terminal(&self) {
        self.active().terminal.toggle();
    }

    /// Shows the current layout: which pane takes keys, highlights, location
    /// bar and footer, and what the tab's chrome acts on.
    fn apply(&self) {
        let layout = self.layout.get();
        let active = self.active();
        for pane in self.all() {
            let is_active = Rc::ptr_eq(&pane, &active);
            pane.input.set(is_active);
            pane.sync_location_bar(layout.split(), &self.locations);
            if is_active && layout.split() {
                pane.shell.add_css_class("active-pane");
            } else {
                pane.shell.remove_css_class("active-pane");
            }
            if !is_active {
                pane.footer.shortcuts.cancel_chord();
            }
        }
        if !layout.split() {
            self.locations
                .set_visible_child(&active.view.location_widget());
        }
        self.footers
            .set_visible_child(active.footer.shortcuts.widget());
        self.target.activate(&active.view);
        self.share_presence();
        self.notify_location();
    }

    pub(super) fn set_presence(&self, presence: Presence, power_saver: bool) {
        self.presence.set((presence, power_saver));
        self.share_presence();
    }

    fn share_presence(&self) {
        let (presence, power_saver) = self.presence.get();
        self.main.view.set_presence(presence, power_saver);
        if let Some(extra) = self.extra.borrow().as_ref() {
            let shown = if self.layout.get().split() {
                presence
            } else {
                Presence::Hidden
            };
            extra.view.set_presence(shown, power_saver);
        }
    }

    /// Calls `listener` with the active pane's folder now and whenever it, or
    /// the active pane, changes; tab labels and the window title follow it.
    pub(super) fn observe_location(&self, listener: impl Fn(Option<&Location>) + 'static) {
        let listener: LocationListener = Rc::new(listener);
        self.location_listeners.borrow_mut().push(listener.clone());
        listener(self.active().view.tab_location().as_ref());
    }

    fn follow_location(self: &Rc<Self>, pane: &Pane, side: Side) {
        let weak = Rc::downgrade(self);
        pane.view.observe_tab_location(move |_| {
            if let Some(panes) = weak.upgrade()
                && panes.layout.get().active() == side
            {
                panes.notify_location();
            }
        });
    }

    fn notify_location(&self) {
        let location = self.active().view.tab_location();
        let listeners = self.location_listeners.borrow().clone();
        for listener in listeners {
            listener(location.as_ref());
        }
    }

    fn install_activation(self: &Rc<Self>, pane: &Pane, side: Side) {
        let focus = gtk::EventControllerFocus::new();
        let weak = Rc::downgrade(self);
        focus.connect_enter(move |_| {
            if let Some(panes) = weak.upgrade() {
                panes.activate_from_input(side);
            }
        });
        pane.shell.add_controller(focus);
        let click = gtk::GestureClick::new();
        click.set_button(0);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        click.connect_pressed(move |_, _, _, _| {
            let Some(panes) = weak.upgrade() else {
                return;
            };
            panes.activate_from_input(side);
            // A click on a caption or blank space focuses nothing, but the
            // active pane's dispatcher reads keys only with its listing focused.
            let weak = Rc::downgrade(&panes);
            glib::idle_add_local_once(move || {
                if let Some(pane) = weak.upgrade().and_then(|panes| panes.pane(side))
                    && !pane.contains_focus()
                    && pane.shell.is_visible()
                {
                    pane.focus_listing();
                }
            });
        });
        pane.shell.add_controller(click);
    }

    /// Copies or moves the active pane's selection into the other pane's folder.
    pub(super) fn transfer_to_other(&self, moving: bool) {
        self.transfer_from(self.layout.get().active(), None, moving);
    }

    /// Transfers `entries`, or else `side`'s selection, into the other pane's
    /// folder, and reports a refusal in `side`'s footer.
    fn transfer_from(&self, side: Side, entries: Option<Vec<FileEntry>>, moving: bool) {
        let Some(source) = self.pane(side) else {
            return;
        };
        let split = self.layout.get().split();
        let other = self.pane(side.other()).and_then(|pane| pane.location());
        let result = transfer_destination(split, other).and_then(|destination| {
            let entries = match entries {
                Some(entries) => entries,
                None => source.view.transfer_targets(moving)?,
            };
            source
                .view
                .transfer_to_location(entries, destination, moving)
        });
        if let Err(reason) = result {
            source
                .footer
                .shortcuts
                .show_feedback(&crate::i18n::tr(reason));
        }
    }
}

/// Splits the room evenly once the paned has its width.
fn center_divider(paned: &gtk::Paned) {
    let paned = paned.downgrade();
    glib::idle_add_local_once(move || {
        if let Some(paned) = paned.upgrade()
            && paned.width() > 0
        {
            paned.set_position(paned.width() / 2);
        }
    });
}
