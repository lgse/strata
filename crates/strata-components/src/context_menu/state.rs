// SPDX-License-Identifier: MIT

//! What is open: the menu's target, where it opened, its model and its open submenus.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;

use super::model::{ActionId, MenuModel};
use crate::contract::{EntryId, Point};
use crate::scroll::{Bar, BarMode};

/// A menu scroll bar: shown whenever the menu overflows, hidden when it fits.
pub(super) fn menu_bar() -> Bar {
    let bar = Bar::new();
    bar.set_mode(BarMode::Always);
    bar
}

/// What a menu is for. Which actions apply to it is the host's policy.
#[derive(Clone)]
pub enum MenuTarget {
    Entry(EntryId),
    /// The pane's background: a press on empty space, or the Menu key with nothing selected.
    Background,
    /// Anything else the host opens a menu for: a sidebar row, a crumb.
    Other(Rc<dyn Any>),
}

impl MenuTarget {
    /// The entry, for [`MenuTarget::Entry`].
    pub fn entry(&self) -> Option<EntryId> {
        match self {
            Self::Entry(id) => Some(*id),
            _ => None,
        }
    }
}

impl fmt::Debug for MenuTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Entry(id) => f.debug_tuple("Entry").field(id).finish(),
            Self::Background => f.write_str("Background"),
            Self::Other(_) => f.write_str("Other(..)"),
        }
    }
}

/// `Other` targets are equal when they are the same value.
impl PartialEq for MenuTarget {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Entry(a), Self::Entry(b)) => a == b,
            (Self::Background, Self::Background) => true,
            (Self::Other(a), Self::Other(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}

/// What the menu asks the host to do.
#[derive(Clone, Debug, PartialEq)]
pub enum MenuIntent {
    /// Run `action` on `target`. The menu has closed.
    Activate {
        /// What the menu was for.
        target: MenuTarget,
        /// The item's action.
        action: ActionId,
    },
}

#[derive(Clone, Debug, Default)]
pub struct ContextMenuState {
    open: Option<Open>,
}

#[derive(Clone, Copy, Debug)]
struct LockedWidth {
    font_px: f32,
    min_width: f32,
    width: f32,
}

#[derive(Clone, Debug)]
struct Open {
    target: MenuTarget,
    at: Point,
    model: MenuModel,
    /// The open submenus: the item index (as [`MenuModel::item_at`] counts) at each level.
    submenu: Vec<usize>,
    locked_width: Cell<Option<LockedWidth>>,
    scroll_bars: RefCell<Vec<Bar>>,
}

impl Open {
    fn drain_bars_past(&self, keep_submenus: usize) {
        let mut bars = self.scroll_bars.borrow_mut();
        let keep = (keep_submenus + 1).min(bars.len());
        for bar in bars.drain(keep..) {
            bar.release();
        }
    }
}

impl ContextMenuState {
    /// No menu open.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(&mut self, target: MenuTarget, at: Point, model: MenuModel) {
        if let Some(prev) = self.open.take() {
            for bar in prev.scroll_bars.borrow().iter() {
                bar.release();
            }
        }
        self.open = Some(Open {
            target,
            at,
            model,
            submenu: Vec::new(),
            locked_width: Cell::new(None),
            scroll_bars: RefCell::new(Vec::new()),
        });
    }

    pub fn update_model(&mut self, model: MenuModel) {
        if let Some(open) = &mut self.open {
            let keep = (1..=open.submenu.len())
                .take_while(|&n| {
                    model
                        .item_at(&open.submenu[..n])
                        .is_some_and(|i| i.submenu.is_some())
                })
                .count();
            open.submenu.truncate(keep);
            open.drain_bars_past(keep);
            open.model = model;
        }
    }

    /// Closes the menu, handing back what it was for.
    pub fn close(&mut self) -> Option<MenuTarget> {
        let open = self.open.take()?;
        for bar in open.scroll_bars.borrow().iter() {
            bar.release();
        }
        Some(open.target)
    }

    /// Whether a menu is open.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// What the open menu is for.
    pub fn target(&self) -> Option<&MenuTarget> {
        self.open.as_ref().map(|o| &o.target)
    }

    /// Where the open menu opened, in window points.
    pub fn at(&self) -> Option<Point> {
        self.open.as_ref().map(|o| o.at)
    }

    /// The open menu's model.
    pub fn model(&self) -> Option<&MenuModel> {
        self.open.as_ref().map(|o| &o.model)
    }

    /// The open submenus, outermost first, as item indices at each level.
    pub fn submenu(&self) -> &[usize] {
        self.open.as_ref().map_or(&[], |o| &o.submenu)
    }

    pub fn open_submenu(&mut self, path: &[usize]) -> bool {
        let Some(open) = &mut self.open else {
            return false;
        };
        if open.submenu == path || open.model.item_at(path).is_none_or(|i| i.submenu.is_none()) {
            return false;
        }
        let common = open
            .submenu
            .iter()
            .zip(path)
            .take_while(|(a, b)| a == b)
            .count();
        open.drain_bars_past(common);
        open.submenu = path.to_vec();
        true
    }

    pub fn close_submenu(&mut self, depth: usize) -> bool {
        match &mut self.open {
            Some(open) if open.submenu.len() > depth => {
                open.submenu.truncate(depth);
                open.drain_bars_past(depth);
                true
            }
            _ => false,
        }
    }

    #[cfg(test)]
    pub(super) fn scroll_bar_count(&self) -> usize {
        self.open
            .as_ref()
            .map_or(0, |open| open.scroll_bars.borrow().len())
    }

    pub(super) fn scroll_bar(&self, depth: usize) -> Bar {
        let Some(open) = &self.open else {
            return menu_bar();
        };
        let mut bars = open.scroll_bars.borrow_mut();
        while bars.len() <= depth {
            bars.push(menu_bar());
        }
        bars[depth].clone()
    }

    pub(super) fn resolve_width(
        &self,
        font_px: f32,
        min_width: f32,
        measured: impl FnOnce() -> f32,
    ) -> f32 {
        let Some(open) = &self.open else {
            return measured();
        };
        if let Some(locked) = open.locked_width.get()
            && (locked.font_px - font_px).abs() <= f32::EPSILON
            && (locked.min_width - min_width).abs() <= f32::EPSILON
        {
            return locked.width;
        }
        let width = measured();
        open.locked_width.set(Some(LockedWidth {
            font_px,
            min_width,
            width,
        }));
        width
    }
}
