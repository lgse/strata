// SPDX-License-Identifier: MIT

//! [`BrowserTabs`]: Strata's GTK-matched horizontal tab strip with compact content-sized tabs,
//! graceful shrinking before horizontal scrolling, theme-aware accent colours, variable-width
//! drag-to-reorder, file/content drops, and folder-opening sidebar drags.

use std::hash::{Hash, Hasher};
use std::rc::Rc;

use rune::{
    Anim, Color, Cursor, Cx, Drag, El, Mods, Phase, ScrollFade, Spring, Tx, el, icon, row, text,
};
use rune_kit::shell::{TabDragState, TabTearMove, TabTearOut, TabWords, TearOutSpec};
use strata_kit::icons::IconName;
use strata_kit::look::Look;
use strata_kit::slots::tabs as slot;

use super::data::{LABEL_CHARS, Tab, TabId, TabsData, elide_end};
use super::drag::{PressSpec, Release};
use super::drops::{self, Access, ActionFn, Ctx, Emit, OpensFn, TabSpec};
use super::intent::{DropSite, TabOpen, TabsIntent, Via};
use super::new_tab::{self, TabsLabels};
use super::sizing::{self, CLOSE_BUTTON_SIZE};
use super::{BrowserTabsState, scroll_key, tab_key, wheel};
use crate::contract::dnd::{Dnd, DropAction, DropHover, DropOffer};

/// The 16 pt icon/badge box inside each tab.
const BADGE_BOX: f32 = 16.0;

/// How many tabs carry a number badge: Ctrl+Shift+1 to 9, then 0 for the tenth.
const BADGED: usize = 10;

/// Corner radius for subtle hover/drop/drag overlays, scaled by `look.roundness` (0 at sharp).
const OVERLAY_RADIUS: f32 = 4.0;

/// Corner radius for the close button, matching GTK's `.tab-close` (`border-radius: 5px`).
const CLOSE_RADIUS: f32 = 5.0;

/// Vertical inset above and below the 2 px accent insertion mark during an incoming sidebar drag.
const INSERTION_INSET: f32 = 6.0;

/// Width of the vertical accent insertion mark during an incoming sidebar drag.
const INSERTION_WIDTH: f32 = 2.0;

/// The strip: `BrowserTabs::new(&data, &s.tabs, |s| &mut s.tabs, |s| &mut s.tabs.drag)
/// .on_intent(..).build(look, cx)`. See the module docs for what the host does with it.
pub struct BrowserTabs<'a, S: 'static> {
    data: &'a TabsData<'a>,
    state: &'a BrowserTabsState,
    access: Access<S>,
    _drag: fn(&mut S) -> &mut TabDragState,
    dnd: Dnd<S>,
    action: Option<ActionFn<S>>,
    opens: Option<OpensFn<S>>,
    guard_none: bool,
    content: &'static [&'static str],
    spring: Option<f64>,
    tear: bool,
    badges: bool,
    new_tab: bool,
    labels: TabsLabels,
    words: TabWords,
    emit: Emit<S>,
}

