//! The menu model: what a menu shows, as plain data. Strata builds it (which actions apply to
//! the target is its policy); C21 draws it.

use std::rc::Rc;

use strata_kit::icons::IconName;
use strata_kit::look::Look;

use super::view::{height, width};

/// An action's id, chosen by the host: what [`super::MenuIntent::Activate`] hands back.
pub type ActionId = Rc<str>;

/// A whole menu: optional header lines, then sections divided by separators.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuModel {
    /// Lines over the first section, such as an entry's name and compact path, or "3 items
    /// selected". They take no focus.
    pub header: Vec<String>,
    /// The sections, in order. An empty one draws nothing: no label and no extra separator.
    pub sections: Vec<Section>,
}

impl MenuModel {
    /// A menu with no header and no sections.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same menu with `line` added to the header.
    pub fn header_line(mut self, line: impl Into<String>) -> Self {
        self.header.push(line.into());
        self
    }

    /// The same menu with `section` added at the end.
    pub fn section(mut self, section: Section) -> Self {
        self.sections.push(section);
        self
    }

    /// The item at `path`: an index into the visible items of the top level, then of each
    /// submenu along the way ([`Section`]s flattened, empty ones skipped).
    pub fn item_at(&self, path: &[usize]) -> Option<&Item> {
        let (&first, rest) = path.split_first()?;
        let mut item = items(&self.sections).nth(first)?;
        for &i in rest {
            item = items(item.submenu.as_deref()?).nth(i)?;
        }
        Some(item)
    }
}

/// The items of `sections`, in order, across sections.
pub(super) fn items(sections: &[Section]) -> impl Iterator<Item = &Item> {
    sections.iter().flat_map(|s| s.items.iter())
}

/// A group of items. Sections are divided by separators; a section with no items is left out
/// whole, so actions a target hides never leave an empty group or a doubled separator.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Section {
    /// A small heading over the items, if any.
    pub label: Option<String>,
    /// The items.
    pub items: Vec<Item>,
}

impl Section {
    /// A section with no label.
    pub fn new(items: impl IntoIterator<Item = Item>) -> Self {
        Self {
            label: None,
            items: items.into_iter().collect(),
        }
    }

    /// The same section under heading `label`.
    pub fn labelled(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
}

/// One row: an action, or a submenu's trigger.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// What [`super::MenuIntent::Activate`] reports. A submenu's trigger activates nothing.
    pub action: ActionId,
    /// The row's text, and its accessible name.
    pub label: String,
    /// An icon before the label.
    pub icon: Option<IconName>,
    /// The shortcut hint at the row's end, in the host's notation ("Ctrl+C", "F2 / Ctrl+R").
    /// Assistive technology hears it as the item's keyboard shortcut.
    pub shortcut: Option<String>,
    /// A faint second line, which assistive technology hears as the item's description:
    /// "Looking for compatible applications…" on an insensitive Open With….
    pub description: Option<String>,
    /// A destructive action: the label and icon take the danger ink.
    pub danger: bool,
    /// False draws the row insensitive: it takes no focus and no press.
    pub enabled: bool,
    /// A check item, showing the check when `Some(true)`. A checked row takes the menu's
    /// first focus (rune-kit's `focus_selected`), as a popup opens on its current choice.
    pub checked: Option<bool>,
    /// A submenu: its sections open beside the row.
    pub submenu: Option<Vec<Section>>,
}

impl Item {
    /// An enabled action `action` labelled `label`.
    pub fn new(action: impl Into<ActionId>, label: impl Into<String>) -> Self {
        Self {
            action: action.into(),
            label: label.into(),
            icon: None,
            shortcut: None,
            description: None,
            danger: false,
            enabled: true,
            checked: None,
            submenu: None,
        }
    }

    /// A submenu trigger labelled `label`, opening `sections`.
    pub fn submenu(
        action: impl Into<ActionId>,
        label: impl Into<String>,
        sections: impl IntoIterator<Item = Section>,
    ) -> Self {
        Self {
            submenu: Some(sections.into_iter().collect()),
            ..Self::new(action, label)
        }
    }

    /// With icon `icon`.
    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// With shortcut hint `hint`.
    pub fn shortcut(mut self, hint: impl Into<String>) -> Self {
        self.shortcut = Some(hint.into());
        self
    }

    /// With description `text`.
    pub fn description(mut self, text: impl Into<String>) -> Self {
        self.description = Some(text.into());
        self
    }

    /// Destructive when `on`.
    pub fn danger(mut self, on: bool) -> Self {
        self.danger = on;
        self
    }

    /// Sensitive when `on`.
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }

    /// A check item, checked when `on`. A checked row takes the menu's first focus
    /// (rune-kit's `focus_selected`).
    pub fn checked(mut self, on: bool) -> Self {
        self.checked = Some(on);
        self
    }
}

/// What the chooser window's minimum size must be for every menu and submenu to fit while
/// menus stay inside the window (U14).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChooserMinimum {
    /// Wide enough that each submenu has room beside its menu wherever the menu opens: the
    /// menu's width plus twice the submenu's, plus 16 pt, for the widest such pair (the menu
    /// slides to stay in, and the submenu needs its width free on one side of it). With no
    /// submenus, the widest menu plus 16 pt. Required.
    pub width: f32,
    /// The tallest submenu plus 16 pt: submenus never shrink or scroll. Required.
    pub height: f32,
    /// The tallest top-level menu plus 16 pt: menus scroll, so this is only for a menu that
    /// shouldn't. Optional.
    pub unscrolled_height: f32,
}

/// What a menu keeps from the window's edges, twice (`runtime/anchor.rs`).
const MARGINS: f32 = 16.0;

/// The chooser's minimum size for `menus`, the menus it can open, each menu and submenu
/// measured as [`super::ContextMenu`] draws it in `look`.
///
/// The rule is S1's (Q5, `docs/decisions/S1-linux-window.md`), for menus of different
/// widths. Tiling compositors ignore a window's minimum size, so a tiled chooser can still be
/// smaller.
pub fn chooser_minimum(menus: &[MenuModel], look: &Look) -> ChooserMinimum {
    let mut need = Need::default();
    for m in menus {
        let w = width(look, &m.header, &m.sections);
        need.width = need.width.max(w + MARGINS);
        need.menu = need.menu.max(height(look, &m.header, &m.sections));
        need.submenus(look, w, &m.sections);
    }
    ChooserMinimum {
        width: need.width,
        height: need.submenu + MARGINS,
        unscrolled_height: need.menu + MARGINS,
    }
}

#[derive(Default)]
struct Need {
    width: f32,
    menu: f32,
    submenu: f32,
}

impl Need {
    /// The submenus of a menu `parent` wide, and theirs.
    fn submenus(&mut self, look: &Look, parent: f32, sections: &[Section]) {
        for sub in items(sections).filter_map(|i| i.submenu.as_deref()) {
            let w = width(look, &[], sub);
            self.width = self.width.max(parent + 2.0 * w + MARGINS);
            self.submenu = self.submenu.max(height(look, &[], sub));
            self.submenus(look, w, sub);
        }
    }
}
