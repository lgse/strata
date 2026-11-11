//! The number field's element: a spin button holding a text box frame with −, the number,
//! its unit and +.

use std::rc::Rc;

use rune::{
    Align, Cursor, El, Key, KeyEvent, PointerPress, Tx, WheelEvent, el, icon, row, text, text_input,
};
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use super::state::NumberFieldState;
use crate::contract::Point;
use crate::controls::slider::takes_wheel;

type Get<S> = fn(&S) -> &NumberFieldState;
type GetMut<S> = fn(&mut S) -> &mut NumberFieldState;
type Report<S> = Rc<dyn Fn(&mut S, f64, &mut Tx)>;
type Menu<S> = Rc<dyn Fn(&mut S, Point, &mut Tx) -> bool>;
/// Reports a step's or commit's new value, when it moved.
type Tell<S> = Rc<dyn Fn(&mut S, Option<f64>, &mut Tx)>;

/// A number field over a [`NumberFieldState`] the host holds: − before the number and +
/// after it, as Strata's spin buttons, with the unit between the number and +.
///
/// It reports a value through [`NumberField::on_change`] only when it commits one: a step
/// (−, +, Up, Down, Page Up, Page Down, Ctrl+Home, Ctrl+End, the wheel, an assistive
/// increment), Return, or the field losing the keyboard, which the host's poll notices
/// through [`NumberField::poll`].
pub struct NumberField<S> {
    key: String,
    get: Get<S>,
    get_mut: GetMut<S>,
    label: Option<String>,
    disabled: bool,
    report: Option<Report<S>>,
    menu: Option<Menu<S>>,
}

