//! [`BrowserTabs`]: rune-kit's `TabStrip`, with Strata's look and what a file manager adds,
//! set over each built tab through `TabItem::decorate`.

use std::hash::{Hash, Hasher};
use std::rc::Rc;

use rune::{Color, Cx, El, Mods, Tx, el, row, text};
use rune_kit::shell::{TabDragState, TabItem, TabOrientation, TabOverflow, TabStrip, TearOutSpec};
use strata_kit::look::Look;
use strata_kit::slots::tabs as slot;

use super::data::{LABEL_CHARS, TabId, TabsData, elide_end};
use super::drops::{self, Access, ActionFn, Ctx, Emit, OpensFn};
use super::intent::{DropSite, TabsIntent, Via};
use super::new_tab::{self, TabsLabels};
use super::probe::Probe;
use super::{BrowserTabsState, scroll_key, wheel};
use crate::contract::dnd::{Dnd, DropAction, DropHover, DropOffer};

/// rune-kit's tab pill radius, its private `RADIUS` at the pinned rev
/// (`rune-kit/src/shell/tab_strip/tab.rs:11`), so a drop tint on the tab's slot lines up with
/// the pill drawn over it. Check it at each Rune bump.
const PILL_RADIUS: f32 = 6.0;

/// The box rune-kit draws a tab's avatar in (`row().key("avatar").size(16.0, 16.0)`,
/// `rune-kit/src/shell/tab_strip/tab.rs:205` at the pinned rev): a badge fills it.
const AVATAR_BOX: f32 = 16.0;

/// How many tabs carry a number badge: Ctrl+Shift+1 to 9, then 0 for the tenth.
const BADGED: usize = 10;

/// The strip: `BrowserTabs::new(&data, &s.tabs, |s| &mut s.tabs, |s| &mut s.tabs.drag)
/// .on_intent(..).build(look, cx)`. See the module docs for what the host does with it.
pub struct BrowserTabs<'a, S: 'static> {
    data: &'a TabsData<'a>,
    state: &'a BrowserTabsState,
    access: Access<S>,
    drag: fn(&mut S) -> &mut TabDragState,
    dnd: Dnd<S>,
    action: Option<ActionFn<S>>,
    opens: Option<OpensFn<S>>,
    content: &'static [&'static str],
    spring: Option<f64>,
    tear: bool,
    badges: bool,
    new_tab: bool,
    labels: TabsLabels,
    emit: Emit<S>,
}

