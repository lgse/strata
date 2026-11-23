// SPDX-License-Identifier: MIT

use std::borrow::Cow;
use std::rc::Rc;

use rune::{El, Key, KeyEvent, Mods, TextStyle, Tx, col, el, icon, row, text};
use rune_kit::{Palette, Placement, popover};
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use super::model::{Item, Section};
use crate::contract::Point;
use crate::location_bar::elide_middle;
use crate::scroll::{Bar, BarLook, anchor};

/// Strata's menu corners before roundness (`.item-context-menu`, 9 px).
pub(super) const RADIUS: f32 = 9.0;
/// Menu row highlight corners before roundness (`.item-context-option`, 5 px).
const ROW_RADIUS: f32 = 5.0;
/// Least menu width in window points.
pub(super) const MIN_WIDTH: f32 = 160.0;
/// The widest a menu grows to show its rows whole; past it, labels end in an ellipsis.
pub(super) const MAX_WIDTH: f32 = 360.0;
pub(super) const HEADER_CHARS: usize = 30;
/// A compact row's height, and a group label's.
const ROW: f32 = 28.0;
/// Row left padding, in points.
const ROW_PL: f32 = 12.0;
const ROW_PR: f32 = 14.0;
/// Gap between a row's leading icon, label, shortcut, indicator, and chevron.
const ROW_GAP: f32 = 8.0;
/// List padding inside the menu frame, in points.
const LIST_PAD: f32 = 2.0;
const ICON_SIZE: f32 = 15.0;
/// Trailing submenu chevron size, in points.
const CHEVRON_SIZE: f32 = 14.0;
/// Trailing check indicator slot size, in points.
const INDICATOR_SIZE: f32 = 16.0;
const BAR_INSET: f32 = 2.0;

fn shown(line: &str) -> Cow<'_, str> {
    elide_middle(line, HEADER_CHARS)
}

/// Whether the menu draws a line of `header` shortened.
fn shortens(header: &[String]) -> bool {
    header.iter().any(|l| matches!(shown(l), Cow::Owned(_)))
}

type Select<S> = Rc<dyn Fn(&mut S, &[usize], &mut Tx)>;
type Submenu<S> = Rc<dyn Fn(&mut S, Vec<usize>)>;
type CloseTo<S> = Rc<dyn Fn(&mut S, usize)>;
type Dismiss<S> = Rc<dyn Fn(&mut S, &mut Tx)>;

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
    /// The top-level menu's width, in window points.
    pub top_width: f32,
    /// Keeps underlying selectable text non-inert while open (omits Rune's modal trap).
    pub keeps_text_selection: bool,
    pub scroll_bars: Vec<Bar>,
    pub look: &'a Look,
    /// An item without a submenu was chosen: its path.
    pub on_select: Select<S>,
    /// Open the submenu at a path.
    pub on_submenu: Submenu<S>,
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

/// Per-surface parameters passed while building a menu level and its open submenus.
struct SurfaceSpec<'a> {
    prefix: &'a str,
    depth: usize,
    name: &'a str,
    header: &'a [String],
    sections: &'a [Section],
    path: &'a [usize],
    open: &'a [usize],
    width: f32,
}

/// Per-row parameters passed while building a single menu item and its press handler.
struct RowSpec<'a> {
    item: &'a Item,
    key: &'a str,
    path: &'a [usize],
    is_open: bool,
    has_icons: bool,
}

/// Visual state flags for styling a menu row's highlight and ink.
#[derive(Clone, Copy)]
struct RowTone {
    is_open: bool,
    danger: bool,
    disabled: bool,
}

/// Parameters for walking one menu level while collecting [`Row`] metadata.
struct CollectScope<'a> {
    header: &'a [String],
    sections: &'a [Section],
    path: &'a [usize],
    prefix: &'a str,
    open: &'a [usize],
}

/// Parameters for mounting an open submenu surface beside its owning row.
struct SubmenuMount<'a> {
    parent: &'a SurfaceSpec<'a>,
    row: &'a RowSpec<'a>,
    rows: &'a Rc<Vec<Row>>,
}

