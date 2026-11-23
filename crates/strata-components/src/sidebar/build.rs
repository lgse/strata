// SPDX-License-Identifier: MIT

//! [`PlacesSidebar`]: the builder that lays out the places, PINNED and DEVICES.

use std::rc::Rc;

use rune::{Drag, El, Mods, Phase, PointerPress, ScrollSpec, Tx, col, el};
use strata_kit::look::Look;
use strata_kit::slots::sidebar as slot;

use super::data::{DeviceLine, ItemId, SidebarData, device_lines};
use super::drag::{Block, Landing};
use super::heading::heading;
use super::intent::{DropSite, Gap, Section, SidebarIntent};
use super::rows::{Ctx, Spec, device_row, folder_row, row_icon, trash_full, trash_icon};
use super::state::{SidebarState, Step};
use super::{ActionFn, Emit, drops};
use crate::contract::dnd::{Dnd, DropAction, DropHover, DropOffer};
use crate::scroll::{Bar, BarLook, anchor};

type DragFn<S> = Rc<dyn Fn(&mut S, &mut Drag, &mut Tx)>;

pub struct PlacesSidebar<'a, S> {
    key: String,
    state: &'a SidebarState,
    access: fn(&mut S) -> &mut SidebarState,
    data: SidebarData<'a>,
    dnd: Dnd<S>,
    content: &'static [&'static str],
    action: Option<ActionFn<S>>,
    reorder: bool,
    new_tabs: bool,
    tenxer: bool,
    confined_grip: bool,
    footer: Option<Box<dyn FnOnce(Step) -> El + 'a>>,
    trash: Option<Box<dyn FnOnce(El) -> El + 'a>>,
    background_drag: Option<DragFn<S>>,
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
            content: &[],
            action: None,
            reorder: false,
            new_tabs: true,
            tenxer: false,
            confined_grip: false,
            footer: None,
            trash: None,
            background_drag: None,
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

    pub fn content_types(mut self, mimes: &'static [&'static str]) -> Self {
        self.content = mimes;
        self
    }

    pub fn drop_action(
        mut self,
        f: impl Fn(&S, &ItemId, &DropOffer, Mods) -> Option<DropAction> + 'static,
    ) -> Self {
        self.action = Some(Rc::new(f));
        self
    }

    pub fn reorder(mut self, on: bool) -> Self {
        self.reorder = on;
        self
    }

    /// Whether a middle click opens a row in a new tab (default on; off in the file chooser).
    pub fn open_in_new_tab(mut self, on: bool) -> Self {
        self.new_tabs = on;
        self
    }

    pub fn tenxer(mut self, on: bool) -> Self {
        self.tenxer = on;
        self
    }

    pub fn confined_grip(mut self, on: bool) -> Self {
        self.confined_grip = on;
        self
    }

    /// Below the list, outside its scroll: Strata's update notice, built for the step shown.
    pub fn footer(mut self, f: impl FnOnce(Step) -> El + 'a) -> Self {
        self.footer = Some(Box::new(f));
        self
    }

    pub fn trash_icon(mut self, f: impl FnOnce(El) -> El + 'a) -> Self {
        self.trash = Some(Box::new(f));
        self
    }

    /// Hears every [`SidebarIntent`].
    pub fn on_intent(mut self, f: impl Fn(&mut S, SidebarIntent, &mut Tx) + 'static) -> Self {
        self.emit = Rc::new(f);
        self
    }

    pub fn on_background_drag(mut self, f: impl Fn(&mut S, &mut Drag, &mut Tx) + 'static) -> Self {
        self.background_drag = Some(Rc::new(f));
        self
    }

    pub fn build(self, look: &Look) -> El {
        let shown = self.state.width.shown();
        if self.state.last_shown.replace(Some(shown)) != Some(shown) || shown == Step::Hidden {
            self.state.scroll_bar.release();
        }
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
            content: self.content,
            action: self.action.as_ref(),
            reorder: self.reorder,
            rail,
            new_tabs: self.new_tabs,
            tenxer: self.tenxer,
            labels: self.data.labels,
        };
        let list = build_list(&ctx, &self.data, self.trash);
        let width = if rail {
            slot::rail_width(look)
        } else {
            self.state.width.pane.width()
        };
        let scroll = scroll_area(
            list,
            ScrollAreaSpec {
                key: &self.key,
                look,
                bar: &self.state.scroll_bar,
                rail,
                emit: &self.emit,
                background_drag: self.background_drag,
            },
        );
        let mut root = root
            .w(width)
            .fill(look.kit.c.bg_deep)
            .border_sides([0.0, 1.0, 0.0, 0.0], look.kit.c.border_base)
            .child(scroll);
        if let Some(f) = self.footer {
            root = root.child(f(shown));
        }
        if !rail {
            root = root.child(grip(
                &self.key,
                look,
                self.access,
                GripSpec {
                    width,
                    resizing: self.state.resizing,
                    confined: self.confined_grip,
                },
            ));
        }
        self.dnd
            .drop_zone_keyed(root, self.key.clone(), drops::background(self.access))
    }
}

fn build_list<S: 'static>(
    ctx: &Ctx<'_, S>,
    data: &SidebarData<'_>,
    trash_hook: Option<Box<dyn FnOnce(El) -> El + '_>>,
) -> El {
    let mut list = col()
        .key(format!("{}-list", ctx.key))
        .role("list")
        .focus_group()
        .w_full()
        .padding(ctx.look.space.xs)
        .child(places(ctx, data, trash_hook));
    if let Some(pinned) = pinned(ctx, data) {
        list = list.child(pinned);
    }
    if let Some(devices) = devices(ctx, data) {
        list = list.child(devices);
    }
    list
}

