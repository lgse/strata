//! What is open: the menu's target, where it opened, its model and its open submenus.

use std::any::Any;
use std::fmt;
use std::rc::Rc;

use super::model::{ActionId, MenuModel};
use crate::contract::{EntryId, Point};

/// What a menu is for. Which actions apply to it is the host's policy.
#[derive(Clone)]
pub enum MenuTarget {
    /// An entry: the one right-clicked, or the keyboard cursor. A multi-selection's menu is
    /// this entry's too; the host reads the selection.
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

/// The context menu's state, held by the host and reached through an accessor. One menu is
/// open at a time.
#[derive(Clone, Debug, Default)]
pub struct ContextMenuState {
    open: Option<Open>,
}

#[derive(Clone, Debug)]
struct Open {
    target: MenuTarget,
    at: Point,
    model: MenuModel,
    /// The open submenus: the item index (as [`MenuModel::item_at`] counts) at each level.
    submenu: Vec<usize>,
    /// Keys opened a submenu and sent the Tab that moves the keyboard into it: that Tab is
    /// Rune's, not a Down (see `view.rs`).
    entering: bool,
}

impl ContextMenuState {
    /// No menu open.
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens `model` for `target` at window point `at` (where a secondary press landed, or
    /// the keyboard anchor), closing any menu that was open.
    pub fn open(&mut self, target: MenuTarget, at: Point, model: MenuModel) {
        self.open = Some(Open {
            target,
            at,
            model,
            submenu: Vec::new(),
            entering: false,
        });
    }

    /// Replaces the open menu's model, as when a lookup finishes (Open With…). Open
    /// submenus stay open while the new model still has a submenu at their place.
    ///
    /// Rows are keyed by their place in the menu (rune-kit's, counting the header, labels and
    /// separators), and the keyboard follows the key. Replacing a row in place keeps the
    /// keyboard on its action; adding or removing an entry above the focused row hands the
    /// keyboard to another action, or to the first row when its key is now a separator or a
    /// label. So reserve a row that a lookup will fill (insensitive) rather than add it then.
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
            open.model = model;
        }
    }

    /// Closes the menu, handing back what it was for.
    pub fn close(&mut self) -> Option<MenuTarget> {
        self.open.take().map(|o| o.target)
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

    /// Opens the submenu at `path`, as a pointer does, closing any other: a path that names
    /// no submenu changes nothing. True when it changed.
    pub fn open_submenu(&mut self, path: &[usize]) -> bool {
        let Some(open) = &mut self.open else {
            return false;
        };
        if open.submenu == path || open.model.item_at(path).is_none_or(|i| i.submenu.is_none()) {
            return false;
        }
        open.submenu = path.to_vec();
        true
    }

    /// [`Self::open_submenu`] from the keys: the next Tab moves the keyboard into it. True
    /// when the path names a submenu, open already or not.
    pub(super) fn enter_submenu(&mut self, path: &[usize]) -> bool {
        self.open_submenu(path);
        match &mut self.open {
            Some(open) if open.submenu == path => {
                open.entering = true;
                true
            }
            _ => false,
        }
    }

    /// Whether the Tab arriving now is the one [`Self::enter_submenu`] sent.
    pub(super) fn take_entering(&mut self) -> bool {
        self.open
            .as_mut()
            .is_some_and(|o| std::mem::take(&mut o.entering))
    }

    /// Closes the submenus `depth` levels in and deeper: 0 closes them all. True when one
    /// closed.
    pub fn close_submenu(&mut self, depth: usize) -> bool {
        match &mut self.open {
            Some(open) if open.submenu.len() > depth => {
                open.submenu.truncate(depth);
                true
            }
            _ => false,
        }
    }
}
