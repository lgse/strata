//! The picker's parts: the plane, a strip, the hex row and a swatch row.

use std::f32::consts::FRAC_PI_2;

use rune::{
    Align, Color, Cursor, Drag, El, Key, KeyEvent, Phase, PointerPress, Shape, Tx, col, el, row,
    text, text_input,
};
use strata_kit::look::Look;

use super::super::state::{ColorChange, ColorPickerState, Part, percent};
use super::super::swatch::SwatchRow;
use super::super::value::{Hsv, Srgba};
use super::{ColorPicker, Metrics, escape_key};
use crate::contract::Point;

/// The hue strip's stops: red, yellow, green, cyan, blue, magenta and red again, which sRGB
/// interpolation between them draws as HSV's hue exactly.
fn hue_stops() -> [(f32, Color); 7] {
    let at = |i: u8| {
        let c = Hsv {
            h: f32::from(i) * 60.0,
            s: 1.0,
            v: 1.0,
            a: 1.0,
        };
        (f32::from(i) / 6.0, c.to_srgba().to_color())
    };
    [at(0), at(1), at(2), at(3), at(4), at(5), at(6)]
}

/// A checkerboard `w` by `h` of `cell` squares in `ink`, over the element's own fill: what
/// shows through a translucent colour.
fn checker(w: f32, h: f32, cell: f32, ink: Color) -> Shape {
    let mut shape = Shape::new();
    if w <= 0.0 || h <= 0.0 || cell <= 0.0 {
        return shape;
    }
    let (cols, rows) = ((w / cell).ceil() as u32, (h / cell).ceil() as u32);
    for j in 0..rows {
        for i in (j % 2..cols).step_by(2) {
            let (x0, y0) = (i as f32 * cell, j as f32 * cell);
            let (x1, y1) = ((x0 + cell).min(w), (y0 + cell).min(h));
            shape = shape.fill(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]], ink);
        }
    }
    shape
}

/// A `w` by `h` box of `color`, over a checkerboard where it is translucent, clipped to
/// `radius`.
fn paint(look: &Look, m: Metrics, w: f32, h: f32, radius: f32, color: Srgba) -> El {
    let c = &look.kit.c;
    let mut e = el().size(w, h).radius(radius).clip_rounded(radius);
    if color.is_opaque() {
        return e.fill(color.to_color());
    }
    e = e
        .fill(c.bg_base)
        .shape(checker(w, h, m.cell, c.border_base));
    e.child(
        el().pin(Some(0.0), Some(0.0), None, None)
            .size(w, h)
            .fill(color.to_color()),
    )
}

/// The button's face: the colour as a small rounded bar, over a checkerboard where
/// translucent.
pub(super) fn swatch_paint(look: &Look, m: Metrics, color: Srgba) -> El {
    paint(look, m, m.dot * 1.5, m.knob, look.kit.radius.sm, color)
        .border(1.0, look.kit.c.border_base)
        .a11y_hidden()
}