impl<S: 'static> NumberField<S> {
    /// The field keyed `key` (the spin button) over the state `get` and `get_mut` reach. Its
    /// text field is keyed `<key>-input`, and its steppers `<key>-decrease` and
    /// `<key>-increase`.
    pub fn new(key: impl Into<String>, get: Get<S>, get_mut: GetMut<S>) -> Self {
        NumberField {
            key: key.into(),
            get,
            get_mut,
            label: None,
            disabled: false,
            report: None,
            menu: None,
        }
    }

    /// The spin button's key.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The accessible name, of the spin button and its text field.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// A field that can't be used now: dimmed, out of the focus order, and deaf to input.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Hears each committed value that differs from the one before.
    pub fn on_change(mut self, f: impl Fn(&mut S, f64, &mut Tx) + 'static) -> Self {
        self.report = Some(Rc::new(f));
        self
    }

    /// The field's text menu: its right-click, Menu key and Shift+F10, at the window point
    /// Rune gives. The host opens C21's `FieldMenu` there and returns whether it opened; while
    /// it is open, losing the keyboard to it commits nothing. The host calls
    /// `NumberFieldState::hold(false)` from the menu's `on_close`, and `edited` from its
    /// `on_change`. Without it the field asks for Rune's own menu, which Linux doesn't show
    /// (U-C21-1).
    pub fn on_menu(mut self, f: impl Fn(&mut S, Point, &mut Tx) -> bool + 'static) -> Self {
        self.menu = Some(Rc::new(f));
        self
    }

    /// Commits the text when the field has lost the keyboard: call it from the host's poll
    /// every frame (Rune has no focus-change hook, upstream #106). True when it changed the
    /// state.
    pub fn poll(&self, s: &mut S, tx: &mut Tx) -> bool {
        let Some(moved) = (self.get_mut)(s).blurred() else {
            return false;
        };
        if let (Some(v), Some(f)) = (moved, &self.report) {
            f(s, v, tx);
        }
        true
    }

    pub fn build(self, s: &S, look: &Look) -> El {
        let st = (self.get)(s);
        let k = &look.kit;
        let c = &k.c;
        let range = st.range();
        let font = k.ty.ui_compact;
        let height = look.frame.control + look.space.xs;
        let focused = st.edit.focused && !self.disabled;
        let invalid = st.invalid() && !self.disabled;
        let ink = if invalid && k.invalid_text {
            c.danger.fg
        } else {
            c.text_base
        };
        // Wide enough for either end, so the box doesn't change width as the value does.
        let digits = [range.format(range.min()), range.format(range.max())]
            .iter()
            .map(|t| rune::text::advance(t, font))
            .fold(0.0f32, f32::max);
        let get_mut = self.get_mut;
        let report = self.report.clone();
        let tell: Tell<S> = Rc::new(move |s: &mut S, moved: Option<f64>, tx: &mut Tx| {
            if let (Some(v), Some(f)) = (moved, &report) {
                f(s, v, tx);
            }
        });

        let mut field = text_input(&st.edit, font)
            .single_line()
            .key(format!("{}-input", self.key))
            .w(digits.ceil() + look.space.xs)
            .align(Align::Center)
            .color(ink)
            .caret(ink, k.selection())
            .spellcheck(false)
            .invalid(invalid);
        if let Some(l) = &self.label {
            field = field.label(l.clone());
        }
        if self.disabled {
            // A field without `on_edit` takes neither the keyboard nor a press.
            field = field.disabled(true);
        } else {
            let t = tell.clone();
            field = field
                .on_edit(move |s: &mut S| &mut get_mut(s).edit)
                .on_input_change(move |s: &mut S, _: &mut Tx| get_mut(s).edited())
                .on_input_key(move |s: &mut S, e: &KeyEvent, tx: &mut Tx| {
                    let Some(moved) = key_step(get_mut(s), e) else {
                        return false;
                    };
                    t(s, moved, tx);
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

        let mut value = row().gap(look.space.xxs).items_center().child(field);
        if !st.unit().is_empty() {
            value = value.child(
                text(st.unit().to_owned())
                    .font(font)
                    .color(c.text_muted)
                    .a11y_hidden(),
            );
        }
        let at_min = st.value() <= range.min();
        let at_max = st.value() >= range.max();
        // The steppers sit inside the text box frame, so a press on one leaves the keyboard
        // in the field (Rune keeps focus for a press inside a `textbox`, probe P2b).
        let mut frame = row()
            .role("textbox")
            .h(height)
            .px(look.space.xxs)
            .gap(look.space.xxs)
            .items_center()
            .child(self.stepper(look, false, at_min, tell.clone()))
            .child(value)
            .child(self.stepper(look, true, at_max, tell.clone()))
            .radius(k.radius.md)
            .fill(c.bg_base)
            .token_motion(k.motion.control);
        frame = if self.disabled {
            k.elevate(frame, k.elevation.input()).layer_opacity(0.5)
        } else if invalid {
            frame.outline(1.0, -1.0, c.danger.fg)
        } else if focused {
            frame.outline(1.0, -1.0, c.border_focus)
        } else {
            k.elevate(frame, k.elevation.input()).hover(c.hover)
        };
        if !self.disabled
            && let Some(menu) = &self.menu
        {
            let menu = menu.clone();
            frame = frame.on_secondary_press(move |s: &mut S, p: PointerPress, tx: &mut Tx| {
                if menu(s, Point::new(p.x, p.y), tx) {
                    get_mut(s).hold(true);
                }
            });
        }

        // The spin button assistive technology steps; the field inside it has the keyboard
        // (a text field can't take the role itself: Rune draft U-C20a-1).
        let mut spin = el()
            .key(self.key.clone())
            .role("spinbutton")
            .range_value(st.value(), range.min(), range.max())
            .range_step(range.step())
            .a11y_value(st.value_text())
            .invalid(invalid)
            .child(frame);
        if let Some(l) = &self.label {
            spin = spin.label(l.clone());
        }
        if self.disabled {
            return spin.disabled(true);
        }
        let t = tell.clone();
        spin.on_step(move |s: &mut S, steps: i32, tx: &mut Tx| {
            let moved = get_mut(s).step(steps);
            t(s, moved, tx);
        })
        // `on_step` makes it focusable: the keyboard belongs to the field inside.
        .focusable(false)
        .on_wheel(move |s: &mut S, w: WheelEvent, tx: &mut Tx| {
            if !takes_wheel(&w) {
                return false;
            }
            let moved = get_mut(s).wheel(w.dy, w.precise);
            tell(s, moved, tx);
            true
        })
    }

    /// − or +: hidden from assistive technology, which steps the spin button itself, and
    /// never focused, as GTK's are. Insensitive at its end.
    fn stepper(&self, look: &Look, up: bool, at_end: bool, tell: Tell<S>) -> El {
        let k = &look.kit;
        let side = look.frame.control;
        let glyph = look.icon(if up { IconName::Plus } else { IconName::Minus });
        let off = self.disabled || at_end;
        let mut b = el()
            .key(format!(
                "{}-{}",
                self.key,
                if up { "increase" } else { "decrease" }
            ))
            .role("button")
            .size(side, side)
            .radius(k.radius.sm)
            .center()
            .child(icon(glyph, look.icon_size.md).color(if off {
                k.c.icon_faint
            } else {
                k.c.icon_muted
            }))
            .a11y_hidden();
        if off {
            return b;
        }
        let get_mut = self.get_mut;
        b = b
            .cursor(Cursor::Pointer)
            .hit()
            .hover(k.c.hover)
            .press(k.c.pressed)
            .on_press_tx(move |s: &mut S, tx: &mut Tx| {
                let moved = get_mut(s).step(if up { 1 } else { -1 });
                tell(s, moved, tx);
            })
            .focusable(false);
        b
    }
}

/// The keys a focused field steps on, as GTK's spin button: Up and Down a step, Page Up and
/// Down a page, Ctrl+Home and Ctrl+Page Down the minimum, Ctrl+End and Ctrl+Page Up the
/// maximum, and Return commits the text. `None` for keys the editor keeps (plain Home and
/// End move the caret); `Some` with the new value when it moved.
pub(super) fn key_step(st: &mut NumberFieldState, e: &KeyEvent) -> Option<Option<f64>> {
    let m = e.mods;
    if m.shift || m.alt || m.logo {
        return None;
    }
    let page = st.range().page_steps();
    if m.ctrl {
        return match e.key {
            Key::Home | Key::PageDown => Some(st.jump_to_end(false)),
            Key::End | Key::PageUp => Some(st.jump_to_end(true)),
            _ => None,
        };
    }
    if m.cmd {
        return None;
    }
    match e.key {
        Key::Enter => Some(st.commit_text()),
        Key::Up => Some(st.step(1)),
        Key::Down => Some(st.step(-1)),
        Key::PageUp => Some(st.step(page)),
        Key::PageDown => Some(st.step(-page)),
        _ => None,
    }
}