impl<S: 'static> Render<'_, S> {
    pub fn build(self, layer: &mut Vec<El>) -> El {
        let kit = &self.look.kit;
        let anchor_key = format!("{}-trigger", self.key);
        let trigger = row()
            .key(anchor_key.clone())
            .child(el().size(0.0, 0.0))
            .absolute(0.0, 0.0);

        let point_key = format!("{}-point", self.key);
        layer.push(
            el().key(point_key.clone())
                .absolute(self.at.x, self.at.y)
                .size(0.0, 0.0),
        );
        let scrim_dismiss = self.on_dismiss.clone();
        let mut scrim = el()
            .key(format!("{}-scrim", self.key))
            .role("none")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .keeps_text_selection()
            .on_press_tx(move |s: &mut S, tx: &mut Tx| scrim_dismiss(s, tx));
        if self.keeps_text_selection {
            let scrim_secondary = self.on_dismiss.clone();
            scrim =
                scrim.on_secondary_press(move |s: &mut S, _, tx: &mut Tx| scrim_secondary(s, tx));
        }
        layer.push(scrim);

        let mut rows = Vec::new();
        let scope = CollectScope {
            header: self.header,
            sections: self.sections,
            path: &[],
            prefix: self.key,
            open: self.open,
        };
        collect_rows(&scope, &mut rows);
        let rows = Rc::new(rows);

        let mut subs = Vec::new();
        let top_spec = SurfaceSpec {
            prefix: self.key,
            depth: 0,
            name: self.name,
            header: self.header,
            sections: self.sections,
            path: &[],
            open: self.open,
            width: self.top_width,
        };
        let top = self.build_surface(&top_spec, &rows, &mut subs);
        let top = if self.keeps_text_selection {
            top
        } else {
            let modal_dismiss = self.on_dismiss.clone();
            top.focus_return(anchor_key)
                .modal_tx(true, move |s: &mut S, tx: &mut Tx| modal_dismiss(s, tx))
        };
        layer.push(popover::anchored(
            kit,
            top,
            &point_key,
            Placement::BottomStart,
            0.0,
        ));
        layer.extend(subs);
        trigger
    }

    fn build_surface(&self, spec: &SurfaceSpec<'_>, rows: &Rc<Vec<Row>>, subs: &mut Vec<El>) -> El {
        let look = self.look;
        let kit = &look.kit;
        let list = self.build_list(spec, rows, subs);
        let bar = self
            .scroll_bars
            .get(spec.depth)
            .cloned()
            .unwrap_or_else(super::state::menu_bar);
        let body = scroll_body::<S>(spec.prefix, &bar, look, list);

        let frame = col()
            .key(format!("{}-menu", spec.prefix))
            .role("menu")
            .focus_group()
            .min_w(spec.width)
            .w(spec.width)
            .items_stretch();
        let mut surface = kit
            .elevate(
                frame.radius(look.round(RADIUS)).fill(kit.c.layer_01),
                &kit.elevation.floating,
            )
            .border(1.0, look.transparency.frosted_border(kit.c.border_base))
            .label(spec.name.to_owned())
            .keeps_text_selection()
            .child(body);
        if look.transparency.frosted_glass {
            surface = surface.backdrop_blur(look.transparency.backdrop_blur);
        }
        if look.glow.on {
            surface = surface.shadow(
                0.0,
                look.glow.radius,
                look.glow.color.alpha(look.glow.opacity),
            );
        }
        if spec.depth == 0 && shortens(spec.header) {
            surface = surface.description(spec.header.join("\n"));
        }
        if spec.depth > 0
            && let Some(owner) = rows.iter().find(|r| r.path == self.open[..spec.depth])
        {
            // Left (or a click on its row) closes it: the keyboard goes back to its row.
            surface = surface.focus_return(owner.key.clone());
        }
        if self.keeps_text_selection
            && spec.depth == 0
            && !super::model::items(spec.sections).any(|i| i.enabled)
        {
            surface = surface.focusable(true).no_focus_ring();
        }
        let depth = spec.depth;
        let rows = rows.clone();
        let hands = Hands {
            open: self.on_submenu.clone(),
            close_to: self.close_to.clone(),
            dismiss: self.keeps_text_selection.then(|| self.on_dismiss.clone()),
        };
        surface.on_key(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| {
            menu_key(s, ev, tx, depth, &rows, &hands)
        })
    }

