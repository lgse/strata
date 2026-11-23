// SPDX-License-Identifier: MIT

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

/// Builds the rename editor as a row slot (`RenameEditor::slot`), for `Collection::rename`.
pub struct RenameEditor<S> {
    get: GetFn<S>,
    get_mut: GetMutFn<S>,
    validate: Option<ValidateFn<S>>,
    font: FontFn,
    align: Align,
    label: Option<LabelFn<S>>,
    focus_return: Option<String>,
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
            focus_return: self.focus_return.clone(),
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
            focus_return: None,
            on_intent: None,
        }
    }

    /// The host's rules for a name: `Some(message)` refuses it.
    pub fn validate(mut self, f: impl Fn(&S, EntryId, &str) -> Option<String> + 'static) -> Self {
        self.validate = Some(Rc::new(f));
        self
    }

    /// The field's font: the label's, which it replaces.
    pub fn font(mut self, f: impl Fn(&Look) -> TextStyle + 'static) -> Self {
        self.font = Rc::new(f);
        self
    }

    /// Where the text sits in the field: `Align::Center` in Icons, as Strata's caption editor.
    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    /// The field's accessible name. By default "Rename", as Strata's.
    pub fn label(mut self, f: impl Fn(&S, &RowCx<'_>) -> String + 'static) -> Self {
        self.label = Some(Rc::new(f));
        self
    }

    /// The key of the element that takes the keyboard when the field goes while it has it.
    pub fn focus_return(mut self, key: impl Into<String>) -> Self {
        self.focus_return = Some(key.into());
        self
    }

    /// Hears the editor's intents from its own input (keys, its menu press).
    pub fn on_intent(mut self, f: impl Fn(&mut S, RenameIntent, &mut Tx) + 'static) -> Self {
        self.on_intent = Some(Rc::new(f));
        self
    }

    /// The slot for `Collection::rename`.
    pub fn slot(&self) -> Slot<S> {
        let cfg = Rc::new(self.clone());
        Slot::pure(move |s: &S, rcx: &RowCx<'_>| build(&cfg, s, rcx))
    }

    /// The validator's message while it refuses the name, for the host's window overlay layer (`rune_kit::root`'s).
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
        .on_secondary_press(menu_press(cfg));
    if let Some(message) = error {
        field = field.invalid(true).description(message.to_owned());
    }

    // Use `textbox` role so margin presses route into the field rather than dismissing the edit;
    // absorb middle-button presses with a no-op drag handler so they do not bubble to the
    // collection and open the row in a new tab.
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
        .focus_return(returns_to(cfg, rcx))
        .child(field)
}

fn returns_to<S>(cfg: &RenameEditor<S>, rcx: &RowCx<'_>) -> String {
    let named = cfg.focus_return.clone();
    named.unwrap_or_else(|| rcx.scope.to_owned())
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
        .role("status")
        .live_region(LiveRegion::Polite)
}
