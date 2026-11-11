//! The slider's element: a track with its fill and knob, a tick under each snap point, and a
//! toggle under each labelled one.

use std::rc::Rc;

use rune::{Cursor, Drag, El, Key, KeyEvent, Phase, Tx, WheelEvent, col, el, text};
use strata_kit::look::Look;

use super::state::{SliderChange, SliderState, takes_wheel};

type Get<S> = fn(&S) -> &SliderState;
type GetMut<S> = fn(&mut S) -> &mut SliderState;
type Report<S> = Rc<dyn Fn(&mut S, f64, SliderChange, &mut Tx)>;

/// A snap point: a value a drag within a few points lands on, with a tick under it, and when
/// labelled a toggle under the tick that sets it (Strata's Compact and Airy).
#[derive(Clone, Debug, PartialEq)]
pub struct Snap {
    pub value: f64,
    pub label: Option<String>,
}

impl Snap {
    /// A tick with no label.
    pub fn at(value: f64) -> Snap {
        Snap { value, label: None }
    }

    /// A tick with a toggle labelled `label` under it.
    pub fn labelled(value: f64, label: impl Into<String>) -> Snap {
        Snap {
            value,
            label: Some(label.into()),
        }
    }
}

/// Sizes, all from Look tokens, so they follow the text size as Strata's em-based CSS did.
#[derive(Clone, Copy)]
struct Metrics {
    /// The slider's height, its hit area: `frame.control` (24 at 13 px, scaled with the text).
    height: f32,
    /// The knob's diameter: `icon_size.md` (16 at 13 px, scaled with the text).
    knob: f32,
    /// The track's thickness: a quarter of the knob (4).
    track: f32,
    /// A tick's length: the room under the knob, `(height - knob) / 2` (4).
    tick: f32,
    /// How close a drag comes to a snap point to land on it: `space.lg` (12), GTK's
    /// `MARK_SNAP_LENGTH`.
    snap_within: f32,
    /// The least room between two snap labels: `space.md` (8), Strata's `GAP`.
    label_gap: f32,
    /// The knob's scale while a drag holds it: as tall as the hit area allows, at most a
    /// quarter larger (16 to 20).
    engaged_scale: f32,
    /// The accent ring's width while a drag holds the knob, before the scale: an eighth of
    /// the knob (2), Strata's 2 px accent border.
    engaged_ring: f32,
}

impl Metrics {
    fn of(look: &Look) -> Metrics {
        let height = look.frame.control;
        let knob = look.icon_size.md.min(height);
        Metrics {
            height,
            knob,
            track: (knob / 4.0).round().max(2.0),
            tick: ((height - knob) / 2.0).max(1.0),
            snap_within: look.space.lg,
            label_gap: look.space.md,
            engaged_scale: (height / knob).clamp(1.0, 1.25),
            engaged_ring: (knob / 8.0).max(1.0),
        }
    }
}

/// A horizontal slider over a [`SliderState`] the host holds.
///
/// It reports through [`Slider::on_change`]: [`SliderChange::Live`] while a drag moves it,
/// [`SliderChange::Commit`] when it settles, and [`SliderChange::Cancel`] when a drag is
/// cancelled and the value is back where it started. The value is already in the state when
/// the handler runs.
pub struct Slider<S> {
    key: String,
    get: Get<S>,
    get_mut: GetMut<S>,
    snaps: Vec<Snap>,
    free_drag: bool,
    wheel: Option<f64>,
    label: Option<String>,
    value_text: Option<String>,
    disabled: bool,
    report: Option<Report<S>>,
}

