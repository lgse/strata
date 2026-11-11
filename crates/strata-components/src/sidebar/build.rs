//! [`PlacesSidebar`]: the builder that lays out the places, PINNED and DEVICES.

use std::rc::Rc;

use rune::{Drag, El, Mods, Phase, PointerPress, ScrollSpec, Tx, col, el};
use strata_kit::look::Look;
use strata_kit::slots::sidebar as slot;

use super::data::{DeviceLine, ItemId, SidebarData, device_lines};
use super::heading::heading;
use super::intent::{DropSite, Gap, Section, SidebarIntent};
use super::rows::{Ctx, Spec, device_row, folder_row, row_icon, trash_full, trash_icon};
use super::state::{SidebarState, Step};
use super::{ActionFn, Emit, ReorderAccess, drops};
use crate::contract::dnd::{Dnd, DropAction, DropHover, DropOffer};

/// The places sidebar: built-in places, then PINNED and DEVICES, in three widths.
///
/// ```ignore
/// PlacesSidebar::new(&s.sidebar, |s: &mut App| &mut s.sidebar, data)
///     .dnd(dnd.clone())
///     .drop_action(|s: &App, item, offer, mods| s.drop_action(item, offer, mods))
///     .footer(|step| update_notice(step))
///     .on_intent(|s: &mut App, intent, tx| s.sidebar_intent(intent, tx))
///     .build(&look)
/// ```
///
/// Keys, under `key` (default `places`): `<key>-list`, `<key>-place-<id>`, `<key>-pin-<id>`,
/// `<key>-device-<id>` with `-lock` and `-eject`, `<key>-gap-<i>` and its drop band
/// `<key>-band-<i>`, `<key>-heading-pinned`, `<key>-heading-devices`, `<key>-divider-<g>`,
/// `<key>-grip` and [`super::trash_target_key`].
pub struct PlacesSidebar<'a, S> {
    key: String,
    state: &'a SidebarState,
    access: fn(&mut S) -> &mut SidebarState,
    data: SidebarData<'a>,
    dnd: Dnd<S>,
    action: Option<ActionFn<S>>,
    reorder: Option<(ReorderAccess<S>, ReorderAccess<S>)>,
    new_tabs: bool,
    tenxer: bool,
    footer: Option<Box<dyn FnOnce(Step) -> El + 'a>>,
    trash: Option<Box<dyn FnOnce(El) -> El + 'a>>,
    emit: Emit<S>,
}

