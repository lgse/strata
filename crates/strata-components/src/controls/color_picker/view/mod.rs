//! The picker's element: the plane, the strips, the hex row and the swatch rows
//! (`parts.rs`), inline or in a popover behind a button (`button.rs`).

mod button;
mod parts;

use std::rc::Rc;

use rune::{El, Key, KeyEvent, Tx, col};
use strata_kit::look::Look;

use super::state::{ColorChange, ColorPickerState};
use super::swatch::SwatchRow;
use super::value::Srgba;
use super::words::PickerWords;
use crate::contract::Point;

pub(super) type Get<S> = fn(&S) -> &ColorPickerState;
pub(super) type GetMut<S> = fn(&mut S) -> &mut ColorPickerState;
type Report<S> = Rc<dyn Fn(&mut S, Srgba, ColorChange, &mut Tx)>;
type Menu<S> = Rc<dyn Fn(&mut S, Point, &mut Tx) -> bool>;
type Closed<S> = Rc<dyn Fn(&mut S, bool, &mut Tx)>;

/// Escape as Rune's modal dismisses on it: bare, or with Shift held, but no Ctrl, Alt, Cmd or
/// Super (`focus_key` in rune's `runtime/input/focus.rs`). Whatever the modal would dismiss on,
/// the picker takes first as a cancel, so no Escape keeps the colour as a press outside does.
pub(super) fn escape_key(e: &KeyEvent) -> bool {
    let m = e.mods;
    e.key == Key::Escape && !(m.ctrl || m.alt || m.cmd || m.logo)
}

/// Sizes, all from Look tokens, so they follow the text size.
#[derive(Clone, Copy)]
pub(super) struct Metrics {
    /// A cursor's diameter: `icon_size.md` (16 at 13 px).
    knob: f32,
    /// The inset of a part's drawn area inside its box: half a cursor, so the cursor stays in.
    margin: f32,
    /// The picker's width: ten `frame.control` (240 at 13 px).
    width: f32,
    /// The plane's box height: six `frame.control` plus a cursor (160).
    plane_h: f32,
    /// A strip's box height, its hit area: `frame.control` (24).
    strip_h: f32,
    /// A strip's drawn thickness: three quarters of a cursor (12).
    track: f32,
    /// A swatch's diameter: `icon_size.lg` (20, Strata's colour dots).
    dot: f32,
    /// The checkerboard's square under translucent colour: `space.sm` (6).
    cell: f32,
}

impl Metrics {
    fn of(look: &Look) -> Metrics {
        let control = look.frame.control;
        let knob = look.icon_size.md.min(control);
        Metrics {
            knob,
            margin: knob / 2.0,
            width: control * 10.0,
            plane_h: control * 6.0 + knob,
            strip_h: control,
            track: (knob * 0.75).round(),
            dot: look.icon_size.lg,
            cell: look.space.sm,
        }
    }
}

/// What the handlers share: the state's accessor and the host's hooks.
pub(super) struct Wire<S> {
    get_mut: GetMut<S>,
    report: Option<Report<S>>,
    closed: Option<Closed<S>>,
}

impl<S> Clone for Wire<S> {
    fn clone(&self) -> Self {
        Wire {
            get_mut: self.get_mut,
            report: self.report.clone(),
            closed: self.closed.clone(),
        }
    }
}

impl<S> Wire<S> {
    fn st<'a>(&self, s: &'a mut S) -> &'a mut ColorPickerState {
        (self.get_mut)(s)
    }

    /// Reports `change`, with the colour now in the state.
    fn tell(&self, s: &mut S, change: Option<ColorChange>, tx: &mut Tx) {
        if let (Some(change), Some(f)) = (change, &self.report) {
            let v = (self.get_mut)(s).value();
            f(s, v, change, tx);
        }
    }

    /// Reports a commit when the colour moved.
    fn commit(&self, s: &mut S, moved: bool, tx: &mut Tx) {
        self.tell(s, moved.then_some(ColorChange::Commit), tx);
    }

    /// Commits typed hex text before another part changes the colour.
    fn pending(&self, s: &mut S, tx: &mut Tx) {
        let moved = self.st(s).commit_pending();
        self.commit(s, moved, tx);
    }

    /// Closes the popover, putting the opening colour back on `cancel`, and reports it.
    fn close(&self, s: &mut S, cancel: bool, tx: &mut Tx) {
        let (changed, moved) = self.st(s).close(cancel);
        self.closed_with(s, changed, moved, cancel, tx);
    }

    fn closed_with(&self, s: &mut S, changed: bool, moved: bool, cancel: bool, tx: &mut Tx) {
        let change = if cancel {
            ColorChange::Cancel
        } else {
            ColorChange::Commit
        };
        self.tell(s, moved.then_some(change), tx);
        if let Some(f) = &self.closed {
            f(s, changed, tx);
        }
    }

    /// Escape: in the open popover it closes it and puts the opening colour back (true);
    /// inline it is the host's (false).
    fn escape(&self, s: &mut S, tx: &mut Tx) -> bool {
        if !self.st(s).is_open() {
            return false;
        }
        tx.instant();
        self.close(s, true, tx);
        true
    }
}