    /// Builds the rows, labels, and separators of one menu surface.
    fn build_list(&self, spec: &SurfaceSpec<'_>, rows: &Rc<Vec<Row>>, subs: &mut Vec<El>) -> El {
        let has_body = spec.sections.iter().any(|s| !s.items.is_empty());
        let has_icons = super::model::items(spec.sections).any(|i| i.icon.is_some());
        let (header_els, mut entry_index) = header_entries(self.look, spec.header, has_body);
        let mut list = col()
            .key("rows")
            .items_stretch()
            .padding(LIST_PAD)
            .children(header_els);
        let mut item_index = 0usize;

        for (sec_i, section) in spec
            .sections
            .iter()
            .filter(|s| !s.items.is_empty())
            .enumerate()
        {
            if sec_i > 0 {
                list = list.child(separator_row(self.look, entry_index));
                entry_index += 1;
            }
            if let Some(label) = &section.label {
                list = list.child(label_row(self.look, entry_index, label.clone()));
                entry_index += 1;
            }
            for item in &section.items {
                let item_key = format!("{}-item{entry_index}", spec.prefix);
                let mut here = spec.path.to_vec();
                here.push(item_index);
                let is_open = item.submenu.is_some() && spec.open.first() == Some(&item_index);
                let row_spec = RowSpec {
                    item,
                    key: &item_key,
                    path: &here,
                    is_open,
                    has_icons,
                };
                list = list.child(self.build_item(&row_spec));
                if is_open {
                    let mount = SubmenuMount {
                        parent: spec,
                        row: &row_spec,
                        rows,
                    };
                    self.mount_submenu(&mount, subs);
                }
                entry_index += 1;
                item_index += 1;
            }
        }
        list
    }

    /// Mounts an open submenu surface anchored to its owning row and appends deeper submenus.
    fn mount_submenu(&self, mount: &SubmenuMount<'_>, subs: &mut Vec<El>) {
        let Some(sub) = mount.row.item.submenu.as_deref() else {
            return;
        };
        let sub_prefix = format!("{}-sub", mount.row.key);
        let sub_spec = SurfaceSpec {
            prefix: &sub_prefix,
            depth: mount.parent.depth + 1,
            name: &mount.row.item.label,
            header: &[],
            sections: sub,
            path: mount.row.path,
            open: &mount.parent.open[1..],
            width: width(self.look, &[], sub),
        };
        let mut deeper = Vec::new();
        let sub_surface = self.build_surface(&sub_spec, mount.rows, &mut deeper);
        subs.push(popover::anchored(
            &self.look.kit,
            sub_surface,
            mount.row.key,
            Placement::RightStart,
            0.0,
        ));
        subs.extend(deeper);
    }

    fn build_item(&self, row_spec: &RowSpec<'_>) -> El {
        let look = self.look;
        let item = row_spec.item;
        let disabled = !item.enabled;
        let tone = RowTone {
            is_open: row_spec.is_open,
            danger: item.danger && !disabled,
            disabled,
        };
        let mut e = row()
            .key(row_spec.key.to_owned())
            .role(item_role(item))
            .label(item.label.clone())
            .radius(look.round(ROW_RADIUS))
            .gap(ROW_GAP)
            .center_y();
        e = item_padding(e, item.description.is_some());
        e = item_a11y(e, item, row_spec.is_open);
        e = item_highlight(e, &look.kit.c, tone);
        e = e.child(item_content(look, item, tone.danger, row_spec.has_icons));
        e = append_trailing(e, look, item);
        e = apply_sensitivity(e, item, disabled);
        if disabled {
            return e;
        }
        self.attach_press(e, row_spec)
    }