impl<'a, S: 'static> BrowserTabs<'a, S> {
    /// The strip over `data`, with its `state` and the accessors that find them in the app's
    /// state: the state's, and its `drag` field's (rune-kit's `TabStrip::drag_state` takes a
    /// plain `fn`, so it can't be derived from the first).
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
            drag,
            dnd: Dnd::rune(),
            action: None,
            opens: None,
            content: &[],
            spring: Some(super::HOVER_DELAY),
            tear: false,
            badges: false,
            new_tab: true,
            labels: TabsLabels::default(),
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

    /// Which drags open tabs rather than drop into one: Strata's own folder-only drags from its
    /// views and sidebar rows, never folders from another app. Such a drag neither tints nor
    /// selects a tab; it places a gap where it would open, and its drop is
    /// [`TabsIntent::OpenTabs`]. The host decides from the offer (its serial, its own drag's
    /// payload). Without it, no drag opens tabs.
    pub fn opens_tabs(mut self, f: impl Fn(&S, &DropOffer) -> bool + 'static) -> Self {
        self.opens = Some(Rc::new(f));
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

    fn assemble(self, look: &Look, cx: &mut Cx<S>, layer: Option<&mut Vec<El>>) -> El {
        let key = self.state.key().to_owned();
        let data = self.data;
        let zones = Ctx {
            access: self.access,
            emit: self.emit.clone(),
            action: self.action.clone(),
            opens: self.opens.clone(),
            content: self.content,
            spring: self.spring,
            len: data.tabs.len(),
        };
        let drop = self.state.drop_hover();
        let scroll = scroll_key(&key);
        let underline = Underline {
            color: slot::underline_color(look),
            height: slot::underline_height(look),
            inset: look.space.sm,
            matched: look.animations.then(|| format!("{key}-underline")),
        };
        let items: Vec<TabItem<'_, S>> = data
            .tabs
            .iter()
            .enumerate()
            .map(|(i, tab)| {
                let active = data.active == Some(tab.id);
                let mut item = TabItem::new(
                    rune_id(&key, tab.id),
                    elide_end(&tab.name, LABEL_CHARS).into_owned(),
                );
                item = match (self.badges && i < BADGED).then(|| badge(i, look)) {
                    Some(b) => item.avatar(b),
                    None => item.icon(look.icon(tab.icon)),
                };
                let dress = Dress {
                    name: tab.name.clone().into_owned(),
                    tint: (drop == Some((DropSite::Tab(tab.id), DropHover::Accepting)))
                        .then(|| slot::drop_fill(look)),
                    underline: active.then(|| underline.clone()),
                    first: i == 0,
                    active: active.then_some(tab.id),
                    probe: self.state.probe.clone(),
                    scroll: scroll.clone(),
                    zone: drops::tab(&zones, tab.id, i, tab.accepts, active),
                    key: super::tab_key(&key, tab.id),
                    dnd: self.dnd.clone(),
                };
                item.decorate(move |e, _| dress.apply(e))
            })
            .collect();
        let active = data.active.map(|id| rune_id(&key, id));
        let min = slot::min_width(look);
        let strip = TabStrip::new(key.clone(), TabOrientation::Horizontal)
            .tabs(items)
            .active(active.as_deref())
            .viewport_width(cx.layout_size(&key).map(|z| z[0]))
            .overflow(TabOverflow::ShrinkThenScroll {
                min_width: min,
                max_width: slot::max_width(look),
                pinned_width: min,
                fade_width: slot::fade_width(look),
            })
            .gap(slot::tab_gap(look))
            .row_height(slot::pill_height(look))
            .fill(slot::strip_fill(look))
            .close_icon(look.icon(strata_kit::icons::IconName::Close))
            .label(self.labels.list.clone().into_owned())
            .drag_state(&self.state.drag, self.drag)
            .tear_out(self.tear.then(TearOutSpec::default));
        let (strip, trailing) = if self.new_tab {
            let width = cx
                .layout_size(&super::new_tab_key(&key))
                .map_or(look.frame.control, |z| z[0]);
            let button = new_tab::button(&zones, self.state, &self.dnd, &self.labels, look);
            let strip = strip.trailing_action(button).trailing_width(width);
            (strip, width + slot::tab_gap(look))
        } else {
            (strip, 0.0)
        };
        self.state
            .probe
            .built(slot::fade_width(look), trailing, data.active);
        let strip = handlers(strip, &key, &self.emit, self.tear);
        let root = match layer {
            Some(layer) => strip.build_layered(&look.kit, layer),
            None => strip.build(&look.kit),
        };
        let root = root.children(self.state.probe.root_markers());
        let root =
            self.dnd
                .drop_zone_keyed(root, key.clone(), drops::opener(&zones, DropSite::Strip));
        row()
            .key(format!("{key}-band"))
            .w_full()
            .h(slot::tab_height(look))
            .px(look.space.sm)
            .gap(look.space.sm)
            .center_y()
            .fill(slot::strip_fill(look))
            .child(root)
    }
}

/// TabStrip's id for tab `id`, which it keys `t-{id}`: the strip's key first, so two strips'
/// tabs never share a key (U-C7-5).
fn rune_id(key: &str, id: TabId) -> String {
    format!("{key}-{id}")
}

fn parse(prefix: &str, rid: &str) -> Option<TabId> {
    rid.strip_prefix(prefix)?.parse().ok().map(TabId)
}