impl<'a, S: 'static> PlacesSidebar<'a, S> {
    /// A sidebar of `data`, with its state `state`, which `access` finds in the app's state.
    pub fn new(
        state: &'a SidebarState,
        access: fn(&mut S) -> &mut SidebarState,
        data: SidebarData<'a>,
    ) -> Self {
        PlacesSidebar {
            key: super::DEFAULT_KEY.into(),
            state,
            access,
            data,
            dnd: Dnd::rune(),
            action: None,
            reorder: None,
            new_tabs: true,
            tenxer: false,
            footer: None,
            trash: None,
            emit: Rc::new(|_, _, _| {}),
        }
    }

    /// The key every element's key starts with (default `places`).
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = key.into();
        self
    }

    /// How drop zones attach (default [`Dnd::rune`]).
    pub fn dnd(mut self, dnd: Dnd<S>) -> Self {
        self.dnd = dnd;
        self
    }

    /// The action a drag over a row would take there, or `None` to refuse it: the host's
    /// copy-or-move rule and volume lookup, and whether the Trash takes the selection.
    /// Without it, rows use [`crate::contract::dnd::choose_action`] on an unknown volume.
    pub fn drop_action(
        mut self,
        f: impl Fn(&S, &ItemId, &DropOffer, Mods) -> Option<DropAction> + 'static,
    ) -> Self {
        self.action = Some(Rc::new(f));
        self
    }

    /// Lets the places and the pins be dragged into a new order. `places` and `pins` find
    /// [`SidebarState::places_reorder`] and [`SidebarState::pins_reorder`]: rune-kit's
    /// `SidebarRow::reorder` takes a plain `fn`, which can't go through the sidebar's own
    /// accessor. Without this, rows don't reorder.
    pub fn reorder(mut self, places: ReorderAccess<S>, pins: ReorderAccess<S>) -> Self {
        self.reorder = Some((places, pins));
        self
    }

    /// Whether a middle click opens a row in a new tab (default on; off in the file chooser).
    pub fn open_in_new_tab(mut self, on: bool) -> Self {
        self.new_tabs = on;
        self
    }

    /// 10xer mode's keys in the sidebar: `j`/`k` step, `l` activates, `h`, Left and
    /// Backspace go back to the files.
    pub fn tenxer(mut self, on: bool) -> Self {
        self.tenxer = on;
        self
    }

    /// Below the list, outside its scroll: Strata's update notice, built for the step shown.
    pub fn footer(mut self, f: impl FnOnce(Step) -> El + 'a) -> Self {
        self.footer = Some(Box::new(f));
        self
    }

    /// Dresses the Trash icon's box, keyed [`super::trash_target_key`]: C23's pulse, shake or
    /// landing slot.
    pub fn trash_icon(mut self, f: impl FnOnce(El) -> El + 'a) -> Self {
        self.trash = Some(Box::new(f));
        self
    }

    /// Hears every [`SidebarIntent`].
    pub fn on_intent(mut self, f: impl Fn(&mut S, SidebarIntent, &mut Tx) + 'static) -> Self {
        self.emit = Rc::new(f);
        self
    }

    pub fn build(self, look: &Look) -> El {
        let shown = self.state.width.shown();
        let root = col()
            .key(self.key.clone())
            .h_full()
            .min_h(0.0)
            .motion(look.kit.motion.disclosure);
        if shown == Step::Hidden {
            return root.w(0.0).a11y_hidden();
        }
        let rail = shown == Step::Rail;
        let ctx = Ctx {
            key: &self.key,
            look,
            state: self.state,
            access: self.access,
            emit: self.emit.clone(),
            dnd: &self.dnd,
            action: self.action.as_ref(),
            reorder: self.reorder,
            rail,
            new_tabs: self.new_tabs,
            tenxer: self.tenxer,
        };
        let pad = look.space.xs;
        let mut list = col()
            .key(format!("{}-list", self.key))
            .role("list")
            .label("Places")
            .focus_group()
            .w_full()
            .px(pad)
            .pb(look.space.md)
            .child(places(&ctx, &self.data, self.trash));
        if let Some(pinned) = pinned(&ctx, &self.data) {
            list = list.child(pinned);
        }
        if let Some(devices) = devices(&ctx, &self.data) {
            list = list.child(devices);
        }
        let width = if rail {
            slot::rail_width(look)
        } else {
            self.state.width.pane.width()
        };
        let mut root = root.w(width).child(
            col()
                .key(format!("{}-scroll", self.key))
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(list)
                .on_secondary_press({
                    let emit = self.emit.clone();
                    move |s: &mut S, point: PointerPress, tx: &mut Tx| {
                        emit(s, SidebarIntent::ContextMenu { item: None, point }, tx)
                    }
                }),
        );
        if let Some(f) = self.footer {
            root = root.child(f(shown));
        }
        if !rail {
            root = root.child(grip(&self.key, look, self.access, width));
        }
        self.dnd
            .drop_zone_keyed(root, self.key.clone(), drops::background(self.access))
    }
}

/// The built-in places block: one reorderable list.
fn places<S: 'static>(
    ctx: &Ctx<'_, S>,
    data: &SidebarData<'_>,
    trash_hook: Option<Box<dyn FnOnce(El) -> El + '_>>,
) -> El {
    let look = ctx.look;
    let n = data.places.len();
    let mut hook = trash_hook;
    let mut block = col()
        .key(format!("{}-places", ctx.key))
        .w_full()
        .gap(slot::row_gap(look));
    for (i, p) in data.places.iter().enumerate() {
        let item = p.item();
        let selected = data.current == Some(&item);
        let icon = match trash_full(p.kind) {
            Some(full) => trash_icon(ctx.key, look, full, selected, hook.take()),
            None => row_icon(look, &p.icon, p.tint, selected),
        };
        let spec = Spec {
            item,
            label: &p.label,
            selected,
            accepts_drops: p.accepts_drops,
            new_tab: ctx.new_tabs,
            keycap: p.keycap.as_deref(),
            reorder: Some((i, n)),
            lifted: !ctx.rail && ctx.state.places_reorder.dragging() == Some(i),
        };
        let intent: Rc<dyn Fn(usize, usize) -> SidebarIntent> =
            Rc::new(|from, to| SidebarIntent::MovePlace { from, to });
        block = block.child(folder_row(
            ctx,
            spec,
            icon,
            ctx.reorder
                .map(|(places, _)| (&ctx.state.places_reorder, places, intent)),
        ));
    }
    block
}