impl<'a, S: 'static> BrowserTabs<'a, S> {
    /// The strip over `data`, with its `state` and the accessors that find them in the app's
    /// state.
    pub fn new(
        data: &'a TabsData<'a>,
        state: &'a BrowserTabsState,
        access: Access<S>,
        drag: fn(&mut S) -> &mut TabDragState,
    ) -> Self {
        BrowserTabs {
            data,
            state,
            access,
            _drag: drag,
            dnd: Dnd::rune(),
            action: None,
            opens: None,
            guard_none: false,
            content: &[],
            spring: Some(super::HOVER_DELAY),
            tear: false,
            badges: false,
            new_tab: true,
            labels: TabsLabels::default(),
            words: TabWords::default(),
            emit: Rc::new(|_, _, _| {}),
        }
    }

    /// The host's drag-and-drop binder (default Rune's).
    pub fn dnd(mut self, dnd: Dnd<S>) -> Self {
        self.dnd = dnd;
        self
    }

    /// The action a drag over tab `id` takes, or `None` to refuse it: W1's `choose_action`
    /// with the host's volume lookup for the tab's folder. Without it, W1's rule with an
    /// unknown volume.
    pub fn drop_action(
        mut self,
        f: impl Fn(&S, TabId, &DropOffer, Mods) -> Option<DropAction> + 'static,
    ) -> Self {
        self.action = Some(Rc::new(f));
        self
    }

    /// Which drags open tabs across the strip (`TabOpen::AtGap`): convenience wrapper for
    /// [`BrowserTabs::tab_open`].
    pub fn opens_tabs(mut self, f: impl Fn(&S, &DropOffer) -> bool + 'static) -> Self {
        self.opens = Some(Rc::new(move |s, offer| {
            f(s, offer).then_some(TabOpen::AtGap)
        }));
        self.guard_none = false;
        self
    }

    /// How an in-process drag opens tabs on the strip ([`TabOpen::AtGap`] for sidebar locations,
    /// [`TabOpen::AtEnd`] for folder-only view drags), or `None` when it transfers or is refused.
    pub fn tab_open(mut self, f: impl Fn(&S, &DropOffer) -> Option<TabOpen> + 'static) -> Self {
        self.opens = Some(Rc::new(f));
        self.guard_none = true;
        self
    }

    /// The content types a tab takes from other apps besides files, most wanted first (the
    /// host's content-drops policy: text, images, web addresses). Default none.
    pub fn content_types(mut self, mimes: &'static [&'static str]) -> Self {
        self.content = mimes;
        self
    }

    /// How long a file or content drag rests on a tab before it is selected (default
    /// [`super::HOVER_DELAY`]); `None` never selects one.
    pub fn spring_delay(mut self, seconds: Option<f64>) -> Self {
        self.spring = seconds;
        self
    }

    /// Lets a tab be dragged off the strip ([`TabsIntent::TearOut`], [`TabsIntent::TearMove`]).
    /// Off by default.
    pub fn tear_out(mut self, on: bool) -> Self {
        self.tear = on;
        self
    }

    /// Shows the number badges (1 to 9, then 0) on the first ten tabs, in place of their
    /// icons: while the host sees Ctrl+Shift held.
    pub fn badges(mut self, on: bool) -> Self {
        self.badges = on;
        self
    }

    /// Whether the strip ends in the New tab button (default true). With one tab, a host that
    /// hides the strip puts a [`super::NewTabButton`] in its header instead.
    pub fn new_tab_button(mut self, on: bool) -> Self {
        self.new_tab = on;
        self
    }

    pub fn labels(mut self, labels: TabsLabels) -> Self {
        self.labels = labels;
        self
    }

    /// Translated words for the strip's own chrome, including each tab's close button
    /// ([`TabWords::close`], default `"Close tab"`).
    pub fn words(mut self, words: TabWords) -> Self {
        self.words = words;
        self
    }

    pub fn on_intent(mut self, f: impl Fn(&mut S, TabsIntent, &mut Tx) + 'static) -> Self {
        self.emit = Rc::new(f);
        self
    }

    /// The strip's band, a row: the tabs, then the New tab button. A host appends what else
    /// the band holds after them with `.child` (Strata's window buttons, while the strip
    /// shows). A torn-out tab stays inside the strip's clip; see [`BrowserTabs::build_layered`].
    pub fn build(self, look: &Look, cx: &mut Cx<S>) -> El {
        self.assemble(look, cx, None)
    }

    /// The band, with a torn-out tab floating over the window in `layer`, which the view mounts
    /// at its root with `rune_kit::root`.
    pub fn build_layered(self, look: &Look, cx: &mut Cx<S>, layer: &mut Vec<El>) -> El {
        self.assemble(look, cx, Some(layer))
    }

    fn assemble(self, look: &Look, cx: &mut Cx<S>, mut layer: Option<&mut Vec<El>>) -> El {
        let key = self.state.key().to_owned();
        let scroll_k = scroll_key(&key);
        let list_k = format!("{key}-list");
        let band_h = slot::tab_height(look);
        let tab_gap = look.slots.tabs.tab_gap.unwrap_or(0.0);
        let trailing_gap = look.space.xs;
        let fade = slot::fade_width(look);
        let zones = self.drop_ctx();
        let (new_btn, trailing_w) = self.build_new_tab(&key, trailing_gap, &zones, (look, cx));
        let layout = self.compute_layout(&key, trailing_w, tab_gap, (look, cx));
        let incoming = self
            .state
            .reorder
            .resolve_incoming(&layout.measured, tab_gap);
        self.state.probe.built(fade, trailing_w, self.data.active);

        let tear_spec = (self.tear && self.data.tabs.len() > 1).then(TearOutSpec::default);
        let strip_ctx = StripCtx {
            key: &key,
            scroll_key: &scroll_k,
            band_h,
            cap: layout.cap,
            tab_gap,
            shrunk: layout.shrunk,
            measured: &layout.measured,
            tear: tear_spec,
            has_incoming: incoming.is_some(),
            zones: &zones,
            look,
        };
        let mut tab_els: Vec<El> = self
            .data
            .tabs
            .iter()
            .enumerate()
            .map(|(i, tab)| {
                let (tab_el, ghost) = self.build_tab(i, tab, &layout.labels[i], &strip_ctx);
                if let (Some(layer), Some(ghost)) = (layer.as_deref_mut(), ghost) {
                    layer.push(ghost);
                }
                tab_el
            })
            .collect();
        if let Some(slot_at) = incoming {
            let at = slot_at.min(tab_els.len());
            tab_els.insert(at, incoming_marker(&list_k, band_h, look));
        }

        let list = row()
            .key(list_k)
            .role("tablist")
            .label(self.labels.list.clone().into_owned())
            .focus_group()
            .h(band_h)
            .gap(tab_gap)
            .items_center()
            .children(tab_els);
        let scroll = row()
            .key(scroll_k)
            .h(band_h)
            .shrink()
            .min_w(0.0)
            .scroll_x()
            .scroll_fade(ScrollFade::strength(fade).linear().full_at(0.1))
            .child(list);
        let scroll = self.state.reorder.scroll.watch(scroll);

        let access = self.access;
        let root = row()
            .key(key.clone())
            .grow(1.0)
            .shrink()
            .min_w(0.0)
            .h(band_h)
            .gap(trailing_gap)
            .items_center()
            .child(scroll)
            .children(new_btn)
            .children(self.state.probe.root_markers())
            .on_hover_change(move |s: &mut S, inside, tx: &mut Tx| {
                if inside || !access(s).reorder.leave_strip() {
                    tx.live_only();
                }
            });
        let root =
            self.dnd
                .drop_zone_keyed(root, key.clone(), drops::opener(&zones, DropSite::Strip));

        let hairline = el()
            .key("hairline")
            .pin(Some(0.0), None, Some(0.0), Some(0.0))
            .h(1.0)
            .fill(look.kit.c.border_base)
            .a11y_hidden();

        row()
            .key(format!("{key}-band"))
            .w_full()
            .h(band_h)
            .pl(0.0)
            .pr(look.space.sm)
            .gap(look.space.sm)
            .items_center()
            .fill(slot::strip_fill(look))
            .child(hairline)
            .child(root)
    }

    fn drop_ctx(&self) -> Ctx<S> {
        Ctx {
            access: self.access,
            emit: self.emit.clone(),
            action: self.action.clone(),
            opens: self.opens.clone(),
            guard_none: self.guard_none,
            content: self.content,
            spring: self.spring,
            len: self.data.tabs.len(),
        }
    }

    fn build_new_tab(
        &self,
        key: &str,
        trailing_gap: f32,
        zones: &Ctx<S>,
        env: (&Look, &Cx<S>),
    ) -> (Option<El>, f32) {
        if !self.new_tab {
            return (None, 0.0);
        }
        let (look, cx) = env;
        let new_key = super::new_tab_key(key);
        let width = cx
            .layout_size(&new_key)
            .map_or(look.frame.control, |z| z[0]);
        let button = new_tab::button(
            zones,
            self.state,
            new_tab::ButtonSpec {
                dnd: &self.dnd,
                labels: &self.labels,
                key: new_key,
            },
            look,
        );
        (Some(button), width + trailing_gap)
    }

    fn compute_layout(
        &self,
        key: &str,
        trailing_w: f32,
        tab_gap: f32,
        env: (&Look, &Cx<S>),
    ) -> TabLayout {
        let (look, cx) = env;
        let min_w = sizing::min_cap(look);
        let max_w = sizing::max_cap(look);
        let labels: Vec<String> = self
            .data
            .tabs
            .iter()
            .map(|t| elide_end(&t.name, LABEL_CHARS).into_owned())
            .collect();
        let natural: Vec<f32> = labels
            .iter()
            .map(|l| sizing::estimate_tab_width(l, look, max_w))
            .collect();
        let available = cx.layout_size(key).map(|z| (z[0] - trailing_w).max(0.0));
        let computed = sizing::width_cap(&natural, available, tab_gap, (min_w, max_w));
        let cap = self
            .state
            .reorder
            .width_lock
            .map_or(computed, |locked| locked.clamp(min_w, max_w));
        let measured: Rc<[f32]> = self
            .data
            .tabs
            .iter()
            .enumerate()
            .map(|(i, tab)| {
                let k = tab_key(key, tab.id);
                cx.layout_size(&k)
                    .map(|z| z[0])
                    .unwrap_or_else(|| natural[i].min(cap))
            })
            .collect::<Vec<_>>()
            .into();
        TabLayout {
            labels,
            measured,
            cap,
            shrunk: cap < max_w - 0.5,
        }
    }

    fn build_tab(
        &self,
        index: usize,
        tab: &Tab<'_>,
        label: &str,
        sc: &StripCtx<'_, S>,
    ) -> (El, Option<El>) {
        let look = sc.look;
        let active = self.data.active == Some(tab.id);
        let drop_accepting =
            self.state.drop_hover() == Some((DropSite::Tab(tab.id), DropHover::Accepting));
        let highlighted = active || drop_accepting;
        let lifted = self.state.reorder.dragging() == Some(index);
        let torn = lifted && self.state.reorder.torn_out();
        let shift = self.state.reorder.shift(index);
        let t_key = tab_key(sc.key, tab.id);

        let lead = if self.badges && index < BADGED {
            badge(index, look)
        } else {
            tab_icon(tab.icon, highlighted, look)
        };
        let close_el = close_button(
            CloseSpec {
                id: tab.id,
                label: self.words.close,
                lock_cap: sc.shrunk.then_some(sc.cap),
                access: self.access,
                emit: &self.emit,
            },
            look,
        );

        let mut face = row()
            .key(t_key.clone())
            .h(sc.band_h)
            .max_w(sc.cap)
            .pl(look.space.lg)
            .pr(look.space.xs)
            .gap(look.space.sm)
            .items_center()
            .cursor(Cursor::Pointer)
            .hover_group()
            .hover_motion(Anim::INSTANT, Anim::INSTANT)
            .focus_ring(2.0, -2.0, look.kit.c.border_focus)
            .focusable(true)
            .focus_selected(active)
            .position_in_set(index + 1, self.data.tabs.len())
            .offset(shift[0], shift[1])
            .child(lead)
            .child(tab_label(label, highlighted, look))
            .child(close_el);

        face = apply_tab_surface(face, drop_accepting, lifted, look);
        face = apply_tab_motion(
            face,
            lifted,
            self.state.reorder.has_active_gesture() || sc.has_incoming,
            look,
        );
        face = self.attach_tab_markers(face, index, tab, active, sc);
        let ghost = torn.then(|| torn_ghost(index, tab.icon, label, shift, sc));

        face = bind_tab_handlers(
            face,
            TabBind {
                id: tab.id,
                index,
                active,
                tab_key: &t_key,
                scroll_key: sc.scroll_key,
                gap: sc.tab_gap,
                measured: sc.measured.clone(),
                tear: sc.tear,
                lock_cap: sc.shrunk.then_some(sc.cap),
                access: self.access,
                emit: &self.emit,
            },
        );
        face = wheel::sideways::<S>(face, &self.state.probe, sc.scroll_key.to_owned());

        let zone = drops::tab(
            sc.zones,
            TabSpec {
                id: tab.id,
                index,
                accepts: tab.accepts,
                active,
            },
        );
        if let Some(zone) = zone {
            face = self.dnd.drop_zone_keyed(face, t_key, zone);
        }

        // Apply role and accessible name after handlers so `on_press_tx` does not turn it into a button.
        (face.label(tab.name.clone().into_owned()).role("tab"), ghost)
    }

    fn attach_tab_markers(
        &self,
        mut face: El,
        index: usize,
        tab: &Tab<'_>,
        active: bool,
        sc: &StripCtx<'_, S>,
    ) -> El {
        if index == 0 {
            face = face.child(self.state.probe.first_marker());
        }
        if active {
            let marker = self
                .state
                .probe
                .active_marker(tab.id, name_hash(&tab.name))
                .on_layout_tx(|_: &mut S, _, tx: &mut Tx| tx.live_only());
            face = face.child(active_underline(sc.key, sc.look)).child(marker);
        }
        face
    }
}