impl<S: 'static> Slider<S> {
    /// The slider keyed `key` over the state `get` and `get_mut` reach. Its parts are keyed
    /// `<key>-track`, `<key>-fill`, `<key>-knob`, `<key>-tick-<i>` and `<key>-snap-<i>`,
    /// `i` a snap point's place in [`Slider::snaps`].
    pub fn new(key: impl Into<String>, get: Get<S>, get_mut: GetMut<S>) -> Self {
        Slider {
            key: key.into(),
            get,
            get_mut,
            snaps: Vec::new(),
            free_drag: false,
            wheel: None,
            label: None,
            value_text: None,
            disabled: false,
            report: None,
        }
    }

    /// The snap points, lowest first.
    pub fn snaps(mut self, snaps: Vec<Snap>) -> Self {
        self.snaps = snaps;
        self
    }

    /// A drag sets any value, not only the grid's: C13's volume. Keys still step the grid.
    pub fn free_drag(mut self, free: bool) -> Self {
        self.free_drag = free;
        self
    }

    /// The wheel over the slider moves it `amount` a notch. Without it the wheel passes on to
    /// what scrolls behind the slider (Strata's density sliders in their menu); with it, a
    /// sideways wheel and one with Ctrl, Alt, Cmd or Super held still pass on.
    pub fn wheel(mut self, amount: Option<f64>) -> Self {
        self.wheel = amount;
        self
    }

    /// The accessible name.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The value as assistive technology reads it ("Compact · 4 px"); without it, the number.
    pub fn value_text(mut self, text: impl Into<String>) -> Self {
        self.value_text = Some(text.into());
        self
    }

    /// A slider that can't be used now: dimmed, out of the focus order, and deaf to input.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Hears every change, with the new value.
    pub fn on_change(mut self, f: impl Fn(&mut S, f64, SliderChange, &mut Tx) + 'static) -> Self {
        self.report = Some(Rc::new(f));
        self
    }

    pub fn build(self, s: &S, look: &Look) -> El {
        let st = (self.get)(s);
        let m = Metrics::of(look);
        let range = st.range();
        let value = st.value();
        let k = &look.kit;
        let c = &k.c;
        let key = self.key.clone();
        let w = st.width();
        let x = st.x_of(value, m.knob);
        let mid = m.height / 2.0;

        let mut slider = el()
            .key(key.clone())
            .role("slider")
            .w_full()
            .h(m.height)
            .min_w(m.knob)
            .range_value(value, range.min(), range.max())
            .range_step(range.step());
        if let Some(l) = &self.label {
            slider = slider.label(l.clone());
        }
        if let Some(t) = &self.value_text {
            slider = slider.a11y_value(t.clone());
        }
        let get_mut = self.get_mut;
        slider = slider.on_layout_tx(move |s: &mut S, r: [f32; 4], tx: &mut Tx| {
            if !get_mut(s).set_width(r[2]) {
                tx.live_only();
            }
        });

        // The track, the fill up to the knob's centre, the ticks and the knob.
        let track_y = mid - m.track / 2.0;
        slider = slider
            .child(
                el().key(format!("{key}-track"))
                    .pin(Some(0.0), Some(track_y), None, None)
                    .size(w, m.track)
                    .radius(m.track / 2.0)
                    .fill(c.border_base)
                    .a11y_hidden(),
            )
            .child(
                el().key(format!("{key}-fill"))
                    .pin(Some(0.0), Some(track_y), None, None)
                    .size(x, m.track)
                    .radius(m.track / 2.0)
                    .fill(c.bg_accent)
                    .a11y_hidden(),
            );
        for (i, snap) in self.snaps.iter().enumerate() {
            let tx = st.x_of(range.clamp(snap.value), m.knob);
            slider = slider.child(
                el().key(format!("{key}-tick-{i}"))
                    .pin(Some(tx - 0.5), Some(m.height - m.tick), None, None)
                    .size(1.0, m.tick)
                    .fill(c.border_strong)
                    .a11y_hidden(),
            );
        }
        let knob = el()
            .key(format!("{key}-knob"))
            .pin(Some(x - m.knob / 2.0), Some(mid - m.knob / 2.0), None, None)
            .size(m.knob, m.knob)
            .radius(m.knob / 2.0)
            .fill(c.knob)
            .gradient(k.on(c.depth_top, c.knob), k.on(c.depth_bottom, c.knob))
            .focus_ring(2.0, 1.0, c.border_focus)
            .a11y_hidden();
        // While a drag holds it the knob grows and takes an accent ring, so the press reads
        // as engaged (Strata's `:active` slider); the scale draws over the room around the
        // knob and leaves the layout alone.
        let knob = if st.dragging() && !self.disabled {
            knob.scale(m.engaged_scale)
                .border(m.engaged_ring, c.bg_accent)
        } else {
            knob.border(0.5, k.on(c.depth_top, c.knob))
        };
        slider = slider.child(k.elevate(knob, &k.elevation.knob));

        let widths = self.label_widths(look);
        let labels = self.labels(st, look, m, &widths);
        // As wide as the labels side by side with their gaps, so a menu sized to its content
        // keeps them from overlapping (Strata's `SnapLabels::measure`).
        let reserve =
            widths.iter().sum::<f32>() + m.label_gap * widths.len().saturating_sub(1) as f32;
        // The knob, fill and labels follow the value and the track's width with no motion:
        // the knob stays under the pointer, and the first layout doesn't slide it from where
        // a zero width put it (a tap then would land on the track).
        let parts = |slider: El| {
            col()
                .w_full()
                .min_w(reserve.max(m.knob))
                .instant_changes()
                .children(std::iter::once(slider).chain(labels))
        };
        if self.disabled {
            return parts(slider.disabled(true)).layer_opacity(0.5);
        }
        parts(self.wire(slider.cursor(Cursor::Pointer), m))
    }

    /// The pointer, keys, wheel and assistive actions.
    fn wire(&self, mut slider: El, m: Metrics) -> El {
        let get_mut = self.get_mut;
        let report = self.report.clone();
        let tell = move |s: &mut S, change: Option<SliderChange>, tx: &mut Tx| {
            if let (Some(change), Some(f)) = (change, &report) {
                let v = get_mut(s).value();
                f(s, v, change, tx);
            }
        };
        let tell = Rc::new(tell);

        let snaps: Rc<[f64]> = self.snaps.iter().map(|s| s.value).collect();
        let free = self.free_drag;
        let t = tell.clone();
        slider = slider.on_drag(move |s: &mut S, d: &mut Drag, tx: &mut Tx| {
            let st = get_mut(s);
            let change = match d.phase {
                Phase::Down => st.press(d.x, m.knob, &snaps, m.snap_within, free),
                Phase::Move => st.drag(d.x, m.knob, &snaps, m.snap_within, free),
                Phase::Up => st.release(d.cancelled()),
            };
            t(s, change, tx);
        });

        // Page Up and Down go here, ahead of Rune's `on_step`, whose page is a fixed ten
        // steps (Rune draft U-C13-2): Strata's density pages 4 px a step of 1.
        let t = tell.clone();
        slider = slider.on_key(move |s: &mut S, e: &KeyEvent, tx: &mut Tx| {
            let mods = e.mods;
            if mods.shift || mods.ctrl || mods.alt || mods.cmd || mods.logo {
                return false;
            }
            let st = get_mut(s);
            let page = st.range().page_steps();
            let moved = match e.key {
                Key::PageUp => st.step(page),
                Key::PageDown => st.step(-page),
                _ => return false,
            };
            t(s, moved.then_some(SliderChange::Commit), tx);
            true
        });
        let t = tell.clone();
        slider = slider.on_step(move |s: &mut S, steps: i32, tx: &mut Tx| {
            let moved = get_mut(s).step(steps);
            t(s, moved.then_some(SliderChange::Commit), tx);
        });
        let t = tell.clone();
        slider = slider.on_step_limit(move |s: &mut S, max: bool, tx: &mut Tx| {
            let moved = get_mut(s).jump_to_end(max);
            t(s, moved.then_some(SliderChange::Commit), tx);
        });
        if let Some(amount) = self.wheel {
            let t = tell;
            slider = slider.on_wheel(move |s: &mut S, w: WheelEvent, tx: &mut Tx| {
                if !takes_wheel(&w) {
                    return false;
                }
                let moved = get_mut(s).wheel(w.dy, w.precise, amount);
                t(s, moved.then_some(SliderChange::Commit), tx);
                true
            });
        }
        slider
    }

    /// The labelled snaps' toggles' widths: the label in `kit.ty.small` and `space.xs` each
    /// side.
    fn label_widths(&self, look: &Look) -> Vec<f32> {
        let font = look.kit.ty.small;
        self.snaps
            .iter()
            .filter_map(|s| s.label.as_deref())
            .map(|l| rune::text::advance(l, font).ceil() + 2.0 * look.space.xs)
            .collect()
    }

    /// The labelled snaps' toggles, `widths` wide, each centred under its tick, kept inside
    /// the track and at least `label_gap` apart (Strata's `SnapLabels::size_allocate`).
    fn labels(&self, st: &SliderState, look: &Look, m: Metrics, widths: &[f32]) -> Option<El> {
        let labelled: Vec<(usize, &Snap, &str)> = self
            .snaps
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.label.as_deref().map(|l| (i, s, l)))
            .collect();
        if labelled.is_empty() {
            return None;
        }
        let k = &look.kit;
        let font = k.ty.small;
        let line = strata_kit::typography::line_of(font);
        let height = line + 2.0 * look.space.xxs;
        let width = st.width();
        let range = st.range();
        // Narrower than the track only before the first layout.
        let widths: Vec<f32> = widths.iter().map(|w| w.min(width.max(0.0))).collect();
        let gaps = m.label_gap * labelled.len().saturating_sub(1) as f32;
        let mut remaining = widths.iter().sum::<f32>() + gaps;
        let mut left = 0.0f32;
        let mut row = el().w_full().h(height);
        for ((i, snap, label), lw) in labelled.iter().zip(widths) {
            let centre = st.x_of(range.clamp(snap.value), m.knob);
            let x = (centre - lw / 2.0).clamp(left, (width - remaining).max(left));
            left = x.ceil() + lw + m.label_gap;
            remaining -= lw + m.label_gap;
            let current = range.clamp(snap.value) == st.value();
            let mut b = el()
                .key(format!("{}-snap-{i}", self.key))
                .role("toggle")
                .label((*label).to_owned())
                .checked(current)
                .pin(Some(x), Some(0.0), None, None)
                .size(lw, height)
                .radius(k.radius.sm)
                .center()
                .child(
                    text((*label).to_owned())
                        .font(font)
                        .color(if current {
                            k.c.text_accent
                        } else {
                            k.c.text_muted
                        })
                        .a11y_hidden(),
                );
            if self.disabled {
                b = b.disabled(true);
            } else {
                let (get_mut, report, value) = (self.get_mut, self.report.clone(), snap.value);
                b = b
                    .focusable(true)
                    .cursor(Cursor::Pointer)
                    .hover(k.c.hover)
                    .press(k.c.pressed)
                    .on_press_tx(move |s: &mut S, tx: &mut Tx| {
                        if get_mut(s).set(value)
                            && let Some(f) = &report
                        {
                            let v = get_mut(s).value();
                            f(s, v, SliderChange::Commit, tx);
                        }
                    });
            }
            row = row.child(b);
        }
        Some(row)
    }
}
