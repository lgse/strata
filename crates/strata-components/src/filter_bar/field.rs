// SPDX-License-Identifier: MIT

use std::rc::Rc;

use rune::{El, KeyEvent, PointerPress, TextEdit, TextStyle, Tx, row, text_input};
use strata_kit::look::Look;

use crate::contract::Point;

pub(crate) type EditFn<S> = Rc<dyn Fn(&mut S) -> &mut TextEdit>;
pub(crate) type KeyFn<S> = Rc<dyn Fn(&mut S, &KeyEvent, &mut Tx) -> bool>;
pub(crate) type ChangeFn<S> = Rc<dyn Fn(&mut S, &mut Tx)>;
pub(crate) type MenuFn<S> = Rc<dyn Fn(&mut S, Point, &mut Tx)>;

/// What a field draws and names.
pub(crate) struct Spec<'a> {
    /// The field's key.
    pub key: &'a str,
    /// Its accessible name.
    pub label: &'a str,
    pub placeholder: Option<&'a str>,
    /// Its accessible description (a prompt's hint).
    pub description: Option<&'a str>,
    pub invalid: bool,
    pub font: TextStyle,
}

/// The handlers a field sends its input to.
pub(crate) struct Handlers<S> {
    pub edit: EditFn<S>,
    pub key: KeyFn<S>,
    pub change: ChangeFn<S>,
    pub menu: MenuFn<S>,
}

impl<S> Clone for Handlers<S> {
    fn clone(&self) -> Self {
        Self {
            edit: self.edit.clone(),
            key: self.key.clone(),
            change: self.change.clone(),
            menu: self.menu.clone(),
        }
    }
}

pub(crate) fn input<S: 'static>(
    look: &Look,
    edit: &TextEdit,
    spec: &Spec<'_>,
    h: &Handlers<S>,
) -> El {
    let k = &look.kit;
    // A refused text turns red only where the theme says so, as rune-kit's fields do.
    let ink = if spec.invalid && k.invalid_text {
        k.c.danger.fg
    } else {
        k.c.text_base
    };
    let (on_edit, on_key, on_change, on_menu) = (
        h.edit.clone(),
        h.key.clone(),
        h.change.clone(),
        h.menu.clone(),
    );
    let mut field = text_input(edit, spec.font)
        .single_line()
        .key(spec.key.to_owned())
        .label(spec.label.to_owned())
        .color(ink)
        .caret(ink, k.selection())
        .spellcheck(false)
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .on_edit(move |s: &mut S| on_edit(s))
        .on_input_key(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| on_key(s, ev, tx))
        .on_input_change(move |s: &mut S, tx: &mut Tx| on_change(s, tx))
        .on_secondary_press(move |s: &mut S, p: PointerPress, tx: &mut Tx| {
            on_menu(s, Point::new(p.x, p.y), tx)
        })
        .invalid(spec.invalid);
    if let Some(p) = spec.placeholder {
        field = field.placeholder(p.to_owned(), k.c.text_faint);
    }
    if let Some(d) = spec.description {
        field = field.description(d.to_owned());
    }
    field
}

pub(crate) fn frame(look: &Look, inner: El, focused: bool, invalid: bool) -> El {
    let k = &look.kit;
    let e = inner
        .role("textbox")
        .radius(k.radius.md)
        .fill(k.c.bg_base)
        .token_motion(k.motion.control);
    if invalid {
        e.outline(1.0, -1.0, k.c.danger.fg)
    } else if focused {
        e.outline(1.0, -1.0, k.c.border_focus)
    } else {
        k.elevate(e, k.elevation.input())
    }
}

/// A row for [`frame`]: the field's height, its content centred.
pub(crate) fn frame_row(look: &Look, key: String) -> El {
    row()
        .key(key)
        .w_full()
        .h(look.frame.control)
        .px(look.space.sm)
        .gap(look.space.xs)
        .items_center()
        .min_w(0.0)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct FocusWatch {
    seen: bool,
}

impl FocusWatch {
    /// Forget the field: it is new, and hasn't had the keyboard yet.
    pub fn reset(&mut self) {
        self.seen = false;
    }

    /// True once, the first frame the field is seen without the keyboard after having had it.
    pub fn lost(&mut self, focused: bool) -> bool {
        if focused {
            self.seen = true;
            false
        } else {
            std::mem::take(&mut self.seen)
        }
    }
}