struct TabLayout {
    labels: Vec<String>,
    measured: Rc<[f32]>,
    cap: f32,
    shrunk: bool,
}

struct CloseSpec<'a, S: 'static> {
    id: TabId,
    label: &'static str,
    lock_cap: Option<f32>,
    access: Access<S>,
    emit: &'a Emit<S>,
}

fn torn_ghost<S: 'static>(
    index: usize,
    icon_name: IconName,
    label: &str,
    shift: [f32; 2],
    sc: &StripCtx<'_, S>,
) -> El {
    let look = sc.look;
    let w = sc.measured.get(index).copied().unwrap_or(sc.cap);
    row()
        .key(format!("{}-ghost", sc.key))
        .size(w, sc.band_h)
        .pl(look.space.lg)
        .pr(look.space.xs)
        .gap(look.space.sm)
        .items_center()
        .radius(look.round(OVERLAY_RADIUS))
        .fill(slot::strip_fill(look))
        .offset(shift[0], shift[1])
        .offset_motion(Anim::INSTANT)
        .a11y_hidden()
        .child(tab_icon(icon_name, true, look))
        .child(tab_label(label, true, look))
}

struct StripCtx<'a, S: 'static> {
    key: &'a str,
    scroll_key: &'a str,
    band_h: f32,
    cap: f32,
    tab_gap: f32,
    shrunk: bool,
    measured: &'a Rc<[f32]>,
    tear: Option<TearOutSpec>,
    has_incoming: bool,
    zones: &'a Ctx<S>,
    look: &'a Look,
}

