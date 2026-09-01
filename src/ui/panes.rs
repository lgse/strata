// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

use gtk::{glib, prelude::*};

use crate::{
    app::{Browser, BrowserEvent},
    model::Location,
    services::{FileSource, OperationProvider},
};

use super::{
    browser::{BrowserView, PeekBehavior, PinStatus, SharedCutState},
    browser_modes::{BrowserDensity, BrowserMode},
    preview::PreviewDrawer,
    theme::ThemeManager,
};

pub(super) const PANE_PREFIX_TIMEOUT: Duration = Duration::from_millis(1_500);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum PaneLayout {
    #[default]
    Single,
    SideBySide,
    Stacked,
    Grid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PaneDirection {
    Left,
    Down,
    Up,
    Right,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PaneCommand {
    Layout(PaneLayout),
    Focus(PaneDirection),
    Toggle,
    Close,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SplitAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PaneNode {
    Leaf(u64),
    Split {
        axis: SplitAxis,
        first: Box<Self>,
        second: Box<Self>,
    },
}

impl PaneNode {
    fn pane_ids(&self, ids: &mut Vec<u64>) {
        match self {
            Self::Leaf(id) => ids.push(*id),
            Self::Split { first, second, .. } => {
                first.pane_ids(ids);
                second.pane_ids(ids);
            }
        }
    }

    fn contains(&self, target: u64) -> bool {
        match self {
            Self::Leaf(id) => *id == target,
            Self::Split { first, second, .. } => first.contains(target) || second.contains(target),
        }
    }

    fn first_leaf(&self) -> u64 {
        match self {
            Self::Leaf(id) => *id,
            Self::Split { first, .. } => first.first_leaf(),
        }
    }

    fn split(&mut self, target: u64, axis: SplitAxis, new_id: u64) -> bool {
        match self {
            Self::Leaf(id) if *id == target => {
                *self = Self::Split {
                    axis,
                    first: Box::new(Self::Leaf(*id)),
                    second: Box::new(Self::Leaf(new_id)),
                };
                true
            }
            Self::Leaf(_) => false,
            Self::Split { first, second, .. } => {
                first.split(target, axis, new_id) || second.split(target, axis, new_id)
            }
        }
    }

    fn remove(&mut self, target: u64) -> Option<u64> {
        let Self::Split { first, second, .. } = self else {
            return None;
        };
        if matches!(first.as_ref(), Self::Leaf(id) if *id == target) {
            let replacement = second.as_ref().clone();
            let next = replacement.first_leaf();
            *self = replacement;
            return Some(next);
        }
        if matches!(second.as_ref(), Self::Leaf(id) if *id == target) {
            let replacement = first.as_ref().clone();
            let next = replacement.first_leaf();
            *self = replacement;
            return Some(next);
        }
        if first.contains(target) {
            first.remove(target)
        } else {
            second.remove(target)
        }
    }

    fn rectangles(&self, area: PaneRect, panes: &mut Vec<(u64, PaneRect)>) {
        match self {
            Self::Leaf(id) => panes.push((*id, area)),
            Self::Split {
                axis: SplitAxis::Horizontal,
                first,
                second,
            } => {
                let middle = (area.left + area.right) / 2;
                first.rectangles(
                    PaneRect {
                        right: middle,
                        ..area
                    },
                    panes,
                );
                second.rectangles(
                    PaneRect {
                        left: middle,
                        ..area
                    },
                    panes,
                );
            }
            Self::Split {
                axis: SplitAxis::Vertical,
                first,
                second,
            } => {
                let middle = (area.top + area.bottom) / 2;
                first.rectangles(
                    PaneRect {
                        bottom: middle,
                        ..area
                    },
                    panes,
                );
                second.rectangles(
                    PaneRect {
                        top: middle,
                        ..area
                    },
                    panes,
                );
            }
        }
    }

    fn is_grid(&self) -> bool {
        let Self::Split {
            axis: SplitAxis::Horizontal,
            first,
            second,
        } = self
        else {
            return false;
        };
        [first.as_ref(), second.as_ref()].into_iter().all(|node| {
            matches!(
                node,
                Self::Split {
                    axis: SplitAxis::Vertical,
                    first,
                    second,
                } if matches!(first.as_ref(), Self::Leaf(_))
                    && matches!(second.as_ref(), Self::Leaf(_))
            )
        })
    }
}

#[derive(Clone, Copy)]
struct PaneRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl PaneRect {
    fn center(self) -> (i32, i32) {
        (self.left + self.right, self.top + self.bottom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PaneState {
    root: PaneNode,
    active: u64,
}

impl Default for PaneState {
    fn default() -> Self {
        Self {
            active: 0,
            root: PaneNode::Leaf(0),
        }
    }
}

impl PaneState {
    fn pane_ids(&self) -> Vec<u64> {
        let mut ids = Vec::with_capacity(4);
        self.root.pane_ids(&mut ids);
        ids
    }

    fn pane_count(&self) -> usize {
        self.pane_ids().len()
    }

    fn layout(&self) -> PaneLayout {
        match &self.root {
            PaneNode::Leaf(_) => PaneLayout::Single,
            _ if self.root.is_grid() => PaneLayout::Grid,
            PaneNode::Split {
                axis: SplitAxis::Horizontal,
                ..
            } => PaneLayout::SideBySide,
            PaneNode::Split {
                axis: SplitAxis::Vertical,
                ..
            } => PaneLayout::Stacked,
        }
    }

    fn split(&mut self, layout: PaneLayout, new_id: u64) -> bool {
        if self.pane_count() == 4 {
            return false;
        }
        let axis = match layout {
            PaneLayout::SideBySide => SplitAxis::Horizontal,
            PaneLayout::Stacked => SplitAxis::Vertical,
            PaneLayout::Single | PaneLayout::Grid => return false,
        };
        if !self.root.split(self.active, axis, new_id) {
            return false;
        }
        self.active = new_id;
        true
    }

    fn keep_active(&mut self) -> Vec<u64> {
        let removed = self
            .pane_ids()
            .into_iter()
            .filter(|id| *id != self.active)
            .collect();
        self.root = PaneNode::Leaf(self.active);
        removed
    }

    fn keep_pair(&mut self, layout: PaneLayout) -> Vec<u64> {
        let (axis, directions) = match layout {
            PaneLayout::SideBySide => (
                SplitAxis::Horizontal,
                [(PaneDirection::Left, true), (PaneDirection::Right, false)],
            ),
            PaneLayout::Stacked => (
                SplitAxis::Vertical,
                [(PaneDirection::Up, true), (PaneDirection::Down, false)],
            ),
            PaneLayout::Single | PaneLayout::Grid => return Vec::new(),
        };
        let Some((partner, partner_first)) =
            directions.into_iter().find_map(|(direction, first)| {
                let mut candidate = self.clone();
                candidate
                    .focus(direction)
                    .then_some((candidate.active, first))
            })
        else {
            return Vec::new();
        };
        let removed = self
            .pane_ids()
            .into_iter()
            .filter(|id| *id != self.active && *id != partner)
            .collect();
        let active = Box::new(PaneNode::Leaf(self.active));
        let partner = Box::new(PaneNode::Leaf(partner));
        let (first, second) = if partner_first {
            (partner, active)
        } else {
            (active, partner)
        };
        self.root = PaneNode::Split {
            axis,
            first,
            second,
        };
        removed
    }

    fn set_grid(&mut self, ids: [u64; 4]) {
        let column = |top, bottom| PaneNode::Split {
            axis: SplitAxis::Vertical,
            first: Box::new(PaneNode::Leaf(top)),
            second: Box::new(PaneNode::Leaf(bottom)),
        };
        self.root = PaneNode::Split {
            axis: SplitAxis::Horizontal,
            first: Box::new(column(ids[0], ids[1])),
            second: Box::new(column(ids[2], ids[3])),
        };
    }

    fn close_active(&mut self) -> Option<u64> {
        if self.pane_count() == 1 {
            return None;
        }
        let removed = self.active;
        self.active = self.root.remove(removed)?;
        Some(removed)
    }

    fn focus(&mut self, direction: PaneDirection) -> bool {
        let mut panes = Vec::with_capacity(4);
        self.root.rectangles(
            PaneRect {
                left: 0,
                top: 0,
                right: 1_024,
                bottom: 1_024,
            },
            &mut panes,
        );
        let Some((_, active)) = panes.iter().find(|(id, _)| *id == self.active) else {
            return false;
        };
        let active = *active;
        let (active_x, active_y) = active.center();
        let target = panes
            .into_iter()
            .filter(|(id, _)| *id != self.active)
            .filter_map(|(id, area)| {
                let (x, y) = area.center();
                match direction {
                    PaneDirection::Left if x < active_x => Some((
                        i32::from(!ranges_overlap(
                            active.top,
                            active.bottom,
                            area.top,
                            area.bottom,
                        )),
                        active_x - x,
                        (active_y - y).abs(),
                        id,
                    )),
                    PaneDirection::Down if y > active_y => Some((
                        i32::from(!ranges_overlap(
                            active.left,
                            active.right,
                            area.left,
                            area.right,
                        )),
                        y - active_y,
                        (active_x - x).abs(),
                        id,
                    )),
                    PaneDirection::Up if y < active_y => Some((
                        i32::from(!ranges_overlap(
                            active.left,
                            active.right,
                            area.left,
                            area.right,
                        )),
                        active_y - y,
                        (active_x - x).abs(),
                        id,
                    )),
                    PaneDirection::Right if x > active_x => Some((
                        i32::from(!ranges_overlap(
                            active.top,
                            active.bottom,
                            area.top,
                            area.bottom,
                        )),
                        x - active_x,
                        (active_y - y).abs(),
                        id,
                    )),
                    _ => None,
                }
            })
            .min()
            .map(|(_, _, _, id)| id);
        let Some(target) = target else {
            return false;
        };
        self.active = target;
        true
    }

    fn toggle(&mut self) -> bool {
        let panes = self.pane_ids();
        if panes.len() == 1 {
            return false;
        }
        let current = panes.iter().position(|id| *id == self.active).unwrap_or(0);
        self.active = panes[(current + 1) % panes.len()];
        true
    }
}

fn ranges_overlap(first_start: i32, first_end: i32, second_start: i32, second_end: i32) -> bool {
    first_start < second_end && second_start < first_end
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PrefixResult {
    Inactive,
    Command(PaneCommand),
    Cancelled,
    Unknown,
}

#[derive(Default)]
pub(super) struct PanePrefix {
    deadline: Option<Instant>,
}

impl PanePrefix {
    pub(super) fn begin(&mut self, now: Instant) {
        self.deadline = Some(now + PANE_PREFIX_TIMEOUT);
    }

    pub(super) fn cancel(&mut self) {
        self.deadline = None;
    }

    pub(super) fn suffix(&mut self, now: Instant, key: gtk::gdk::Key) -> PrefixResult {
        let Some(deadline) = self.deadline.take() else {
            return PrefixResult::Inactive;
        };
        if now > deadline {
            return PrefixResult::Inactive;
        }
        if key == gtk::gdk::Key::Escape {
            return PrefixResult::Cancelled;
        }
        pane_command_for_key(key).map_or(PrefixResult::Unknown, PrefixResult::Command)
    }
}

fn pane_command_for_key(key: gtk::gdk::Key) -> Option<PaneCommand> {
    match key {
        gtk::gdk::Key::v | gtk::gdk::Key::V => Some(PaneCommand::Layout(PaneLayout::SideBySide)),
        gtk::gdk::Key::s | gtk::gdk::Key::S => Some(PaneCommand::Layout(PaneLayout::Stacked)),
        gtk::gdk::Key::h | gtk::gdk::Key::H | gtk::gdk::Key::Left => {
            Some(PaneCommand::Focus(PaneDirection::Left))
        }
        gtk::gdk::Key::j | gtk::gdk::Key::J | gtk::gdk::Key::Down => {
            Some(PaneCommand::Focus(PaneDirection::Down))
        }
        gtk::gdk::Key::k | gtk::gdk::Key::K | gtk::gdk::Key::Up => {
            Some(PaneCommand::Focus(PaneDirection::Up))
        }
        gtk::gdk::Key::l | gtk::gdk::Key::L | gtk::gdk::Key::Right => {
            Some(PaneCommand::Focus(PaneDirection::Right))
        }
        gtk::gdk::Key::w | gtk::gdk::Key::W => Some(PaneCommand::Toggle),
        gtk::gdk::Key::c | gtk::gdk::Key::C => Some(PaneCommand::Close),
        _ => None,
    }
}

struct Pane {
    id: u64,
    shell: gtk::Box,
    view: BrowserView,
}

type ChangedHandler = Rc<dyn Fn(PaneLayout)>;
type PinHandler = Rc<dyn Fn(Location, String)>;
type PinStatusHandler = Rc<dyn Fn(&Location) -> PinStatus>;

struct WorkspaceState {
    widget: gtk::Box,
    locations: gtk::Stack,
    panes: RefCell<Vec<Pane>>,
    pane_state: RefCell<PaneState>,
    next_id: Cell<u64>,
    source: Rc<dyn FileSource>,
    operation_provider: Rc<dyn OperationProvider>,
    preferences: Rc<ThemeManager>,
    cut_state: Rc<SharedCutState>,
    preview: PreviewDrawer,
    peek_enabled: Cell<bool>,
    single_click_previews: Cell<bool>,
    pin_handler: RefCell<Option<PinHandler>>,
    pin_status_handler: RefCell<Option<PinStatusHandler>>,
    changed_handlers: RefCell<Vec<ChangedHandler>>,
}

#[derive(Clone)]
pub(super) struct PaneWorkspace {
    state: Rc<WorkspaceState>,
}

impl PaneWorkspace {
    pub(super) fn new(
        source: Rc<dyn FileSource>,
        operation_provider: Rc<dyn OperationProvider>,
        preferences: Rc<ThemeManager>,
        preview: PreviewDrawer,
        initial_location: Location,
    ) -> Self {
        let widget = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        widget.add_css_class("pane-workspace");
        widget.set_hexpand(true);
        widget.set_vexpand(true);
        let locations = gtk::Stack::builder()
            .hhomogeneous(false)
            .vhomogeneous(false)
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(100)
            .hexpand(true)
            .build();
        let workspace = Self {
            state: Rc::new(WorkspaceState {
                widget,
                locations,
                panes: RefCell::new(Vec::with_capacity(2)),
                pane_state: RefCell::new(PaneState::default()),
                next_id: Cell::new(1),
                source,
                operation_provider,
                preferences: preferences.clone(),
                cut_state: Rc::new(SharedCutState::default()),
                preview,
                peek_enabled: Cell::new(true),
                single_click_previews: Cell::new(preferences.single_click_previews()),
                pin_handler: RefCell::new(None),
                pin_status_handler: RefCell::new(None),
                changed_handlers: RefCell::new(Vec::new()),
            }),
        };
        workspace.create_pane(0);
        workspace.rebuild();
        workspace.active_browser().navigate(initial_location);
        workspace
    }

    pub(super) fn widget(&self) -> gtk::Widget {
        self.state.widget.clone().upcast()
    }

    pub(super) fn location_widget(&self) -> gtk::Widget {
        self.state.locations.clone().upcast()
    }

    pub(super) fn layout(&self) -> PaneLayout {
        self.state.pane_state.borrow().layout()
    }

    pub(super) fn pane_count(&self) -> usize {
        self.state.pane_state.borrow().pane_count()
    }

    pub(super) fn active_view(&self) -> BrowserView {
        let active = self.state.pane_state.borrow().active;
        self.state
            .panes
            .borrow()
            .iter()
            .find(|pane| pane.id == active)
            .expect("active pane exists")
            .view
            .clone()
    }

    pub(super) fn active_browser(&self) -> Rc<Browser> {
        self.active_view().browser()
    }

    pub(super) fn preview_occupied_width(&self) -> i32 {
        if self.pane_count() == 2 {
            self.state.widget.width()
        } else {
            self.active_view().preview_occupied_width()
        }
    }

    pub(super) fn observe_changed(&self, handler: ChangedHandler) {
        self.state.changed_handlers.borrow_mut().push(handler);
    }

    pub(super) fn set_pin_handlers(&self, handler: PinHandler, status: PinStatusHandler) {
        self.state.pin_handler.replace(Some(handler.clone()));
        self.state.pin_status_handler.replace(Some(status.clone()));
        for pane in self.state.panes.borrow().iter() {
            pane.view.set_pin_handlers(handler.clone(), status.clone());
        }
    }

    pub(super) fn set_view_mode(&self, mode: BrowserMode) {
        for pane in self.state.panes.borrow().iter() {
            pane.view.set_view_mode(mode);
        }
    }

    pub(super) fn set_density(&self, density: BrowserDensity) {
        for pane in self.state.panes.borrow().iter() {
            pane.view.set_density(density);
        }
    }

    pub(super) fn set_peek_enabled(&self, enabled: bool) {
        self.state.peek_enabled.set(enabled);
        for pane in self.state.panes.borrow().iter() {
            pane.view.set_peek_enabled(enabled);
        }
    }

    pub(super) fn set_single_click_previews(&self, enabled: bool) {
        self.state.single_click_previews.set(enabled);
        for pane in self.state.panes.borrow().iter() {
            pane.view.set_single_click_previews(enabled);
        }
    }

    pub(super) fn set_layout(&self, layout: PaneLayout) {
        match layout {
            PaneLayout::Single => self.keep_active(),
            PaneLayout::SideBySide | PaneLayout::Stacked => self.split_active(layout),
            PaneLayout::Grid => self.set_grid(),
        }
    }

    fn keep_active(&self) {
        if self.pane_count() == 1 {
            return;
        }
        let removed = self.state.pane_state.borrow_mut().keep_active();
        for id in removed {
            self.remove_pane(id);
        }
        self.finish_layout_change();
    }

    fn split_active(&self, layout: PaneLayout) {
        if self.pane_count() == 4 {
            let removed = self.state.pane_state.borrow_mut().keep_pair(layout);
            for id in removed {
                self.remove_pane(id);
            }
            self.finish_layout_change();
            return;
        }
        let location = self.active_browser().active_location();
        let id = self.allocate_pane_id();
        self.create_pane(id);
        let split = self.state.pane_state.borrow_mut().split(layout, id);
        debug_assert!(split);
        if let Some(location) = location {
            self.active_browser().navigate(location);
        }
        self.finish_layout_change();
    }

    fn set_grid(&self) {
        let location = self.active_browser().active_location();
        let mut ids = self.state.pane_state.borrow().pane_ids();
        let mut created = Vec::new();
        while ids.len() < 4 {
            let id = self.allocate_pane_id();
            self.create_pane(id);
            ids.push(id);
            created.push(id);
        }
        self.state
            .pane_state
            .borrow_mut()
            .set_grid(ids.try_into().expect("grid has four panes"));
        if let Some(location) = location {
            for id in created {
                let browser = self
                    .state
                    .panes
                    .borrow()
                    .iter()
                    .find(|pane| pane.id == id)
                    .expect("created pane exists")
                    .view
                    .browser();
                browser.navigate(location.clone());
            }
        }
        self.finish_layout_change();
    }

    fn finish_layout_change(&self) {
        self.rebuild();
        self.focus_active();
        self.sync_preview();
        self.notify_changed();
    }

    fn allocate_pane_id(&self) -> u64 {
        let id = self.state.next_id.get();
        self.state.next_id.set(id.saturating_add(1));
        id
    }

    pub(super) fn apply_command(&self, command: PaneCommand) {
        match command {
            PaneCommand::Layout(layout) => self.set_layout(layout),
            PaneCommand::Focus(direction) => {
                let changed = self.state.pane_state.borrow_mut().focus(direction);
                if changed {
                    self.activate_current(true);
                }
            }
            PaneCommand::Toggle => {
                let changed = self.state.pane_state.borrow_mut().toggle();
                if changed {
                    self.activate_current(true);
                }
            }
            PaneCommand::Close => self.close_active(),
        }
    }

    pub(super) fn close_active(&self) {
        let closed = self.state.pane_state.borrow_mut().close_active();
        let Some(id) = closed else {
            return;
        };
        self.remove_pane(id);
        self.rebuild();
        self.focus_active();
        self.sync_preview();
        self.notify_changed();
    }

    pub(super) fn clear(&self) {
        for pane in self.state.panes.borrow().iter() {
            pane.view.browser().clear_observer();
        }
        self.state.changed_handlers.borrow_mut().clear();
    }

    fn create_pane(&self, id: u64) {
        let view = BrowserView::new(
            self.state.source.clone(),
            PeekBehavior::default(),
            self.state.cut_state.clone(),
        );
        view.set_view_mode(self.state.preferences.browser_mode());
        view.set_density(self.state.preferences.browser_density());
        view.set_peek_enabled(self.state.peek_enabled.get());
        view.set_single_click_previews(self.state.single_click_previews.get());
        view.set_operation_provider(self.state.operation_provider.clone());
        if let (Some(handler), Some(status)) = (
            self.state.pin_handler.borrow().clone(),
            self.state.pin_status_handler.borrow().clone(),
        ) {
            view.set_pin_handlers(handler, status);
        }

        let shell = gtk::Box::new(gtk::Orientation::Vertical, 0);
        shell.add_css_class("browser-pane");
        shell.set_focusable(true);
        shell.set_hexpand(true);
        shell.set_vexpand(true);
        shell.append(&view.widget());

        let weak_workspace = Rc::downgrade(&self.state);
        let activity_shell = shell.clone();
        let focus = gtk::EventControllerFocus::new();
        focus.connect_enter(move |_| {
            if let Some(state) = weak_workspace.upgrade() {
                PaneWorkspace { state }.activate_shell(&activity_shell);
            }
        });
        shell.add_controller(focus);

        let weak_workspace = Rc::downgrade(&self.state);
        let activity_shell = shell.clone();
        let pointer = gtk::GestureClick::new();
        pointer.set_button(0);
        pointer.set_propagation_phase(gtk::PropagationPhase::Capture);
        pointer.connect_pressed(move |_, _, _, _| {
            if let Some(state) = weak_workspace.upgrade() {
                PaneWorkspace { state }.activate_shell(&activity_shell);
            }
        });
        shell.add_controller(pointer);

        let browser = view.browser();
        let weak_browser = Rc::downgrade(&browser);
        let weak_workspace = Rc::downgrade(&self.state);
        browser.observe(move |event| {
            if let (Some(state), Some(browser)) = (weak_workspace.upgrade(), weak_browser.upgrade())
            {
                PaneWorkspace { state }.handle_browser_event(&browser, event);
            }
        });

        self.state
            .locations
            .add_named(&view.location_widget(), Some(&format!("pane-{id}")));
        self.state.panes.borrow_mut().push(Pane { id, shell, view });
        self.show_active_location();
    }

    fn remove_pane(&self, id: u64) {
        let index = self
            .state
            .panes
            .borrow()
            .iter()
            .position(|pane| pane.id == id)
            .expect("removed pane exists");
        let pane = self.state.panes.borrow_mut().remove(index);
        self.state.locations.remove(&pane.view.location_widget());
        pane.view.browser().clear_observer();
    }

    fn rebuild(&self) {
        if let Some(child) = self.state.widget.first_child() {
            self.state.widget.remove(&child);
            Self::detach_tree(&child);
        }
        self.state.widget.remove_css_class("split");
        if self.pane_count() > 1 {
            self.state.widget.add_css_class("split");
        }
        let root = self.state.pane_state.borrow().root.clone();
        self.state.widget.append(&self.build_tree(&root));
        self.update_active_style();
        self.show_active_location();
    }

    fn build_tree(&self, node: &PaneNode) -> gtk::Widget {
        match node {
            PaneNode::Leaf(id) => self
                .state
                .panes
                .borrow()
                .iter()
                .find(|pane| pane.id == *id)
                .expect("pane tree leaf exists")
                .shell
                .clone()
                .upcast(),
            PaneNode::Split {
                axis,
                first,
                second,
            } => {
                let paned = gtk::Paned::new(match axis {
                    SplitAxis::Horizontal => gtk::Orientation::Horizontal,
                    SplitAxis::Vertical => gtk::Orientation::Vertical,
                });
                paned.set_wide_handle(false);
                paned.set_resize_start_child(true);
                paned.set_resize_end_child(true);
                paned.set_shrink_start_child(false);
                paned.set_shrink_end_child(false);
                paned.set_hexpand(true);
                paned.set_vexpand(true);
                paned.set_start_child(Some(&self.build_tree(first)));
                paned.set_end_child(Some(&self.build_tree(second)));
                Self::center_divider(&paned);
                paned.upcast()
            }
        }
    }

    fn detach_tree(widget: &gtk::Widget) {
        let Ok(paned) = widget.clone().downcast::<gtk::Paned>() else {
            return;
        };
        if let Some(child) = paned.start_child() {
            paned.set_start_child(None::<&gtk::Widget>);
            Self::detach_tree(&child);
        }
        if let Some(child) = paned.end_child() {
            paned.set_end_child(None::<&gtk::Widget>);
            Self::detach_tree(&child);
        }
    }

    fn center_divider(paned: &gtk::Paned) {
        let paned = paned.clone();
        glib::idle_add_local_once(move || {
            let size = match paned.orientation() {
                gtk::Orientation::Horizontal => paned.width(),
                gtk::Orientation::Vertical => paned.height(),
                _ => 0,
            };
            if size > 0 {
                paned.set_position(size / 2);
            }
        });
    }

    fn activate_shell(&self, shell: &gtk::Box) {
        let id = self
            .state
            .panes
            .borrow()
            .iter()
            .find(|pane| pane.shell == *shell)
            .map(|pane| pane.id);
        let Some(id) = id else {
            return;
        };
        let changed = {
            let mut state = self.state.pane_state.borrow_mut();
            let changed = state.active != id;
            state.active = id;
            changed
        };
        if changed {
            self.activate_current(false);
        }
    }

    fn activate_current(&self, focus: bool) {
        self.update_active_style();
        self.show_active_location();
        self.sync_preview();
        self.notify_changed();
        if focus {
            self.focus_active();
        }
    }

    fn update_active_style(&self) {
        let state = self.state.pane_state.borrow();
        for pane in self.state.panes.borrow().iter() {
            if state.pane_count() > 1 && pane.id == state.active {
                pane.shell.add_css_class("active-pane");
            } else {
                pane.shell.remove_css_class("active-pane");
            }
        }
    }

    fn show_active_location(&self) {
        let active = self.state.pane_state.borrow().active;
        if let Some(pane) = self
            .state
            .panes
            .borrow()
            .iter()
            .find(|pane| pane.id == active)
        {
            self.state
                .locations
                .set_visible_child_name(&format!("pane-{}", pane.id));
        }
    }

    fn focus_active(&self) {
        let active = self.state.pane_state.borrow().active;
        let (shell, browser) = {
            let panes = self.state.panes.borrow();
            let pane = panes
                .iter()
                .find(|pane| pane.id == active)
                .expect("active pane exists");
            (pane.shell.clone(), pane.view.browser())
        };
        shell.grab_focus();
        browser.focus_active();
    }

    fn is_active_browser(&self, browser: &Rc<Browser>) -> bool {
        Rc::ptr_eq(browser, &self.active_browser())
    }

    fn handle_browser_event(&self, source: &Rc<Browser>, event: BrowserEvent) {
        if let BrowserEvent::LocationsInvalidated { locations } = &event {
            let peers = self
                .state
                .panes
                .borrow()
                .iter()
                .map(|pane| pane.view.browser())
                .collect::<Vec<_>>();
            for browser in peers {
                if !Rc::ptr_eq(&browser, source) {
                    browser.invalidate_locations(locations);
                }
            }
        }
        if !self.is_active_browser(source) {
            return;
        }
        match event {
            BrowserEvent::PreviewRequested { entry } => self.state.preview.show(entry),
            BrowserEvent::FocusChanged { .. } if self.state.preview.is_open() => {
                self.sync_preview();
            }
            _ => {}
        }
        self.notify_changed();
    }

    fn sync_preview(&self) {
        if !self.state.preview.is_open() {
            return;
        }
        match self.active_browser().focused_entry() {
            Some(entry) if !entry.is_directory() => self.state.preview.show(entry),
            _ => self.state.preview.close(),
        }
    }

    fn notify_changed(&self) {
        let handlers = self.state.changed_handlers.borrow().clone();
        let layout = self.layout();
        for handler in handlers {
            handler(layout);
        }
    }
}

#[cfg(test)]
mod tests;