/// Full-sidebar trailing resize grip width: 1 pt right border plus `space.sm` (6 pt) inside.
pub(crate) fn full_grip_width(look: &Look) -> f32 {
    look.space.sm + 1.0
}

pub(crate) fn full_bar_edge(look: &Look) -> f32 {
    full_grip_width(look) + 1.0
}

pub(crate) fn full_row_trailing_pad(look: &Look) -> f32 {
    let clear = full_bar_edge(look) + strata_kit::slots::scroll::bar_width(look) + look.space.xxs
        - look.space.xs;
    clear.max(look.space.md)
}

fn sidebar_bar_look(look: &Look, rail: bool) -> BarLook {
    let mut bar_look = BarLook::app(look);
    bar_look.ends = look.space.xs;
    bar_look.edge = if rail {
        look.space.xxs
    } else {
        full_bar_edge(look)
    };
    bar_look
}

struct ScrollAreaSpec<'a, S> {
    key: &'a str,
    look: &'a Look,
    bar: &'a Bar,
    rail: bool,
    emit: &'a super::Emit<S>,
    background_drag: Option<DragFn<S>>,
}

fn scroll_area<S: 'static>(list: El, spec: ScrollAreaSpec<'_, S>) -> El {
    let scroll_key = format!("{}-scroll", spec.key);
    let bar_key = format!("{}-scrollbar", spec.key);
    let emit = spec.emit.clone();
    let scroll = col()
        .key(scroll_key.clone())
        .w_full()
        .h_full()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .on_scroll_offset(spec.bar.watch())
        .child(list)
        .on_secondary_press(move |s: &mut S, point: PointerPress, tx: &mut Tx| {
            emit(s, SidebarIntent::ContextMenu { item: None, point }, tx)
        });
    let viewport = match spec.background_drag {
        Some(drag) => scroll
            .on_drag(move |s: &mut S, d: &mut Drag, tx: &mut Tx| drag(s, d, tx))
            .focusable(false)
            .preserve_focus_on_press(true),
        None => scroll,
    };
    let bar_el = spec
        .bar
        .build::<S>(
            &bar_key,
            &scroll_key,
            sidebar_bar_look(spec.look, spec.rail),
        )
        .wheel_scrolls(scroll_key);
    col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .min_w(0.0)
        .clip_soft(0.0)
        .child(viewport)
        .child(anchor(0.0, 0.0, bar_el))
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
            description: p.description.as_deref(),
            reorder: Some((i, n)),
            lifted: ctx.state.places_reorder.dragging() == Some(i),
        };
        let intent: Rc<dyn Fn(usize, Landing) -> SidebarIntent> =
            Rc::new(move |from, landing| SidebarIntent::MovePlace {
                from,
                to: landing.place(from, n),
                landing,
            });
        block = block.child(folder_row(
            ctx,
            spec,
            icon,
            ctx.reorder
                .then_some((&ctx.state.places_reorder, Block::Places, intent)),
        ));
    }
    block
}

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
            description: p.description.as_deref(),
            reorder: Some((i, n)),
            lifted: !collapsed && ctx.state.pins_reorder.dragging() == Some(i),
        };
        let ids = ids.clone();
        let intent: Rc<dyn Fn(usize, Landing) -> SidebarIntent> =
            Rc::new(move |from, landing| SidebarIntent::MovePin {
                id: ids[from].clone(),
                from,
                to: landing.place(from, n),
                landing,
            });
        // Collapsed, the lone open pin has nowhere to move to.
        let reorder =
            (ctx.reorder && !collapsed).then_some((&ctx.state.pins_reorder, Block::Pins, intent));
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
    let band_h = (height + reach).max(closed + 2.0 * reach);
    let gap = gap_at(pins, i);
    let band_key = format!("{}-band-{i}", ctx.key);
    let band = el()
        .key(band_key.clone())
        .absolute(0.0, -reach)
        .w_full()
        .h(band_h);
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
        .paint_order(1)
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

#[derive(Clone, Copy)]
struct GripSpec {
    width: f32,
    resizing: bool,
    confined: bool,
}

fn grip<S: 'static>(
    key: &str,
    look: &Look,
    access: fn(&mut S) -> &mut SidebarState,
    spec: GripSpec,
) -> El {
    let w = if spec.confined {
        1.0
    } else {
        full_grip_width(look)
    };
    let x = spec.width - w;
    let line_x = w - 1.0;
    let rest = if spec.resizing { 1.0 } else { 0.0 };
    let line = el()
        .key(format!("{key}-grip-line"))
        .pin(Some(line_x), Some(0.0), None, Some(0.0))
        .w(1.0)
        .fill(look.kit.c.border_focus)
        .group_opacity(rest, 1.0)
        .opacity_motion(rune::Anim::INSTANT)
        .a11y_hidden()
        .inert();
    el().key(format!("{key}-grip"))
        .absolute(x, 0.0)
        .w(w)
        .h_full()
        .cursor(rune::Cursor::ResizeColumn)
        .hover_group()
        .hover_motion(rune::Anim::INSTANT, rune::Anim::INSTANT)
        .a11y_hidden()
        .child(line)
        .on_drag(move |s: &mut S, d: &mut Drag, tx: &mut Tx| {
            let state = access(s);
            match d.phase {
                Phase::Down => {
                    d.data = state.width.pane.width();
                    state.resizing = true;
                }
                Phase::Move => {
                    tx.instant();
                    state.width.pane.set_width(d.data + d.dx);
                }
                Phase::Up => {
                    state.resizing = false;
                    if d.cancelled() {
                        state.width.pane.set_width(d.data);
                    }
                }
            }
        })
        .on_double_press(move |s: &mut S| {
            let state = access(s);
            state.resizing = false;
            state.width.pane.reset_width();
        })
}