/// TabStrip's handlers, each reporting its intent with the host's id.
fn handlers<'a, S: 'static>(
    strip: TabStrip<'a, S>,
    key: &str,
    emit: &Emit<S>,
    tear: bool,
) -> TabStrip<'a, S> {
    let prefix: Rc<str> = format!("{key}-").into();
    let on = |f: fn(TabId) -> TabsIntent| {
        let (prefix, emit) = (prefix.clone(), emit.clone());
        move |s: &mut S, rid: &str, tx: &mut Tx| {
            if let Some(id) = parse(&prefix, rid) {
                emit(s, f(id), tx);
            }
        }
    };
    let strip = strip
        .on_select(on(|id| TabsIntent::Select {
            id,
            via: Via::Strip,
        }))
        .on_close(on(|id| TabsIntent::Close { id }));
    let (p, e) = (prefix.clone(), emit.clone());
    let strip = strip.on_reorder(move |s: &mut S, r, tx: &mut Tx| {
        if let Some(id) = parse(&p, &r.id) {
            e(
                s,
                TabsIntent::Reorder {
                    id,
                    from: r.from,
                    to: r.to,
                },
                tx,
            );
        }
    });
    if !tear {
        return strip;
    }
    let (p, e) = (prefix.clone(), emit.clone());
    let strip = strip.on_tear_out(move |s: &mut S, out, tx: &mut Tx| {
        if let Some(id) = parse(&p, &out.id) {
            e(s, TabsIntent::TearOut { id, out }, tx);
        }
    });
    let (p, e) = (prefix, emit.clone());
    strip.on_tear_move(move |s: &mut S, out, tx: &mut Tx| {
        if let Some(id) = parse(&p, &out.id) {
            e(s, TabsIntent::TearMove { id, out }, tx);
        }
    })
}

/// The active tab's accent underline.
#[derive(Clone)]
struct Underline {
    color: Color,
    height: f32,
    inset: f32,
    /// Shared between tabs so it slides to the next active one; `None` with animations off,
    /// so it jumps.
    matched: Option<String>,
}

impl Underline {
    fn el(&self) -> El {
        let line = el()
            .key("underline")
            .pin(Some(self.inset), None, Some(self.inset), Some(0.0))
            .h(self.height)
            .radius(self.height / 2.0)
            .fill(self.color)
            .a11y_hidden();
        match &self.matched {
            Some(id) => line.matched(id.clone()),
            None => line,
        }
    }
}

/// What C7 sets over one built tab.
struct Dress<S: 'static> {
    /// The whole name: the accessible name, where the label may be shortened.
    name: String,
    tint: Option<Color>,
    underline: Option<Underline>,
    /// Tab 0 carries a marker for the strip's offset, and the active tab one for its reveal.
    first: bool,
    active: Option<TabId>,
    probe: Probe,
    scroll: String,
    zone: Option<crate::contract::dnd::DropZone<S>>,
    /// The tab's key ([`super::tab_key`]), which rune-kit gave the element.
    key: String,
    dnd: Dnd<S>,
}

impl<S: 'static> Dress<S> {
    fn apply(self, tab: El) -> El {
        let mut tab = tab.label(self.name.clone());
        if let Some(tint) = self.tint {
            tab = tab.fill(tint).radius(PILL_RADIUS);
        }
        if let Some(line) = &self.underline {
            tab = tab.child(line.el());
        }
        if self.first {
            tab = tab.child(self.probe.first_marker());
        }
        if let Some(id) = self.active {
            // A host that changed the active tab, or renamed it, from a handler did it after
            // that frame's poll, and a window at rest may draw no other. The marker is keyed by
            // the tab's name too, so either mounts a fresh one, whose first layout is reported
            // as queued input, which brings the frame whose tick reveals the tab and retitles
            // the window (reviews 2 and 3); the report itself changes nothing the view reads.
            let version = {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                self.name.hash(&mut h);
                h.finish()
            };
            let marker = self
                .probe
                .active_marker(id, version)
                .on_layout_tx(|_: &mut S, _, tx: &mut rune::Tx| tx.live_only());
            tab = tab.child(marker);
        }
        tab = wheel::sideways::<S>(tab, &self.probe, self.scroll);
        match self.zone {
            Some(zone) => self.dnd.drop_zone_keyed(tab, self.key, zone),
            None => tab,
        }
    }
}

/// Tab `i`'s number badge: 1 to 9, then 0.
fn badge(i: usize, look: &Look) -> El {
    let digit = ((i + 1) % 10).to_string();
    let font = look.kit.ty.text_xsmall_medium;
    row()
        .key("badge")
        .size(AVATAR_BOX, AVATAR_BOX)
        // Strata's `.tab-key-hint` corner, scaled with the look's roundness.
        .radius(look.round(3.0))
        .center()
        .fill(slot::badge_fill(look))
        .child(text(digit).font(font).color(slot::badge_text(look)))
        .a11y_hidden()
}