/// A colour picker over a [`ColorPickerState`] the host holds: a saturation and value plane,
/// a hue strip, an alpha strip (with [`ColorPickerState::alpha`]), a hex field and rows of
/// swatches, inline ([`ColorPicker::build`]) or in a popover behind a button
/// ([`ColorPicker::button`]).
///
/// It reports through [`ColorPicker::on_change`]: [`ColorChange::Live`] while a drag moves the
/// colour, [`ColorChange::Commit`] when it settles, and [`ColorChange::Cancel`] when a drag is
/// cancelled or the popover closes on Escape. The colour is already in the state when the
/// handler runs. Nothing is reported when the colour didn't change.
pub struct ColorPicker<S> {
    key: String,
    get: Get<S>,
    get_mut: GetMut<S>,
    label: Option<String>,
    words: PickerWords,
    rows: Vec<SwatchRow>,
    disabled: bool,
    report: Option<Report<S>>,
    menu: Option<Menu<S>>,
    closed: Option<Closed<S>>,
}

impl<S: 'static> ColorPicker<S> {
    /// The picker keyed `key` over the state `get` and `get_mut` reach. Its parts are keyed
    /// `<key>-plane`, `<key>-hue`, `<key>-alpha`, `<key>-hex` (the text field) and
    /// `<key>-swatches-<r>` (row `r`) holding `<key>-swatch-<r>-<i>`; with
    /// [`ColorPicker::button`], `<key>-button` and the popover `<key>-popover-content`.
    pub fn new(key: impl Into<String>, get: Get<S>, get_mut: GetMut<S>) -> Self {
        ColorPicker {
            key: key.into(),
            get,
            get_mut,
            label: None,
            words: PickerWords::default(),
            rows: Vec::new(),
            disabled: false,
            report: None,
            menu: None,
            closed: None,
        }
    }

    /// The picker's accessible name, and the button's ("Background", "Folder color").
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The parts' names and value texts, in place of the English ones.
    pub fn words(mut self, words: PickerWords) -> Self {
        self.words = words;
        self
    }

    /// The swatch rows, top first: theme colours, recent colours.
    pub fn swatches(mut self, rows: Vec<SwatchRow>) -> Self {
        self.rows = rows;
        self
    }

    /// A picker that can't be used now: dimmed, out of the focus order, deaf to input; the
    /// button doesn't open, and an open popover closes ([`ColorPicker::poll`]).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Hears every change, with the new colour.
    pub fn on_change(mut self, f: impl Fn(&mut S, Srgba, ColorChange, &mut Tx) + 'static) -> Self {
        self.report = Some(Rc::new(f));
        self
    }

    /// The hex field's text menu, as C20a's `NumberField::on_menu`: its right-click, Menu key
    /// and Shift+F10, at the window point Rune gives. The host opens C21's `FieldMenu` there
    /// and returns whether it opened; while it is open, losing the keyboard to it commits
    /// nothing. The host calls `ColorPickerState::hold(false)` from the menu's `on_close`, and
    /// `edited` from its `on_change`.
    pub fn on_menu(mut self, f: impl Fn(&mut S, Point, &mut Tx) -> bool + 'static) -> Self {
        self.menu = Some(Rc::new(f));
        self
    }

    /// Hears the popover close ([`ColorPicker::button`]), with whether the colour differs from
    /// the one it opened with: the host remembers it in its recent colours then
    /// ([`super::remember`]). Escape always closes it unchanged.
    pub fn on_close(mut self, f: impl Fn(&mut S, bool, &mut Tx) + 'static) -> Self {
        self.closed = Some(Rc::new(f));
        self
    }

    /// The picker's root key.
    pub fn key(&self) -> &str {
        &self.key
    }

    fn wire(&self) -> Wire<S> {
        Wire {
            get_mut: self.get_mut,
            report: self.report.clone(),
            closed: self.closed.clone(),
        }
    }

    /// Call from the host's poll every frame, built with the view's [`ColorPicker::disabled`]:
    /// commits typed hex text once the field has lost the keyboard (Rune has no focus-change
    /// hook, upstream #106), and closes an open popover once the picker is disabled, keeping
    /// the colour as a press outside does, so enabling it again doesn't bring the popover back.
    /// True when it changed the state.
    pub fn poll(&self, s: &mut S, tx: &mut Tx) -> bool {
        let w = self.wire();
        let mut changed = false;
        if self.disabled && w.st(s).is_open() {
            w.close(s, false, tx);
            changed = true;
        }
        if let Some(moved) = w.st(s).blurred() {
            w.commit(s, moved, tx);
            changed = true;
        }
        changed
    }

    /// The picker inline, `width` wide at most.
    pub fn build(self, s: &S, look: &Look) -> El {
        let m = Metrics::of(look);
        let width = m.width;
        self.body(s, look, m, width)
    }

    fn body(&self, s: &S, look: &Look, m: Metrics, width: f32) -> El {
        let st = (self.get)(s);
        let mut body = col()
            .key(self.key.clone())
            .role("group")
            .w(width)
            .gap(look.space.md)
            .instant_changes()
            .child(self.plane(st, look, m))
            .child(self.strip(st, look, m, super::state::Part::Hue));
        if let Some(l) = &self.label {
            body = body.label(l.clone());
        }
        if st.has_alpha() {
            body = body.child(self.strip(st, look, m, super::state::Part::Alpha));
        }
        // In line with the plane's and strips' drawn edges, half a cursor in.
        body = body.child(self.hex_row(st, look, m).px(m.margin));
        for (r, row) in self.rows.iter().enumerate() {
            body = body.child(self.swatch_row(st, look, m, r, row).px(m.margin));
        }
        if self.disabled {
            return body.disabled(true).layer_opacity(0.5);
        }
        // Escape with a strip, the plane or a swatch focused (the field takes its own): the
        // open popover's cancel, ahead of Rune's modal dismissal, which can't tell Escape from a
        // press outside (draft U-C20b-2).
        let w = self.wire();
        body.on_key(move |s: &mut S, e: &KeyEvent, tx: &mut Tx| escape_key(e) && w.escape(s, tx))
    }
}