/// PINNED: its heading, then a gap before each pin and after the last. Collapsed, only the
/// open pin shows, between its two gaps. With nothing pinned the section shows only while a
/// drag is over the sidebar, expanded, with one gap.
fn pinned<S: 'static>(ctx: &Ctx<'_, S>, data: &SidebarData<'_>) -> Option<El> {
    let look = ctx.look;
    let pins = data.pins;
    let n = pins.len();
    let dragging = ctx.state.dragging();
    if n == 0 && !dragging {
        return None;
    }
    let collapsed = ctx.state.pinned_collapsed && n > 0;
    let mut section = col()
        .key(format!("{}-section-pinned", ctx.key))
        .role("group")
        .label(Section::Pinned.title())
        .w_full()
        .motion(look.kit.motion.disclosure);
    if ctx.rail {
        section = section.child(separator(ctx));
    }
    section = section.child(heading(ctx, Section::Pinned, n, collapsed));
    let shown: Vec<usize> = if collapsed {
        data.current_pin().into_iter().collect()
    } else {
        (0..n).collect()
    };
    let verdict = ctx.state.gap_verdict();
    let ids: Rc<[String]> = pins.iter().map(|p| p.id.clone()).collect();
    for (k, &i) in shown.iter().enumerate() {
        if k == 0 {
            section = section.child(gap(ctx, pins, i, verdict));
        }
        let p = &pins[i];
        let item = p.item();
        let selected = data.current == Some(&item);
        let spec = Spec {
            item,
            label: &p.label,
            selected,
            accepts_drops: p.accepts_drops,
            new_tab: ctx.new_tabs,
            keycap: p.keycap.as_deref(),
            reorder: Some((i, n)),
            lifted: !ctx.rail && !collapsed && ctx.state.pins_reorder.dragging() == Some(i),
        };
        let ids = ids.clone();
        let intent: Rc<dyn Fn(usize, usize) -> SidebarIntent> =
            Rc::new(move |from, to| SidebarIntent::MovePin {
                id: ids[from].clone(),
                from,
                to,
            });
        // Collapsed, the lone open pin has nowhere to move to.
        let reorder = ctx
            .reorder
            .filter(|_| !collapsed)
            .map(|(_, pins)| (&ctx.state.pins_reorder, pins, intent));
        let icon = row_icon(look, &p.icon, p.tint, selected);
        section = section.child(folder_row(ctx, spec, icon, reorder));
        section = section.child(gap(ctx, pins, i + 1, verdict));
    }
    if shown.is_empty() && n == 0 {
        section = section.child(gap(ctx, pins, 0, verdict));
    }
    Some(section)
}

/// The gap at full-list index `i`, with the pins on either side.
pub(crate) fn gap_at(pins: &[super::Pin], i: usize) -> Gap {
    Gap {
        index: i,
        before: i
            .checked_sub(1)
            .and_then(|b| pins.get(b))
            .map(|p| p.id.clone()),
        after: pins.get(i).map(|p| p.id.clone()),
    }
}