    fn attach_press(&self, e: El, row_spec: &RowSpec<'_>) -> El {
        let at = row_spec.path.to_vec();
        let is_open = row_spec.is_open;
        match row_spec.item.submenu.as_deref() {
            Some(sub) => {
                let enterable = super::model::items(sub).any(|i| i.enabled);
                let sub_menu = format!("{}-sub-menu", row_spec.key);
                let (toggle, close_to) = (self.on_submenu.clone(), self.close_to.clone());
                e.on_press_tx(move |s: &mut S, tx: &mut Tx| {
                    if is_open && !tx.by_keys() {
                        close_to(s, at.len() - 1);
                    } else {
                        toggle(s, at.clone());
                        if tx.by_keys() && enterable {
                            tx.focus(sub_menu.clone(), true);
                        }
                    }
                })
            }
            None => {
                let select = self.on_select.clone();
                e.on_press_tx(move |s: &mut S, tx: &mut Tx| select(s, &at, tx))
            }
        }
    }
}

fn header_entries(look: &Look, header: &[String], has_body: bool) -> (Vec<El>, usize) {
    let mut out = Vec::with_capacity(header.len() + 1);
    for (index, line) in header.iter().enumerate() {
        out.push(label_row(look, index, shown(line).into_owned()));
    }
    if !header.is_empty() && has_body {
        out.push(separator_row(look, out.len()));
    }
    let next = out.len();
    (out, next)
}

fn item_role(item: &Item) -> &'static str {
    if item.checked.is_some() {
        "menuitemcheckbox"
    } else {
        "menuitem"
    }
}

fn item_padding(e: El, described: bool) -> El {
    if described {
        e.min_h(ROW).py(6.0).pl(ROW_PL).pr(ROW_PR)
    } else {
        e.h(ROW).pl(ROW_PL).pr(ROW_PR)
    }
}

fn item_a11y(mut e: El, item: &Item, is_open: bool) -> El {
    if let Some(on) = item.checked {
        e = e.checked(on);
    }
    if let Some(s) = &item.shortcut {
        e = e.shortcut_hint(s.clone());
    }
    if let Some(d) = &item.description {
        e = e.description(d.clone());
    }
    if item.submenu.is_some() {
        e = e.expanded(is_open);
    }
    e
}

fn item_highlight(e: El, c: &Palette, tone: RowTone) -> El {
    let hover = if tone.danger { c.danger.bg } else { c.hover };
    if tone.is_open {
        e.fill(hover)
    } else if !tone.disabled {
        // One highlight for pointer and keys, landing at once like NSMenu's.
        e.focus_highlight(hover)
    } else {
        e
    }
}

fn apply_sensitivity(mut e: El, item: &Item, disabled: bool) -> El {
    if disabled {
        e = e.layer_opacity(0.5);
    }
    e.focusable(!disabled)
        .focus_selected(item.checked == Some(true))
        .disabled(disabled)
}

/// Leading icon (or alignment spacer), label, and optional description inside a menu row.
fn item_content(look: &Look, item: &Item, danger: bool, has_icons: bool) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let ink = if danger { c.danger.fg } else { c.text_base };
    let mut content = row()
        .key("content")
        .grow(1.0)
        .shrink()
        .gap(ROW_GAP)
        .center_y()
        .a11y_hidden();
    if has_icons {
        content = content.child(leading_icon(look, item, danger));
    }
    let label = text(item.label.clone())
        .key("label")
        .font(kit.ty.ui_compact)
        .color(ink)
        .truncate();
    match &item.description {
        Some(d) => content.child(
            col()
                .key("text")
                .grow(1.0)
                .shrink()
                .gap(2.0)
                .child(label)
                .child(
                    text(d.clone())
                        .key("description")
                        .font(kit.ty.small)
                        .color(c.text_faint)
                        .truncate(),
                ),
        ),
        None => content.child(label),
    }
}

fn leading_icon(look: &Look, item: &Item, danger: bool) -> El {
    let c = &look.kit.c;
    match item.icon {
        Some(name) => {
            let color = if danger { c.danger.fg } else { c.icon_accent };
            icon(look.icon(name), ICON_SIZE).key("icon").color(color)
        }
        None => el().key("icon-spacer").size(ICON_SIZE, ICON_SIZE),
    }
}