impl<S: 'static> ColorPicker<S> {
    /// A cursor: a ring around the colour under it, keyed `<part>-knob`, with the part's
    /// keyboard focus ring.
    fn cursor(&self, look: &Look, m: Metrics, key: String, x: f32, y: f32, fill: Srgba) -> El {
        let k = &look.kit;
        let c = &k.c;
        let ring = paint(look, m, m.knob, m.knob, m.knob / 2.0, fill)
            .border(2.0, c.knob)
            .focus_ring(2.0, 1.0, c.border_focus);
        k.elevate(
            el().key(key)
                .pin(Some(x - m.knob / 2.0), Some(y - m.knob / 2.0), None, None)
                .size(m.knob, m.knob)
                .radius(m.knob / 2.0)
                .child(ring)
                .a11y_hidden(),
            &k.elevation.knob,
        )
    }

    /// The saturation and value plane: the hue, white fading out to the right over it, and
    /// black fading in downwards over that. With sRGB blending (Rune's default) the layers
    /// composite to `v · (s · hue + 1 − s)`: HSV exactly.
    pub(super) fn plane(&self, st: &ColorPickerState, look: &Look, m: Metrics) -> El {
        let k = &look.kit;
        let hsv = st.hsv();
        let [w, h] = st.plane_size();
        let (aw, ah) = ((w - 2.0 * m.margin).max(0.0), (h - 2.0 * m.margin).max(0.0));
        let key = format!("{}-plane", self.key);
        let radius = k.radius.md;
        let layer = |e: El| {
            e.pin(Some(0.0), Some(0.0), None, None)
                .size(aw, ah)
                .radius(radius)
        };
        let area = el()
            .pin(Some(m.margin), Some(m.margin), None, None)
            .size(aw, ah)
            .radius(radius)
            .fill(hsv.pure_hue().to_color())
            .child(layer(el()).linear_gradient(
                FRAC_PI_2,
                [(0.0, Color::WHITE), (1.0, Color::WHITE.alpha(0.0))],
            ))
            .child(layer(el()).gradient(Color::TRANSPARENT, Color::BLACK))
            .a11y_hidden();
        let x = m.margin + hsv.s.clamp(0.0, 1.0) * aw;
        let y = m.margin + (1.0 - hsv.v.clamp(0.0, 1.0)) * ah;
        let words = &self.words;
        let mut plane = el()
            .key(key.clone())
            .role("slider")
            .label(words.plane)
            .a11y_value((words.plane_value)(percent(hsv.s), percent(hsv.v)))
            .w_full()
            .h(m.plane_h)
            .child(area)
            .child(self.cursor(look, m, format!("{key}-knob"), x, y, st.value().opaque()));
        let get_mut = self.get_mut;
        plane = plane.on_layout_tx(move |s: &mut S, r: [f32; 4], tx: &mut Tx| {
            if !get_mut(s).set_size(Part::Plane, r[2], r[3]) {
                tx.live_only();
            }
        });
        if self.disabled {
            return plane.disabled(true);
        }
        let w = self.wire();
        let drag = self.drag_handler(Part::Plane, m);
        plane
            .focusable(true)
            .cursor(Cursor::Crosshair)
            .on_drag(drag)
            // Arrows move a whole percent, Alt+arrows ten (GTK's 0.01 and 0.1): Left and Right
            // the saturation, Up and Down the value. Typed hex commits first, as for a strip.
            // Shift, Ctrl, Super and Cmd arrows pass on.
            .on_key(move |s: &mut S, e: &KeyEvent, tx: &mut Tx| {
                let mods = e.mods;
                if mods.shift || mods.ctrl || mods.cmd || mods.logo {
                    return false;
                }
                let n = if mods.alt { 10 } else { 1 };
                let (ds, dv) = match e.key {
                    Key::Left => (-n, 0),
                    Key::Right => (n, 0),
                    Key::Up => (0, n),
                    Key::Down => (0, -n),
                    _ => return false,
                };
                tx.instant();
                w.pending(s, tx);
                let moved = w.st(s).plane_step(ds, dv);
                w.commit(s, moved, tx);
                true
            })
    }

    /// The pointer on a draggable part: a press jumps its cursor there, a drag follows it, and
    /// Escape or a lost pointer puts the colour back.
    fn drag_handler(
        &self,
        part: Part,
        m: Metrics,
    ) -> impl Fn(&mut S, &mut Drag, &mut Tx) + 'static {
        let w = self.wire();
        move |s: &mut S, d: &mut Drag, tx: &mut Tx| {
            let change = match d.phase {
                Phase::Down => {
                    w.pending(s, tx);
                    w.st(s)
                        .press(part, d.x, d.y, m.margin)
                        .then_some(ColorChange::Live)
                }
                Phase::Move => w
                    .st(s)
                    .drag(d.x, d.y, m.margin)
                    .then_some(ColorChange::Live),
                Phase::Up => w.st(s).release(d.cancelled()),
            };
            w.tell(s, change, tx);
        }
    }

    /// The hue strip, or the alpha strip: a slider whose track draws what it sets.
    pub(super) fn strip(&self, st: &ColorPickerState, look: &Look, m: Metrics, part: Part) -> El {
        let k = &look.kit;
        let c = &k.c;
        let hsv = st.hsv();
        let width = st.strip_width(part);
        let tw = (width - 2.0 * m.margin).max(0.0);
        let words = &self.words;
        let (name, key) = match part {
            Part::Alpha => (words.alpha, format!("{}-alpha", self.key)),
            _ => (words.hue, format!("{}-hue", self.key)),
        };
        let radius = m.track / 2.0;
        let track_y = (m.strip_h - m.track) / 2.0;
        let track = el()
            .pin(Some(m.margin), Some(track_y), None, None)
            .size(tw, m.track)
            .radius(radius)
            .a11y_hidden();
        let (track, fraction, knob_fill, mut strip) = match part {
            Part::Alpha => {
                let opaque = st.value().opaque().to_color();
                let fade = el()
                    .pin(Some(0.0), Some(0.0), None, None)
                    .size(tw, m.track)
                    .linear_gradient(FRAC_PI_2, [(0.0, opaque.alpha(0.0)), (1.0, opaque)]);
                let pct = percent(hsv.a);
                (
                    track
                        .clip_rounded(radius)
                        .fill(c.bg_base)
                        .shape(checker(tw, m.track, m.cell, c.border_base))
                        .child(fade),
                    hsv.a.clamp(0.0, 1.0),
                    st.value(),
                    el().range_value(f64::from(pct), 0.0, 100.0)
                        .a11y_value((words.alpha_value)(pct)),
                )
            }
            _ => {
                let deg = hsv.h.round().clamp(0.0, 360.0) as u32;
                (
                    track.linear_gradient(FRAC_PI_2, hue_stops()),
                    (hsv.h / 360.0).clamp(0.0, 1.0),
                    hsv.pure_hue(),
                    el().range_value(f64::from(deg), 0.0, 360.0)
                        .a11y_value((words.hue_value)(deg)),
                )
            }
        };
        let x = m.margin + fraction * tw;
        strip = strip
            .key(key.clone())
            .role("slider")
            .label(name)
            .range_step(1.0)
            .w_full()
            .h(m.strip_h)
            .child(track)
            .child(self.cursor(
                look,
                m,
                format!("{key}-knob"),
                x,
                m.strip_h / 2.0,
                knob_fill,
            ));
        let get_mut = self.get_mut;
        strip = strip.on_layout_tx(move |s: &mut S, r: [f32; 4], tx: &mut Tx| {
            if !get_mut(s).set_size(part, r[2], r[3]) {
                tx.live_only();
            }
        });
        if self.disabled {
            return strip.disabled(true);
        }
        let (w1, w2) = (self.wire(), self.wire());
        strip
            .cursor(Cursor::Pointer)
            .on_drag(self.drag_handler(part, m))
            // Rune's slider keys: Left and Down a step down, Right and Up a step up, Page Up and
            // Page Down ten, and assistive increments and set-value.
            .on_step(move |s: &mut S, steps: i32, tx: &mut Tx| {
                tx.instant();
                w1.pending(s, tx);
                let moved = w1.st(s).strip_step(part, steps);
                w1.commit(s, moved, tx);
            })
            .on_step_limit(move |s: &mut S, end: bool, tx: &mut Tx| {
                tx.instant();
                w2.pending(s, tx);
                let moved = w2.st(s).strip_end(part, end);
                w2.commit(s, moved, tx);
            })
    }

    /// The current colour's swatch and the hex field.
    pub(super) fn hex_row(&self, st: &ColorPickerState, look: &Look, m: Metrics) -> El {
        let k = &look.kit;
        let c = &k.c;
        let font = k.ty.ui_compact;
        let height = look.frame.control + look.space.xs;
        let focused = st.edit.focused && !self.disabled;
        let invalid = st.invalid() && !self.disabled;
        let ink = if invalid && k.invalid_text {
            c.danger.fg
        } else {
            c.text_base
        };
        let get_mut = self.get_mut;
        let mut field = text_input(&st.edit, font)
            .single_line()
            .key(format!("{}-hex", self.key))
            .label(self.words.hex)
            .w_full()
            .align(Align::Start)
            .color(ink)
            .caret(ink, k.selection())
            .spellcheck(false)
            .invalid(invalid);
        if self.disabled {
            field = field.disabled(true);
        } else {
            let w = self.wire();
            field = field
                .on_edit(move |s: &mut S| &mut get_mut(s).edit)
                .on_input_change(move |s: &mut S, _: &mut Tx| get_mut(s).edited())
                .on_input_key(move |s: &mut S, e: &KeyEvent, tx: &mut Tx| {
                    // Typed text reverts first; then the open popover's cancel. A field's keys
                    // never reach the body's `on_key`, but they do reach the modal's dismissal.
                    if escape_key(e) {
                        return w.st(s).revert_text() || w.escape(s, tx);
                    }
                    if e.key != Key::Enter || e.mods != rune::Mods::default() {
                        return false;
                    }
                    let moved = w.st(s).commit_text();
                    w.commit(s, moved, tx);
                    true
                });
            if let Some(menu) = &self.menu {
                let menu = menu.clone();
                field = field.on_secondary_press(move |s: &mut S, p: PointerPress, tx: &mut Tx| {
                    if menu(s, Point::new(p.x, p.y), tx) {
                        get_mut(s).hold(true);
                    }
                });
            }
        }
        // The field in a box drawn as C20a's NumberField draws its own (rune-kit TextInput's
        // look, from the same tokens).
        let mut frame = row()
            .h(height)
            .grow(1.0)
            .px(look.space.sm)
            .items_center()
            .child(field)
            .radius(k.radius.md)
            .fill(c.bg_base)
            .token_motion(k.motion.control);
        frame = if self.disabled {
            k.elevate(frame, k.elevation.input())
        } else if invalid {
            frame.outline(1.0, -1.0, c.danger.fg)
        } else if focused {
            frame.outline(1.0, -1.0, c.border_focus)
        } else {
            k.elevate(frame, k.elevation.input()).hover(c.hover)
        };
        let side = height;
        let swatch = paint(look, m, side, side, k.radius.md, st.value())
            .border(1.0, c.border_base)
            .a11y_hidden();
        row()
            .w_full()
            .gap(look.space.sm)
            .items_center()
            .child(swatch)
            .child(frame)
    }

    /// A row of swatches under its heading: a radio group, each swatch checked while it is
    /// the colour. Arrows move between them; Space, Return or a click picks one.
    pub(super) fn swatch_row(
        &self,
        st: &ColorPickerState,
        look: &Look,
        m: Metrics,
        r: usize,
        row_data: &SwatchRow,
    ) -> El {
        let k = &look.kit;
        let c = &k.c;
        let value = st.value();
        let alpha = st.has_alpha();
        let mut dots = row()
            .key(format!("{}-swatches-{r}", self.key))
            .role("radiogroup")
            .label(row_data.label.clone())
            .w_full()
            .flex_wrap()
            .gap(look.space.xs)
            .focus_group();
        for (i, sw) in row_data.swatches.iter().enumerate() {
            let color = if alpha { sw.color } else { sw.color.opaque() };
            let current = color == value;
            let name = sw.label.clone().unwrap_or_else(|| color.hex());
            let mut dot = el()
                .key(format!("{}-swatch-{r}-{i}", self.key))
                .role("radio")
                .label(name)
                .checked(current)
                .focus_selected(current)
                .size(m.dot, m.dot)
                .radius(m.dot / 2.0)
                .child(
                    paint(look, m, m.dot, m.dot, m.dot / 2.0, color)
                        .border(1.0, c.border_base)
                        .a11y_hidden(),
                )
                .focus_ring(2.0, 3.0, c.border_focus);
            if current {
                dot = dot.outline(2.0, 1.0, c.text_base);
            }
            if self.disabled {
                dot = dot.disabled(true);
            } else {
                let w = self.wire();
                dot = dot.focusable(true).cursor(Cursor::Pointer).on_press_tx(
                    move |s: &mut S, tx: &mut Tx| {
                        tx.instant();
                        w.pending(s, tx);
                        let moved = w.st(s).pick(color);
                        w.commit(s, moved, tx);
                    },
                );
            }
            dots = dots.child(dot);
        }
        col()
            .w_full()
            .gap(look.space.xs)
            .child(
                text(row_data.label.clone())
                    .font(k.ty.small)
                    .color(c.text_muted)
                    .a11y_hidden(),
            )
            .child(dots)
    }
}
