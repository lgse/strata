//! Drawing a menu on rune-kit's `Menu`, then adjusting the surfaces it pushed into the layer:
//! their name, Strata's look, and the keys rune-kit's menu leaves out (submenus, Ctrl+A).
//!
//! rune-kit (5d421fd) pushes a pointer menu with a close request as `[scrim, point, surface,
//! submenus…]`, an open submenu after its menu and before any deeper one. The adjustments
//! find the surfaces by that order: an El's key can't be read once built.
//! `layer_order_is_runekits` (tests) fails if a Rune bump changes it.

use std::rc::Rc;

use rune::{El, Key, KeyEvent, Mods, Tx, el};
use rune_kit::{Menu, MenuEntry, MenuItem};
use strata_kit::look::Look;

use super::model::Section;
use crate::contract::Point;

/// Strata's menu corners before roundness (`.item-context-menu`, 9 px).
pub(super) const RADIUS: f32 = 9.0;
/// rune-kit's compact menus' least width.
pub(super) const MIN_WIDTH: f32 = 160.0;
/// The widest a menu grows to show its rows whole; past it, labels end in an ellipsis.
pub(super) const MAX_WIDTH: f32 = 360.0;

type Select<S> = Rc<dyn Fn(&mut S, &[usize], &mut Tx)>;
/// Open the submenu at a path: from the pointer (`false`), or from the keys, which then send
/// the Tab that moves the keyboard into it (`true`; true back when it opened).
type Submenu<S> = Rc<dyn Fn(&mut S, Vec<usize>, bool) -> bool>;
type Entering<S> = Rc<dyn Fn(&mut S) -> bool>;
type CloseTo<S> = Rc<dyn Fn(&mut S, usize)>;
type Dismiss<S> = Rc<dyn Fn(&mut S)>;

/// One menu to draw.
pub(super) struct Render<'a, S> {
    /// The menu's key: its surface is `{key}-menu`, its rows `{key}-item{n}`.
    pub key: &'a str,
    /// Where it opens, in window points.
    pub at: Point,
    /// The menu's accessible name.
    pub name: &'a str,
    pub header: &'a [String],
    pub sections: &'a [Section],
    /// The open submenus: item indices at each level.
    pub open: &'a [usize],
    pub look: &'a Look,
    /// An item without a submenu was chosen: its path.
    pub on_select: Select<S>,
    /// Open the submenu at a path.
    pub on_submenu: Submenu<S>,
    /// Whether this Tab is the one opening a submenu from the keys sent.
    pub entering: Entering<S>,
    /// Close the submenus this many levels in and deeper.
    pub close_to: CloseTo<S>,
    /// Escape, or a press outside: close the whole menu.
    pub on_dismiss: Dismiss<S>,
}

/// A row as the surfaces' key handlers see it.
#[derive(Clone, Debug)]
struct Row {
    key: String,
    path: Vec<usize>,
    submenu: bool,
    /// Its submenu has a sensitive row for the keyboard to move to.
    enterable: bool,
}

impl<S: 'static> Render<'_, S> {
    /// The menu's anchor element (zero size, out of the layout), with its surfaces pushed into
    /// `layer`.
    pub fn build(self, layer: &mut Vec<El>) -> El {
        let kit = &self.look.kit;
        let mut rows = Vec::new();
        let widths = self.widths();
        let mut menu = Menu::new(self.key.to_owned())
            .trigger(el().size(0.0, 0.0))
            .open(true)
            .at(self.at.x, self.at.y)
            .min_width(widths[0]);
        let dismiss = self.on_dismiss.clone();
        menu = menu.on_close(move |s: &mut S| dismiss(s));
        let mut entries: Vec<MenuEntry> =
            self.header.iter().cloned().map(MenuEntry::Label).collect();
        let body = self.entries(self.sections, &[], self.key, self.open, &mut rows);
        if !entries.is_empty() && !body.is_empty() {
            entries.push(MenuEntry::Separator);
        }
        // The header's separator shifts every row's index by one, so number the rows again.
        let shift = entries.len();
        entries.extend(body);
        let rows: Rc<Vec<Row>> = Rc::new(renumber(rows, self.key, shift));
        for e in entries {
            menu = match e {
                MenuEntry::Item(i) => menu.item(*i),
                MenuEntry::Separator => menu.separator(),
                MenuEntry::Label(l) => menu.group_label(l),
            };
        }
        let before = layer.len();
        let anchor = menu.build(kit, layer).absolute(0.0, 0.0);
        let surfaces = before + 2;
        debug_assert!(
            layer.len() > surfaces + self.open.len(),
            "rune-kit's layer order changed"
        );
        // One surface per level: the menu, then each open submenu.
        for (depth, &width) in widths.iter().enumerate() {
            let Some(slot) = layer.get_mut(surfaces + depth) else {
                break;
            };
            let surface = std::mem::replace(slot, el());
            *slot = self.adjust(surface, depth, rows.clone()).min_w(width);
        }
        anchor
    }