struct TabBind<'a, S: 'static> {
    id: TabId,
    index: usize,
    active: bool,
    tab_key: &'a str,
    scroll_key: &'a str,
    gap: f32,
    measured: Rc<[f32]>,
    tear: Option<TearOutSpec>,
    lock_cap: Option<f32>,
    access: Access<S>,
    emit: &'a Emit<S>,
}

fn tab_icon(icon_name: IconName, highlighted: bool, look: &Look) -> El {
    let k = &look.kit;
    let glyph = icon(look.icon(icon_name), look.icon_size.md)
        .key("icon")
        .a11y_hidden();
    let glyph = if highlighted {
        glyph.color(k.c.icon_accent)
    } else {
        glyph.color(k.c.icon_muted).group_color(k.c.icon_accent)
    };
    row()
        .key("avatar")
        .size(look.icon_size.md, look.icon_size.md)
        .center()
        .a11y_hidden()
        .child(glyph)
}

fn tab_label(label: &str, highlighted: bool, look: &Look) -> El {
    let k = &look.kit;
    let font = sizing::label_font(look);
    let text_el = text(label.to_owned())
        .key("label")
        .font(font)
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .truncate()
        .dir(rune::Direction::Auto)
        .a11y_hidden();
    if highlighted {
        text_el.color(k.c.text_accent)
    } else {
        text_el.color(k.c.text_muted).group_color(k.c.text_accent)
    }
}