/// Shortcut hint, check mark, and submenu chevron at the trailing edge of a menu row.
fn append_trailing(mut e: El, look: &Look, item: &Item) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    if let Some(s) = &item.shortcut {
        let st = TextStyle {
            line_height: 11.0,
            ..kit.ty.small_medium
        };
        e = e.child(
            text(s.clone())
                .key("shortcut")
                .font(st)
                .color(c.text_faint)
                .a11y_hidden(),
        );
    }
    if let Some(on) = item.checked {
        let mark = icon(look.icon(IconName::Check), 14.0)
            .color(c.icon_accent)
            .opacity(if on { 1.0 } else { 0.0 });
        e = e.child(
            el().key("indicator")
                .size(INDICATOR_SIZE, INDICATOR_SIZE)
                .center()
                .child(mark)
                .a11y_hidden(),
        );
    }
    if item.submenu.is_some() {
        e = e.child(
            icon(look.icon(IconName::ChevronRight), CHEVRON_SIZE)
                .key("chevron")
                .a11y_hidden()
                .mirror_in_rtl()
                .color(c.icon_muted),
        );
    }
    e
}

fn label_row(look: &Look, index: usize, label: String) -> El {
    let kit = &look.kit;
    row()
        .key(format!("label{index}"))
        .h(ROW)
        .pl(ROW_PL)
        .pr(ROW_PR)
        .center_y()
        .child(
            text(label)
                .font(kit.ty.small_medium)
                .color(kit.c.text_faint),
        )
}

fn separator_row(look: &Look, index: usize) -> El {
    el().key(format!("sep{index}"))
        .h(1.0)
        .mt(LIST_PAD)
        .mb(LIST_PAD)
        .ml(-LIST_PAD)
        .mr(-LIST_PAD)
        .fill(look.kit.c.border_muted)
        .a11y_hidden()
}

fn scroll_body<S: 'static>(prefix: &str, bar: &Bar, look: &Look, list: El) -> El {
    let scroll_key = format!("{prefix}-scroll");
    let bar_key = format!("{prefix}-scrollbar");
    let area_key = format!("{prefix}-scroll-area");
    let viewport = popover::fit_scroll(list)
        .key(scroll_key.clone())
        .on_scroll_offset(bar.watch());
    let mut bar_look = BarLook::app(look);
    bar_look.edge = BAR_INSET;
    bar_look.ends = BAR_INSET;
    let bar_el = bar
        .build::<S>(&bar_key, &scroll_key, bar_look)
        .wheel_scrolls(scroll_key);
    col()
        .key(area_key)
        .w_full()
        .shrink()
        .min_h(0.0)
        .child(viewport)
        .child(anchor(0.0, 0.0, bar_el))
}

fn collect_rows(scope: &CollectScope<'_>, rows: &mut Vec<Row>) {
    let has_body = scope.sections.iter().any(|s| !s.items.is_empty());
    let mut entry_index = scope.header.len() + usize::from(!scope.header.is_empty() && has_body);
    let mut item_index = 0usize;

    for (sec_i, section) in scope
        .sections
        .iter()
        .filter(|s| !s.items.is_empty())
        .enumerate()
    {
        entry_index += usize::from(sec_i > 0) + usize::from(section.label.is_some());
        for item in &section.items {
            let key = format!("{}-item{entry_index}", scope.prefix);
            let mut here = scope.path.to_vec();
            here.push(item_index);
            let enterable = item
                .submenu
                .as_deref()
                .is_some_and(|sub| super::model::items(sub).any(|i| i.enabled));
            if let Some(sub) = item.submenu.as_deref()
                && scope.open.first() == Some(&item_index)
            {
                let sub_prefix = format!("{key}-sub");
                let child_scope = CollectScope {
                    header: &[],
                    sections: sub,
                    path: &here,
                    prefix: &sub_prefix,
                    open: &scope.open[1..],
                };
                collect_rows(&child_scope, rows);
            }
            rows.push(Row {
                key,
                path: here,
                submenu: item.submenu.is_some(),
                enterable,
            });
            entry_index += 1;
            item_index += 1;
        }
    }
}

