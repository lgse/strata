// SPDX-License-Identifier: MIT

//! The browser views one window tab shows and the one its shared chrome acts on.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use crate::{
    app::Browser,
    model::{FileEntry, Location},
};

use super::{BrowserView, OtherPane, ViewState};

type Listener = Rc<dyn Fn(&BrowserView)>;

struct TargetState {
    views: RefCell<Vec<BrowserView>>,
    active: Cell<usize>,
    retargeted: RefCell<Vec<Listener>>,
    added: RefCell<Vec<Listener>>,
}

/// The sidebar, header and other tab chrome resolve their view through this
/// handle when they act, so they follow whichever pane is active.
#[derive(Clone)]
pub(in crate::ui) struct PaneTarget(Rc<TargetState>);

impl PaneTarget {
    pub(in crate::ui) fn new(view: &BrowserView) -> Self {
        Self(Rc::new(TargetState {
            views: RefCell::new(vec![view.clone()]),
            active: Cell::new(0),
            retargeted: RefCell::new(Vec::new()),
            added: RefCell::new(Vec::new()),
        }))
    }

    pub(in crate::ui) fn view(&self) -> BrowserView {
        self.0.views.borrow()[self.0.active.get()].clone()
    }

    pub(in crate::ui) fn browser(&self) -> Rc<Browser> {
        self.view().browser()
    }

    pub(in crate::ui) fn views(&self) -> Vec<BrowserView> {
        self.0.views.borrow().clone()
    }

    pub(in crate::ui) fn is_active(&self, view: &BrowserView) -> bool {
        self.view().ptr_eq(view)
    }

    /// Adds another pane's view; chrome that serves every pane, such as the
    /// sidebar's place highlighting, hears about it.
    pub(in crate::ui) fn add(&self, view: &BrowserView) {
        if self.position(view).is_some() {
            return;
        }
        self.0.views.borrow_mut().push(view.clone());
        let listeners = self.0.added.borrow().clone();
        for listener in listeners {
            listener(view);
        }
    }

    /// Makes `view`, which must have been added, the one chrome acts on.
    pub(in crate::ui) fn activate(&self, view: &BrowserView) {
        let Some(index) = self.position(view) else {
            return;
        };
        if self.0.active.replace(index) == index {
            return;
        }
        let listeners = self.0.retargeted.borrow().clone();
        for listener in listeners {
            listener(view);
        }
    }

    pub(in crate::ui) fn connect_retargeted(&self, listener: impl Fn(&BrowserView) + 'static) {
        self.0.retargeted.borrow_mut().push(Rc::new(listener));
    }

    pub(in crate::ui) fn connect_added(&self, listener: impl Fn(&BrowserView) + 'static) {
        self.0.added.borrow_mut().push(Rc::new(listener));
    }

    pub(in crate::ui) fn downgrade(&self) -> WeakPaneTarget {
        WeakPaneTarget(Rc::downgrade(&self.0))
    }

    fn position(&self, view: &BrowserView) -> Option<usize> {
        self.0
            .views
            .borrow()
            .iter()
            .position(|known| known.ptr_eq(view))
    }
}

#[derive(Clone)]
pub(in crate::ui) struct WeakPaneTarget(std::rc::Weak<TargetState>);

impl WeakPaneTarget {
    pub(in crate::ui) fn upgrade(&self) -> Option<PaneTarget> {
        self.0.upgrade().map(PaneTarget)
    }
}

impl BrowserView {
    pub(in crate::ui) fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.state, &other.state)
    }

    pub(in crate::ui) fn trash_button(&self) -> Option<gtk::Button> {
        self.state.trash_button.upgrade()
    }

    pub(in crate::ui) fn set_split_pane(&self, split: bool) {
        self.state.split_pane.set(split);
    }

    pub(in crate::ui) fn is_split_pane(&self) -> bool {
        self.state.split_pane.get()
    }

    pub(in crate::ui) fn set_other_pane(&self, other: Option<OtherPane>) {
        self.state.other_pane.replace(other);
    }
}

impl ViewState {
    pub(super) fn other_pane_location(&self) -> Option<Location> {
        let location = self.other_pane.borrow().as_ref()?.location.clone();
        location()
    }

    pub(super) fn transfer_to_other_pane(&self, entries: Vec<FileEntry>, moving: bool) {
        let transfer = self
            .other_pane
            .borrow()
            .as_ref()
            .map(|other| other.transfer.clone());
        if let Some(transfer) = transfer {
            transfer(entries, moving);
        }
    }
}
