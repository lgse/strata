// SPDX-License-Identifier: MIT

use std::rc::Rc;

use rune::{Drag, Mods, Phase, PointerButton, Tx};
use strata_components::{
    collection::{
        element::{CollectionState, Ctx, Effect, MarqueeController, Settings, viewport_path},
        marquee::MarqueeSettings,
        pointer::PointerSettings,
    },
    contract::{CollectionGeometry, EntrySource, Point},
};

use crate::{
    App, columns, filter,
    icons::{self, Icons},
    list::{self, List},
    shell, single,
};

#[derive(Clone, Copy, Debug)]
struct OriginPress {
    start: Point,
    mods: Mods,
    dragging: bool,
}

#[derive(Default)]
pub(super) struct OriginState {
    press: Option<OriginPress>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TargetKind {
    Columns,
    Icons,
    List,
    FilterIcons,
    FilterList,
}

#[derive(Clone, Copy)]
struct StepSpec {
    target: TargetKind,
    start: Point,
    now: Point,
}

#[derive(Clone, Copy)]
struct StepPoints {
    anchor: Point,
    current: Point,
}

impl TargetKind {
    fn resolve(app: &App) -> Option<Self> {
        let focus = shell::shown_view(app)?.focus;
        match focus {
            columns::VIEW => app.browser.location_at(0).map(|_| Self::Columns),
            icons::VIEW => Some(if app.filter.pane.shows() {
                Self::FilterIcons
            } else {
                Self::Icons
            }),
            list::VIEW => Some(if app.filter.pane.shows() {
                Self::FilterList
            } else {
                Self::List
            }),
            _ => None,
        }
    }

    fn focus_key(self) -> &'static str {
        match self {
            Self::Columns => columns::VIEW,
            Self::Icons | Self::FilterIcons => icons::VIEW,
            Self::List | Self::FilterList => list::VIEW,
        }
    }

    fn geometry(self, app: &App) -> Option<Box<dyn CollectionGeometry>> {
        match self {
            Self::Columns => Some(columns::view(app).column_collection(app, 0)?.geometry(app)),
            Self::Icons => Some(icons::view::grid().collection(app).geometry(app)),
            Self::FilterIcons => Some(icons::view::results().collection(app).geometry(app)),
            Self::List => Some(list::view::ticked(app).geometry(app)),
            Self::FilterList => Some(list::view::results(app).geometry(app)),
        }
    }

    fn parts_mut(self, app: &mut App) -> Option<(&dyn EntrySource, &mut CollectionState, String)> {
        match self {
            Self::Columns => app.columns.leading_marquee_parts(),
            Self::Icons => {
                let (src, st) = app.icons.single.pane.parts_mut();
                Some((src, st, viewport_path(icons::VIEW)))
            }
            Self::List => {
                let (src, st) = app.list.single.pane.parts_mut();
                Some((src, st, viewport_path(list::VIEW)))
            }
            Self::FilterIcons | Self::FilterList => {
                let (src, st) = app.filter.pane.parts_mut();
                Some((src, st, viewport_path(self.focus_key())))
            }
        }
    }

    fn interrupt_visit(self, app: &mut App) {
        match self {
            Self::Icons | Self::FilterIcons => single::interrupted::<Icons>(app),
            Self::List | Self::FilterList => single::interrupted::<List>(app),
            Self::Columns => {}
        }
    }

    fn sync_selection(self, app: &mut App, tx: &mut Tx) {
        match self {
            Self::Columns => {
                if app.columns.shows_hits(0) {
                    filter::picked(app);
                }
                shell::settle(app, tx);
            }
            Self::Icons | Self::List => single::moved(app, tx),
            Self::FilterIcons | Self::FilterList => filter::picked(app),
        }
    }
}

fn marquee_settings(animations: bool) -> Rc<Settings> {
    let mut settings = Settings::default();
    let mut marquee = MarqueeSettings::default();
    marquee.animations = animations;
    settings.insert(marquee);
    Rc::new(settings)
}

fn apply_effects(viewport: &str, effects: &[Effect], tx: &mut Tx) {
    for effect in effects {
        match *effect {
            Effect::ScrollTo { y, glide: true } => tx.scroll_to(viewport.to_owned(), 0.0, y),
            Effect::ScrollTo { y, glide: false } => {
                tx.scroll_to_instant(viewport.to_owned(), 0.0, y);
            }
            Effect::Glide { y, spring } => tx.scroll_to_with(viewport.to_owned(), 0.0, y, spring),
            Effect::WakeAfter(d) => tx.wake_after(d),
            _ => {}
        }
    }
}