fn close_button<S: 'static>(spec: CloseSpec<'_, S>, look: &Look) -> El {
    let k = &look.kit;
    let id = spec.id;
    let access = spec.access;
    let lock_cap = spec.lock_cap;
    let emit = spec.emit.clone();
    row()
        .key("close")
        .size(CLOSE_BUTTON_SIZE, CLOSE_BUTTON_SIZE)
        .center()
        .radius(look.round(CLOSE_RADIUS))
        .hover_group()
        .hover_motion(Anim::INSTANT, Anim::INSTANT)
        .hover(k.c.hover)
        .press(k.c.pressed)
        .cursor(Cursor::Pointer)
        .label(spec.label)
        .focusable(false)
        .no_focus_ring()
        .on_press_tx(move |s: &mut S, tx: &mut Tx| {
            if let Some(cap) = lock_cap {
                access(s).reorder.width_lock = Some(cap);
            }
            emit(s, TabsIntent::Close { id }, tx);
        })
        .role("button")
        .child(
            icon(look.icon(IconName::Close), look.icon_size.sm)
                .key("close-icon")
                .color(k.c.icon_muted)
                .group_color(k.c.icon_accent)
                .a11y_hidden(),
        )
}

fn apply_tab_surface(tab: El, drop_accepting: bool, lifted: bool, look: &Look) -> El {
    let radius = look.round(OVERLAY_RADIUS);
    if drop_accepting {
        tab.fill(slot::drop_fill(look)).radius(radius)
    } else if lifted {
        tab.fill(slot::strip_fill(look)).radius(radius).child(
            el().key("drag-ground")
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .radius(radius)
                .fill(look.kit.c.hover)
                .a11y_hidden(),
        )
    } else {
        tab.fill(Color::TRANSPARENT)
    }
}