/// The insertion gap at full-list index `i`: the space between two rows, with a drop band
/// reaching over each neighbour's edge. Taking a drag, it opens to `sidebar.gap.open` and
/// draws the insertion line. Hidden from assistive technology: it is a drop target only.
fn gap<S: 'static>(
    ctx: &Ctx<'_, S>,
    pins: &[super::Pin],
    i: usize,
    verdict: super::PinVerdict,
) -> El {
    let look = ctx.look;
    let open = matches!(
        ctx.state.drop_over(),
        Some((DropSite::Gap(g), DropHover::Accepting)) if *g == i
    ) && ctx.state.pin_check.shows(verdict);
    let closed = slot::row_gap(look);
    let height = if open { slot::gap_open(look) } else { closed };
    let reach = slot::gap_reach(look);
    let gap = gap_at(pins, i);
    let band_key = format!("{}-band-{i}", ctx.key);
    let band = el()
        .key(band_key.clone())
        .absolute(0.0, -reach)
        .w_full()
        .h(height + 2.0 * reach)
        .raise();
    let band = ctx.dnd.drop_zone_keyed(
        band,
        band_key,
        drops::gap(ctx.access, &ctx.emit, gap, verdict, &ctx.state.pin_check),
    );
    let mut g = col()
        .key(format!("{}-gap-{i}", ctx.key))
        .w_full()
        .h(height)
        .px(look.space.xs)
        .justify_center()
        .motion(look.kit.motion.disclosure)
        .a11y_hidden();
    if open {
        g = g.child(
            el().key("line")
                .h(closed)
                .w_full()
                .radius(closed / 2.0)
                .fill(look.kit.c.bg_accent),
        );
    }
    g.child(band)
}

/// DEVICES: its heading, then the rows, with a divider between drive groups. Collapsed, no
/// row shows, not even the open device's.
fn devices<S: 'static>(ctx: &Ctx<'_, S>, data: &SidebarData<'_>) -> Option<El> {
    let look = ctx.look;
    let n = data.device_count();
    if n == 0 {
        return None;
    }
    let collapsed = ctx.state.devices_collapsed;
    let mut section = col()
        .key(format!("{}-section-devices", ctx.key))
        .role("group")
        .label(Section::Devices.title())
        .w_full()
        .gap(slot::row_gap(look))
        .motion(look.kit.motion.disclosure);
    if ctx.rail {
        section = section.child(separator(ctx));
    }
    section = section.child(heading(ctx, Section::Devices, n, collapsed));
    if collapsed {
        return Some(section);
    }
    for line in device_lines(data.devices) {
        section = section.child(match line {
            DeviceLine::Divider(g) => divider(ctx, g),
            DeviceLine::Device(d) => device_row(ctx, d, data.current == Some(&d.item())),
        });
    }
    Some(section)
}

/// A drive divider: a line in `sidebar.divider.color`, inset `sidebar.divider.inset` from the
/// full sidebar's sides (`space.xxs` in the rail), with `space.xs` above and below.
fn divider<S>(ctx: &Ctx<'_, S>, g: usize) -> El {
    let look = ctx.look;
    let pad = look.space.xs;
    let inset = if ctx.rail {
        look.space.xxs
    } else {
        slot::divider_inset(look)
    };
    el().key(format!("{}-divider-{g}", ctx.key))
        .h(1.0)
        .w_full()
        .my(look.space.xs)
        .px((inset - pad).max(0.0))
        .child(el().w_full().h(1.0).fill(slot::divider_color(look)))
        .a11y_hidden()
}

/// The rail's separator above a section, which the full sidebar's heading rule replaces.
fn separator<S>(ctx: &Ctx<'_, S>) -> El {
    let look = ctx.look;
    el().h(1.0)
        .w_full()
        .my(look.space.sm)
        .fill(slot::divider_color(look))
        .a11y_hidden()
}

/// The full sidebar's resize grip on its trailing edge. A double click resets the width.
fn grip<S: 'static>(
    key: &str,
    look: &Look,
    access: fn(&mut S) -> &mut SidebarState,
    width: f32,
) -> El {
    let reach = look.space.xs;
    el().key(format!("{key}-grip"))
        .absolute(width - reach, 0.0)
        .w(2.0 * reach)
        .h_full()
        .cursor(rune::Cursor::ResizeColumn)
        .a11y_hidden()
        .on_drag(move |s: &mut S, d: &mut Drag, tx: &mut Tx| {
            let pane = &mut access(s).width.pane;
            match d.phase {
                Phase::Down => d.data = pane.width(),
                Phase::Move => {
                    tx.instant();
                    pane.set_width(d.data + d.dx);
                }
                Phase::Up => {
                    if d.cancelled() {
                        pane.set_width(d.data);
                    }
                }
            }
        })
        .on_double_press(move |s: &mut S| access(s).width.pane.reset_width())
}