    /// The row that opened the submenu `depth` levels in.
    fn submenu_row(&self, depth: usize) -> Option<&super::model::Item> {
        let mut sections = self.sections;
        let mut item = None;
        for &i in self.open.get(..depth)? {
            let it = super::model::items(sections).nth(i)?;
            sections = it.submenu.as_deref()?;
            item = Some(it);
        }
        item
    }

    /// Each open level's width: its widest row whole, between [`MIN_WIDTH`] and
    /// [`MAX_WIDTH`]. rune-kit's rows shrink their labels to fit rather than widen the menu,
    /// and it draws submenus 160 pt wide.
    fn widths(&self) -> Vec<f32> {
        let mut out = vec![width(self.look, self.header, self.sections)];
        let mut sections = self.sections;
        for &i in self.open {
            let Some(sub) = super::model::items(sections)
                .nth(i)
                .and_then(|i| i.submenu.as_deref())
            else {
                break;
            };
            out.push(width(self.look, &[], sub));
            sections = sub;
        }
        out
    }

    /// The rune-kit entries for `sections`, under item key prefix `prefix`, numbered from 0:
    /// the top level's caller renumbers them past the header.
    fn entries(
        &self,
        sections: &[Section],
        path: &[usize],
        prefix: &str,
        open: &[usize],
        rows: &mut Vec<Row>,
    ) -> Vec<MenuEntry> {
        let mut out = Vec::new();
        let mut n = 0;
        for section in sections.iter().filter(|s| !s.items.is_empty()) {
            if !out.is_empty() {
                out.push(MenuEntry::Separator);
            }
            if let Some(label) = &section.label {
                out.push(MenuEntry::Label(label.clone()));
            }
            for item in &section.items {
                let key = format!("{prefix}-item{}", out.len());
                let mut here = path.to_vec();
                here.push(n);
                let mut mi = MenuItem::new(item.label.clone())
                    .danger(item.danger)
                    .disabled(!item.enabled);
                if let Some(icon) = item.icon {
                    mi = mi.icon(self.look.icon(icon));
                }
                if let Some(s) = &item.shortcut {
                    mi = mi.shortcut(s.clone());
                }
                if let Some(d) = &item.description {
                    mi = mi.description(d.clone());
                }
                if let Some(on) = item.checked {
                    mi = mi.checked(on);
                }
                match &item.submenu {
                    Some(sub) => {
                        let is_open = open.first() == Some(&n);
                        let deeper = if is_open { &open[1..] } else { &[] };
                        let sub_entries =
                            self.entries(sub, &here, &format!("{key}-sub"), deeper, rows);
                        let (toggle, close_to) = (self.on_submenu.clone(), self.close_to.clone());
                        let at = here.clone();
                        mi = mi
                            .submenu(sub_entries, is_open)
                            .on_select(move |s: &mut S| {
                                if is_open {
                                    close_to(s, at.len() - 1);
                                } else {
                                    toggle(s, at.clone(), false);
                                }
                            });
                    }
                    None => {
                        let select = self.on_select.clone();
                        let at = here.clone();
                        mi = mi.on_select_tx(move |s: &mut S, tx: &mut Tx| select(s, &at, tx));
                    }
                }
                let enterable = item
                    .submenu
                    .as_deref()
                    .is_some_and(|sub| super::model::items(sub).any(|i| i.enabled));
                rows.push(Row {
                    key,
                    path: here,
                    submenu: item.submenu.is_some(),
                    enterable,
                });
                out.push(MenuEntry::item(mi));
                n += 1;
            }
        }
        out
    }