fn apply_tab_motion(tab: El, lifted: bool, reordering: bool, look: &Look) -> El {
    let spring = if look.animations {
        look.kit.motion.control
    } else {
        Spring::NONE
    };
    let tab = tab.motion(if reordering { spring } else { Spring::NONE });
    if lifted {
        tab.raise().offset_motion(Anim::INSTANT)
    } else {
        tab
    }
}

fn active_underline(strip_key: &str, look: &Look) -> El {
    let line = el()
        .key("underline")
        .pin(Some(0.0), None, Some(0.0), Some(0.0))
        .h(slot::underline_height(look))
        .fill(slot::underline_color(look))
        .a11y_hidden();
    if look.animations {
        line.matched(format!("{strip_key}-underline"))
    } else {
        line
    }
}

fn incoming_marker(list_key: &str, band_h: f32, look: &Look) -> El {
    let h = (band_h - 2.0 * INSERTION_INSET).max(0.0);
    row()
        .key(format!("{list_key}-incoming"))
        .w(INSERTION_WIDTH)
        .h(band_h)
        .center()
        .a11y_hidden()
        .child(
            el().key("insertion-line")
                .size(INSERTION_WIDTH, h)
                .fill(slot::underline_color(look))
                .a11y_hidden(),
        )
}

struct DragHandler<S: 'static> {
    id: TabId,
    index: usize,
    active: bool,
    scroll_k: String,
    tab_k: String,
    gap: f32,
    measured: Rc<[f32]>,
    tear: Option<TearOutSpec>,
    access: Access<S>,
    emit: Emit<S>,
}

