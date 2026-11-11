//! The editor's element: a field in the label's place, with the validator's message under it.

use std::rc::Rc;

use rune::{
    Align, Drag, El, KeyEvent, LiveRegion, PointerButton, PointerPress, Side, TextStyle, Tx, col,
    row, text, text_input,
};
use strata_kit::look::Look;

use super::intent::{Polled, RenameIntent};
use super::keys::{EDITOR, FIELD, MESSAGE, MESSAGE_TEXT};
use super::state::RenameState;
use crate::collection::element::{RowCx, Slot};
use crate::contract::{EntryId, Point};

type GetFn<S> = Rc<dyn Fn(&S) -> &RenameState>;
type GetMutFn<S> = Rc<dyn Fn(&mut S) -> &mut RenameState>;
type ValidateFn<S> = Rc<dyn Fn(&S, EntryId, &str) -> Option<String>>;
type FontFn = Rc<dyn Fn(&Look) -> TextStyle>;
type LabelFn<S> = Rc<dyn Fn(&S, &RowCx<'_>) -> String>;
type IntentFn<S> = Rc<dyn Fn(&mut S, RenameIntent, &mut Tx)>;

/// Builds the rename editor as a C1e row slot ([`RenameEditor::slot`]), for
/// `Collection::rename`:
///
/// ```ignore
/// let editor = RenameEditor::new(|s: &App| &s.rename, |s: &mut App| &mut s.rename)
///     .validate(|s: &App, id, name| s.name_error(id, name))
///     .on_intent(|s: &mut App, intent, tx| s.renamed(intent, tx));
/// Collection::new(KEY, ..).rename(|s: &App| s.rename.renaming_in(KEY), editor.slot())
/// ```
///
/// Used by: hosts (C2, C3, C4, Strata).
pub struct RenameEditor<S> {
    get: GetFn<S>,
    get_mut: GetMutFn<S>,
    validate: Option<ValidateFn<S>>,
    font: FontFn,
    align: Align,
    label: Option<LabelFn<S>>,
    on_intent: Option<IntentFn<S>>,
}

impl<S> Clone for RenameEditor<S> {
    fn clone(&self) -> Self {
        Self {
            get: self.get.clone(),
            get_mut: self.get_mut.clone(),
            validate: self.validate.clone(),
            font: self.font.clone(),
            align: self.align,
            label: self.label.clone(),
            on_intent: self.on_intent.clone(),
        }
    }
}

impl<S: 'static> RenameEditor<S> {
    /// An editor over the [`RenameState`] `get` and `get_mut` reach.
    pub fn new(
        get: impl Fn(&S) -> &RenameState + 'static,
        get_mut: impl Fn(&mut S) -> &mut RenameState + 'static,
    ) -> Self {
        Self {
            get: Rc::new(get),
            get_mut: Rc::new(get_mut),
            validate: None,
            font: Rc::new(|look: &Look| look.kit.ty.ui_compact),
            align: Align::Start,
            label: None,
            on_intent: None,
        }
    }

    /// The host's rules for a name (Strata's): `Some(message)` refuses it, and the editor
    /// shows the message under the field and gives it to assistive technology. Called each
    /// time the text changes; an empty field is never refused. A name holding a `/` that it
    /// accepts closes as [`RenameIntent::Move`]. Without one, every name is accepted.
    pub fn validate(mut self, f: impl Fn(&S, EntryId, &str) -> Option<String> + 'static) -> Self {
        self.validate = Some(Rc::new(f));
        self
    }

    /// The field's font: the label's, which it replaces. By default `ty.ui_compact`, C1e's
    /// default label font.
    pub fn font(mut self, f: impl Fn(&Look) -> TextStyle + 'static) -> Self {
        self.font = Rc::new(f);
        self
    }

    /// Where the text sits in the field: [`Align::Center`] in Icons, as Strata's caption
    /// editor; [`Align::Start`] (the default) in List and Columns.
    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    /// The field's accessible name. By default "Rename", as Strata's.
    pub fn label(mut self, f: impl Fn(&S, &RowCx<'_>) -> String + 'static) -> Self {
        self.label = Some(Rc::new(f));
        self
    }

    /// Hears the editor's intents from its own input (keys, its menu press). Intents from
    /// the host's calls ([`RenameState::tick`], `begin`, `commit`) come back from them.
    pub fn on_intent(mut self, f: impl Fn(&mut S, RenameIntent, &mut Tx) + 'static) -> Self {
        self.on_intent = Some(Rc::new(f));
        self
    }

    /// The slot for `Collection::rename`. Rows with a rename open aren't memoized, so the
    /// slot needs no version.
    pub fn slot(&self) -> Slot<S> {
        let cfg = Rc::new(self.clone());
        Slot::pure(move |s: &S, rcx: &RowCx<'_>| build(&cfg, s, rcx))
    }

    /// The validator's message while it refuses the name, for the host's window overlay
    /// layer (`rune_kit::root`'s): anchored under the field, drawn above the rows, and flipped
    /// over the field only at the window's edge (not the listing's). Inside the row it would
    /// be clipped to the row, whose height it mustn't change. A polite live region, so
    /// assistive technology says each new message; the field's description holds it too. It
    /// takes no presses: one on it reaches what lies under it and ends the edit.
    pub fn message(&self, s: &S, look: &Look) -> Option<El> {
        let st = (self.get)(s);
        st.renaming()?;
        st.error().map(|m| message_el(look, m))
    }
}

/// Sends what a handler's call did to the host.
fn route<S>(cfg: &RenameEditor<S>, s: &mut S, polled: Polled, tx: &mut Tx) {
    if let Some(f) = &cfg.on_intent {
        for intent in polled.intents {
            f(s, intent, tx);
        }
    }
}

fn build<S: 'static>(cfg: &Rc<RenameEditor<S>>, s: &S, rcx: &RowCx<'_>) -> El {
    let st = (cfg.get)(s);
    let look = rcx.look;
    let kit = &look.kit;
    let error = st.error();
    let ink = kit.c.text_base;
    let label = match &cfg.label {
        Some(f) => f(s, rcx),
        None => "Rename".to_owned(),
    };

    let (edit_cfg, key_cfg, change_cfg) = (cfg.clone(), cfg.clone(), cfg.clone());
    let mut field = text_input(&st.edit, (cfg.font)(look))
        .single_line()
        .key(FIELD)
        .label(label)
        .color(ink)
        .caret(ink, kit.selection())
        // A file name isn't prose: no misspelling marks (Rune has no checker on Linux, but
        // the field says so wherever it runs).
        .spellcheck(false)
        .align(cfg.align)
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .on_edit(move |s: &mut S| &mut (edit_cfg.get_mut)(s).edit)
        .on_input_key(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| {
            let repeat = tx.key_repeat();
            match (key_cfg.get_mut)(s).key(ev, repeat) {
                Some(polled) => {
                    route(&key_cfg, s, polled, tx);
                    true
                }
                None => false,
            }
        })
        .on_input_change(move |s: &mut S, _tx: &mut Tx| {
            let answer = {
                let st = (change_cfg.get)(s);
                match (&change_cfg.validate, st.renaming()) {
                    (Some(f), Some(id)) if !st.edit.text.is_empty() => f(s, id, &st.edit.text),
                    _ => None,
                }
            };
            (change_cfg.get_mut)(s).validated(answer);
        })
        // The field's own menu press: without it Rune asks for its native field menu, which
        // shows nothing on Linux at the pinned rev. The host's menu (C21) answers the intent.
        .on_secondary_press(menu_press(cfg));
    if let Some(message) = error {
        field = field.invalid(true).description(message.to_owned());
    }

    // The box is a `textbox`, as rune-kit's text inputs are: Rune routes a press anywhere in it
    // to the field (`runtime/input/hit.rs`, `textbox_input`), so a press in its margin, off the
    // text line, edits rather than reaching the collection under it and ending the edit. Its
    // menu press is the field's too (which gives the box a context-menu action of its own:
    // without it, a right-click in the margin would open the row's menu).
    //
    // The `textbox` routing is for primary presses only. A middle press walks up to the
    // nearest element that takes the middle button (`runtime/input/gesture.rs`,
    // `button_down`), which would be the collection's root: the row being renamed would open in
    // a new tab. So the box takes the middle button itself, with a drag handler that does
    // nothing (an empty `on_press` would give it a click action). Back and Forward stay the
    // host's: its own handling commits the edit (D4).
    row()
        .key(EDITOR)
        .role("textbox")
        .on_secondary_press(menu_press(cfg))
        .pointer_buttons([PointerButton::Middle])
        .on_drag(|_: &mut S, _: &mut Drag, _: &mut Tx| {})
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .px(look.space.xs)
        .center_y()
        .radius(kit.radius.sm)
        .fill(kit.c.bg_base)
        .outline(
            1.0,
            -1.0,
            if error.is_some() {
                kit.c.danger.fg
            } else {
                kit.c.border_focus
            },
        )
        // Enter, Escape or a last Tab removes the field with the keyboard in it: give the
        // keyboard back to the collection, as Strata keeps it in the listing. A `focus_return`
        // covers every way the field goes, and only while it holds the keyboard.
        .focus_return(rcx.scope.to_owned())
        .child(field)
}

/// The field's menu press, on the field and on the box around it.
fn menu_press<S: 'static>(
    cfg: &Rc<RenameEditor<S>>,
) -> impl Fn(&mut S, PointerPress, &mut Tx) + 'static {
    let cfg = cfg.clone();
    move |s: &mut S, p: PointerPress, tx: &mut Tx| {
        let polled = (cfg.get)(s).field_menu(Point::new(p.x, p.y));
        route(&cfg, s, polled, tx);
    }
}

/// The validator's message: under the field, or over it at the window's edge.
fn message_el(look: &Look, message: &str) -> El {
    let kit = &look.kit;
    col()
        .key(MESSAGE)
        .anchored(FIELD, Side::Bottom, Align::Start, look.space.xxs)
        .radius(kit.radius.sm)
        .fill(kit.c.bg_base)
        .child(
            col()
                .radius(kit.radius.sm)
                .fill(kit.c.danger.bg)
                .outline(1.0, -1.0, kit.c.danger.border)
                .px(look.space.sm)
                .py(look.space.xxs)
                .child(
                    text(message.to_owned())
                        .key(MESSAGE_TEXT)
                        .font(kit.ty.small)
                        .color(kit.c.danger.fg),
                ),
        )
        // A status: ARIA's role for a polite message. It takes no presses: one on it reaches
        // what lies under it (the row below), and ends the edit as any press outside the
        // field does, as Strata's. (`.hit()` wouldn't make it a press target either: it marks
        // a rect as native overlay input, `Runtime::hit_regions`, and `top_target` skips such
        // a node unless it is a button.)
        .role("status")
        .live_region(LiveRegion::Polite)
}