    /// Strata's look and the keys rune-kit leaves out, on the surface `depth` levels in.
    fn adjust(&self, surface: El, depth: usize, rows: Rc<Vec<Row>>) -> El {
        let look = self.look;
        let kit = &look.kit;
        let mut e = surface
            .radius(look.round(RADIUS))
            // rune-kit's floating elevation already rings the surface in `border_base`, which
            // is the edge that shows; this lies under it, for a kit whose elevation has none.
            .border(1.0, kit.c.border_base);
        if look.glow.on {
            e = e.shadow(
                0.0,
                look.glow.radius,
                look.glow.color.alpha(look.glow.opacity),
            );
        }
        // The menu takes the host's name; a submenu its row's label.
        let name = if depth == 0 {
            Some(self.name.to_owned())
        } else {
            self.submenu_row(depth).map(|i| i.label.clone())
        };
        if let Some(name) = name {
            e = e.label(name);
        }
        if depth > 0
            && let Some(row) = rows.iter().find(|r| r.path == self.open[..depth])
        {
            // Left (or a click on its row) closes it: the keyboard goes back to its row.
            e = e.focus_return(row.key.clone());
        }
        let hands = Hands {
            open: self.on_submenu.clone(),
            entering: self.entering.clone(),
            close_to: self.close_to.clone(),
        };
        e.on_key(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| {
            menu_key(s, ev, tx, depth, &rows, &hands)
        })
    }
}

/// What a surface's keys can ask of the menu.
struct Hands<S> {
    open: Submenu<S>,
    entering: Entering<S>,
    close_to: CloseTo<S>,
}

/// The surface `depth` levels in heard `ev`:
///
/// - Right, Enter and Space on a submenu's row open it and send a Tab, which Rune's focus moves
///   into it, the next focus group in the menu's modal. (A submenu of its own modal would move
///   the keyboard too, but swallows a press outside it, so a click on the parent's rows would
///   run nothing.) A submenu with no sensitive row opens with the keyboard left on its row.
/// - Left in a submenu closes it. Escape anywhere is the top menu's modal's: it closes the
///   whole menu.
/// - Tab and Shift+Tab move as Down and Up do, wrapping, as GTK's menus move (Rune's Tab
///   leaves a focus group whole).
/// - Ctrl+A stays in the menu.
fn menu_key<S>(
    s: &mut S,
    ev: &KeyEvent,
    tx: &mut Tx,
    depth: usize,
    rows: &[Row],
    hands: &Hands<S>,
) -> bool {
    let m = ev.mods;
    let plain = !(m.ctrl || m.cmd || m.alt || m.logo || m.shift);
    if ev.is_command('a') {
        return true;
    }
    if ev.key == Key::Tab && (hands.entering)(s) {
        return false;
    }
    if ev.key == Key::Tab && !(m.ctrl || m.cmd || m.alt || m.logo) {
        let to = if m.shift { Key::Up } else { Key::Down };
        tx.send_key(KeyEvent {
            key: to,
            mods: Mods::default(),
        });
        return true;
    }
    if !plain {
        return false;
    }
    let focused = tx.focused().and_then(|k| rows.iter().find(|r| r.key == k));
    match &ev.key {
        Key::Right | Key::Enter => {}
        Key::Text(t) if t == " " => {}
        Key::Left if depth > 0 => {
            (hands.close_to)(s, depth - 1);
            return true;
        }
        // Escape in a submenu needs no arm: the top menu's modal takes it and closes the
        // whole menu.
        _ => return false,
    }
    // A surface hears only its own rows' keys: a submenu is the layer's, not a child of its
    // parent's surface.
    match focused {
        // A submenu with no sensitive row opens, so its rows show, but the keyboard stays on
        // its row: the Tab would find no row there and leave the menu, for the top level's
        // first row (Rune's `tab_order` takes the next group's selected or first member).
        Some(row) if row.submenu && !row.enterable => {
            (hands.open)(s, row.path.clone(), false);
            true
        }
        Some(row) if row.submenu => {
            if (hands.open)(s, row.path.clone(), true) {
                tx.send_key(KeyEvent {
                    key: Key::Tab,
                    mods: Mods::default(),
                });
            }
            true
        }
        _ => false,
    }
}