/// What a surface's keys can ask of the menu.
struct Hands<S> {
    open: Submenu<S>,
    close_to: CloseTo<S>,
    /// Present for non-modal (`keeps_text_selection`) menus so Escape closes the menu.
    dismiss: Option<Dismiss<S>>,
}

fn menu_key<S>(
    s: &mut S,
    ev: &KeyEvent,
    tx: &mut Tx,
    depth: usize,
    rows: &[Row],
    hands: &Hands<S>,
) -> bool {
    let focused = tx.focused().and_then(|k| rows.iter().find(|r| r.key == k));
    if let Some(dismiss) = &hands.dismiss
        && non_modal_key(s, ev, tx, focused.is_some(), dismiss)
    {
        return true;
    }
    let m = ev.mods;
    let plain = !(m.ctrl || m.cmd || m.alt || m.logo || m.shift);
    if ev.is_command('a') {
        return true;
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
    match &ev.key {
        Key::Right | Key::Enter => {}
        Key::Text(t) if t == " " => {}
        Key::Left if depth > 0 => {
            (hands.close_to)(s, depth - 1);
            return true;
        }
        _ => return false,
    }
    match focused {
        Some(row) if row.submenu => {
            (hands.open)(s, row.path.clone());
            if row.enterable {
                tx.focus(format!("{}-sub-menu", row.key), true);
            }
            true
        }
        _ => false,
    }
}

fn non_modal_key<S>(
    s: &mut S,
    ev: &KeyEvent,
    tx: &mut Tx,
    focused_row: bool,
    dismiss: &Dismiss<S>,
) -> bool {
    let m = ev.mods;
    if m.ctrl || m.cmd || m.alt || m.logo {
        return false;
    }
    match ev.key {
        Key::Escape => {
            if !tx.key_repeat() {
                dismiss(s, tx);
            }
            true
        }
        Key::Function(6) => true,
        Key::PageUp | Key::PageDown | Key::Home | Key::End if !focused_row => true,
        _ => false,
    }
}

pub(super) fn width(look: &Look, header: &[String], sections: &[Section]) -> f32 {
    width_with_min(look, header, sections, MIN_WIDTH)
}

/// Like [`width`], with a minimum width floor of at least `min_width` points.
pub(super) fn width_with_min(
    look: &Look,
    header: &[String],
    sections: &[Section],
    min_width: f32,
) -> f32 {
    let k = &look.kit;
    let measure = |t: &str, style: TextStyle| rune::text::measure(t, &[], style, None).0.ceil();
    let hint = TextStyle {
        line_height: 11.0,
        ..k.ty.small_medium
    };
    let row_pad = ROW_PL + ROW_PR;
    let has_icons = super::model::items(sections).any(|i| i.icon.is_some());
    let labels = sections.iter().filter_map(|s| s.label.as_deref());
    let mut w: f32 = header
        .iter()
        .map(|l| shown(l))
        .chain(labels.map(Cow::from))
        .map(|l| row_pad + measure(&l, k.ty.small_medium))
        .fold(0.0, f32::max);
    for item in super::model::items(sections) {
        let mut text = measure(&item.label, k.ty.ui_compact);
        if let Some(d) = &item.description {
            text = text.max(measure(d, k.ty.small));
        }
        let mut row = row_pad + text;
        if has_icons {
            row += ICON_SIZE + ROW_GAP;
        }
        if let Some(s) = &item.shortcut {
            row += ROW_GAP + measure(s, hint);
        }
        if item.checked.is_some() {
            row += ROW_GAP + INDICATOR_SIZE;
        }
        if item.submenu.is_some() {
            row += ROW_GAP + CHEVRON_SIZE;
        }
        w = w.max(row);
    }
    let floor = min_width.max(MIN_WIDTH);
    let scale = strata_kit::typography::factor(k.ty.ui_compact.size).max(1.0);
    let ceiling = (MAX_WIDTH * scale).round().max(floor);
    (w + 2.0 * LIST_PAD).clamp(floor, ceiling)
}

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
    2.0 * LIST_PAD + ROW * labels as f32 + rows + (1.0 + 2.0 * LIST_PAD) * separators as f32
}