fn with_marquee(
    app: &mut App,
    spec: StepSpec,
    tx: &mut Tx,
    op: impl FnOnce(&mut dyn MarqueeController, StepPoints, &mut Ctx<'_>),
) -> Option<bool> {
    let animations = app.look.animations;
    let geometry = spec.target.geometry(app)?;
    let (src, st, viewport) = spec.target.parts_mut(app)?;
    if st.viewport().w <= 0.0 || st.viewport().h <= 0.0 {
        return None;
    }
    let points = StepPoints {
        anchor: st.content_point(spec.start),
        current: st.content_point(spec.now),
    };
    let visible = st.visible_content();
    let origin = Point::new(st.viewport().x, st.viewport().y);
    let settings = marquee_settings(animations);
    let marquee = st.controllers.marquee.as_deref_mut()?;
    let mut cx = Ctx::new(src, &*geometry, visible, tx.clock(), &mut st.selection)
        .with_animations(animations)
        .window_origin(origin)
        .with_settings(settings);
    op(marquee, points, &mut cx);
    let changed = cx.changed().any();
    apply_effects(&viewport, cx.effects(), tx);
    Some(changed)
}

fn clear_at_press(app: &mut App, target: TargetKind, mods: Mods, tx: &mut Tx) -> bool {
    target.interrupt_visit(app);
    if mods.ctrl || mods.shift {
        return false;
    }
    if target == TargetKind::Columns {
        app.columns.clear_selection();
        if app.browser.location_at(1).is_some() {
            app.browser.close_column(1);
        }
        shell::settle(app, tx);
        return true;
    }
    let Some((src, st, _)) = target.parts_mut(app) else {
        return false;
    };
    let changed = st.selection.clear();
    st.note_selection(src);
    if changed.any() {
        target.sync_selection(app, tx);
    }
    changed.any()
}

fn past_drag_threshold(start: Point, now: Point) -> bool {
    let threshold = PointerSettings::default().drag_threshold;
    (now.x - start.x).abs() > threshold || (now.y - start.y).abs() > threshold
}

fn begin_origin_band(app: &mut App, spec: StepSpec, mods: Mods, tx: &mut Tx) {
    if let Some(held) = &mut app.pointer.origin.press {
        held.dragging = true;
    }
    tx.focus(spec.target.focus_key(), false);
    if spec.target == TargetKind::Columns {
        let look = app.look.clone();
        app.columns.activate_leading(&look, tx);
    }
    let changed = with_marquee(app, spec, tx, |m, pts, cx| {
        m.begin(pts.anchor, mods, cx);
        m.moved(pts.current, cx);
    })
    .unwrap_or(false);
    if changed {
        spec.target.sync_selection(app, tx);
    }
}

fn move_origin(app: &mut App, press: OriginPress, window: Point, tx: &mut Tx) {
    let Some(target) = TargetKind::resolve(app) else {
        tx.live_only();
        return;
    };
    let spec = StepSpec {
        target,
        start: press.start,
        now: window,
    };
    if !press.dragging {
        if !past_drag_threshold(press.start, window) {
            tx.live_only();
            return;
        }
        begin_origin_band(app, spec, press.mods, tx);
        return;
    }
    let changed = with_marquee(app, spec, tx, |m, pts, cx| {
        if m.active() {
            m.moved(pts.current, cx);
        }
    });
    match changed {
        Some(true) => target.sync_selection(app, tx),
        _ => tx.live_only(),
    }
}

fn end_origin(app: &mut App, (press, window): (OriginPress, Point), cancelled: bool, tx: &mut Tx) {
    if !press.dragging {
        tx.live_only();
        return;
    }
    let Some(target) = TargetKind::resolve(app) else {
        return;
    };
    let spec = StepSpec {
        target,
        start: press.start,
        now: window,
    };
    let changed = with_marquee(app, spec, tx, |m, _, cx| {
        if m.active() {
            if cancelled {
                m.cancel(cx);
            } else {
                m.end(cx);
            }
        }
    })
    .unwrap_or(false);
    if let Some((src, st, _)) = target.parts_mut(app) {
        st.note_selection(src);
        st.hold_rest();
    }
    if changed || cancelled {
        target.sync_selection(app, tx);
    }
}

pub(crate) fn leading_origin_drag(app: &mut App, drag: &mut Drag, tx: &mut Tx) {
    if tx.button() != PointerButton::Primary {
        tx.live_only();
        return;
    }
    let [wx, wy] = drag.window_point();
    let window = Point::new(wx, wy);
    match drag.phase {
        Phase::Down => {
            cancel(app, tx);
            let Some(target) = TargetKind::resolve(app) else {
                tx.live_only();
                return;
            };
            let mods = tx.mods();
            app.pointer.origin.press = Some(OriginPress {
                start: window,
                mods,
                dragging: false,
            });
            if !clear_at_press(app, target, mods, tx) {
                tx.live_only();
            }
        }
        Phase::Move => {
            let Some(press) = app.pointer.origin.press.filter(|_| !drag.cancelled()) else {
                tx.live_only();
                return;
            };
            move_origin(app, press, window, tx);
        }
        Phase::Up => {
            let Some(press) = app.pointer.origin.press.take() else {
                tx.live_only();
                return;
            };
            end_origin(app, (press, window), drag.cancelled(), tx);
        }
    }
}

/// Cancels any in-flight leading-chrome marquee (on a view or tab switch).
pub(super) fn cancel(app: &mut App, tx: &mut Tx) {
    let Some(press) = app.pointer.origin.press.take() else {
        return;
    };
    end_origin(app, (press, press.start), true, tx);
}