/// `rows` with the top level's keys moved on by `shift` entries (the header and its
/// separator). Submenu keys hang off their row's key, so they move with it.
fn renumber(rows: Vec<Row>, key: &str, shift: usize) -> Vec<Row> {
    if shift == 0 {
        return rows;
    }
    let prefix = format!("{key}-item");
    rows.into_iter()
        .map(|mut r| {
            let rest = &r.key[prefix.len()..];
            let digits = rest.chars().take_while(char::is_ascii_digit).count();
            let n: usize = rest[..digits].parse().unwrap_or(0);
            r.key = format!("{prefix}{}{}", n + shift, &rest[digits..]);
            r
        })
        .collect()
}

/// How wide a compact rune-kit menu must be to show `header` and `sections` whole: each
/// row's parts as `MenuItem::build` lays them out (5d421fd), plus the list's padding.
pub(super) fn width(look: &Look, header: &[String], sections: &[Section]) -> f32 {
    use rune::TextStyle;
    let k = &look.kit;
    let measure = |t: &str, style: TextStyle| rune::text::measure(t, &[], style, None).0.ceil();
    let hint = TextStyle {
        line_height: 11.0,
        ..k.ty.small_medium
    };
    // Rows: 12 pt before, 6 after, 8 between parts. Labels: 12 pt either side.
    let mut w: f32 = header
        .iter()
        .chain(sections.iter().filter_map(|s| s.label.as_ref()))
        .map(|l| 24.0 + measure(l, k.ty.small_medium))
        .fold(0.0, f32::max);
    for item in super::model::items(sections) {
        let mut text = measure(&item.label, k.ty.ui_compact);
        if let Some(d) = &item.description {
            text = text.max(measure(d, k.ty.small));
        }
        let mut row = 18.0 + text;
        if item.icon.is_some() {
            row += 14.0 + 8.0;
        }
        if let Some(s) = &item.shortcut {
            row += 8.0 + measure(s, hint);
        }
        if item.checked.is_some() {
            row += 8.0 + 16.0;
        }
        if item.submenu.is_some() {
            row += 8.0 + 16.0;
        }
        w = w.max(row);
    }
    // The list's 2 pt padding either side.
    (w + 4.0).clamp(MIN_WIDTH, MAX_WIDTH)
}

/// How tall a compact rune-kit menu of `header` and `sections` is, as `Menu` and
/// `MenuItem::build` lay it out (5d421fd): 28 pt rows and labels, a described row as tall as
/// its two lines and 6 pt above and below them, a 1 pt separator with 2 pt either side, and
/// 2 pt of padding top and bottom.
pub(super) fn height(look: &Look, header: &[String], sections: &[Section]) -> f32 {
    let ty = &look.kit.ty;
    let described = (12.0 + ty.ui_compact.line_px() + 2.0 + ty.small.line_px()).max(ROW);
    let sections: Vec<&Section> = sections.iter().filter(|s| !s.items.is_empty()).collect();
    let separators =
        sections.len().saturating_sub(1) + usize::from(!header.is_empty() && !sections.is_empty());
    let labels = header.len() + sections.iter().filter(|s| s.label.is_some()).count();
    let rows: f32 = sections
        .iter()
        .flat_map(|s| s.items.iter())
        .map(|i| {
            if i.description.is_some() {
                described
            } else {
                ROW
            }
        })
        .sum();
    4.0 + ROW * labels as f32 + rows + 5.0 * separators as f32
}

/// A compact row's height, and a group label's.
const ROW: f32 = 28.0;