impl<S: 'static> DragHandler<S> {
    fn handle(&self, s: &mut S, d: &mut Drag, tx: &mut Tx) {
        match d.phase {
            Phase::Down => self.on_down(s, d, tx),
            Phase::Move => self.on_move(s, d, tx),
            Phase::Up => self.on_up(s, d, tx),
        }
    }

    fn on_down(&self, s: &mut S, d: &Drag, tx: &mut Tx) {
        (self.access)(s).reorder.press(PressSpec {
            from: self.index,
            grab: [d.x, d.y],
            widths: self.measured.to_vec(),
            gap: self.gap,
            tear: self.tear,
        });
        if !self.active {
            (self.emit)(
                s,
                TabsIntent::Select {
                    id: self.id,
                    via: Via::Strip,
                },
                tx,
            );
            tx.scroll_into_view_in(self.scroll_k.clone(), self.tab_k.clone());
        } else {
            tx.live_only();
        }
    }

    fn on_move(&self, s: &mut S, d: &Drag, tx: &mut Tx) {
        let st = &mut (self.access)(s).reorder;
        if st.reordering() {
            st.scroll.auto_scroll(&self.scroll_k, d, tx);
        }
        let was_torn = st.torn_out();
        let changed = st.follow(d.dx, d.dy);
        let torn = st.torn_out();
        if changed {
            tx.animated();
        } else {
            tx.live_only();
        }
        if self.tear.is_some() && (torn || was_torn) {
            let grab = st.grab_point();
            (self.emit)(
                s,
                TabsIntent::TearMove {
                    id: self.id,
                    out: TabTearMove {
                        id: self.id.to_string(),
                        index: self.index,
                        torn,
                        grab,
                        window_point: d.window_point(),
                        screen_point: d.screen_point(),
                    },
                },
                tx,
            );
        }
    }

    fn on_up(&self, s: &mut S, d: &Drag, tx: &mut Tx) {
        let on_tab = (0.0..=d.w).contains(&d.x) && (0.0..=d.h).contains(&d.y);
        match (self.access)(s).reorder.release(d.cancelled(), on_tab) {
            Release::Click => tx.live_only(),
            Release::None => tx.animated(),
            Release::Reorder { from, to } => {
                (self.emit)(
                    s,
                    TabsIntent::Reorder {
                        id: self.id,
                        from,
                        to,
                    },
                    tx,
                );
                tx.animated();
            }
            Release::TearOut { from, offset } => {
                (self.emit)(
                    s,
                    TabsIntent::TearOut {
                        id: self.id,
                        out: TabTearOut {
                            id: self.id.to_string(),
                            index: from,
                            offset,
                            window_point: Some(d.window_point()),
                            screen_point: d.screen_point(),
                        },
                    },
                    tx,
                );
                tx.animated();
            }
        }
    }
}

fn bind_tab_handlers<S: 'static>(tab: El, b: TabBind<'_, S>) -> El {
    let id = b.id;
    let access = b.access;
    let scroll_k = b.scroll_key.to_owned();
    let tab_k = b.tab_key.to_owned();

    let close_emit = b.emit.clone();
    let lock_cap = b.lock_cap;
    let tab = tab.on_middle_press(move |s: &mut S, tx: &mut Tx| {
        if let Some(cap) = lock_cap {
            access(s).reorder.width_lock = Some(cap);
        }
        close_emit(s, TabsIntent::Close { id }, tx);
    });

    let press_emit = b.emit.clone();
    let press_scroll = scroll_k.clone();
    let press_tab = tab_k.clone();
    let tab = tab.on_press_tx(move |s: &mut S, tx: &mut Tx| {
        if access(s).reorder.take_swallowed_click() {
            tx.live_only();
            return;
        }
        press_emit(
            s,
            TabsIntent::Select {
                id,
                via: Via::Strip,
            },
            tx,
        );
        tx.scroll_into_view_in(press_scroll.clone(), press_tab.clone());
    });

    let handler = DragHandler {
        id,
        index: b.index,
        active: b.active,
        scroll_k,
        tab_k,
        gap: b.gap,
        measured: b.measured,
        tear: b.tear,
        access,
        emit: b.emit.clone(),
    };
    tab.on_drag(move |s: &mut S, d: &mut Drag, tx: &mut Tx| handler.handle(s, d, tx))
}

fn name_hash(name: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut h);
    h.finish()
}

/// Tab `i`'s number badge: 1 to 9, then 0.
fn badge(i: usize, look: &Look) -> El {
    let digit = ((i + 1) % 10).to_string();
    let font = look.kit.ty.text_xsmall_medium;
    row()
        .key("badge")
        .size(BADGE_BOX, BADGE_BOX)
        // Strata's `.tab-key-hint` corner, scaled with the look's roundness.
        .radius(look.round(3.0))
        .center()
        .fill(slot::badge_fill(look))
        .child(text(digit).font(font).color(slot::badge_text(look)))
        .a11y_hidden()
}
