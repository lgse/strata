//! The search bar's field: Rune's core `text_input`, frameless as Strata's `.search-field` is,
//! and the keys that drive the results.
//!
//! Not rune-kit's `TextInput`, which draws a box and builds its inner field itself, taking no
//! `on_secondary_press` for it (Rune draft U-C21-2): a right-click there asks for Rune's native
//! field menu, empty on Linux. The core field takes the press, and the palette sends it as
//! [`PaletteIntent::FieldMenu`] for C21's menu. C18's filter bar made the same choice; one
//! shared field is a follow-up once both have merged.

use rune::{Chord, ChordMod, El, Key, KeyEvent, PointerPress, Tx, text_input};
use strata_kit::look::Look;

use super::intent::PaletteIntent;
use super::mode::PaletteMode;
use super::state::{PaletteState, Polled};
use super::view::{Ctx, route};
use crate::contract::Point;

/// The field, editing `state.edit` unless nothing can be searched: a field with no edit
/// handler takes neither text nor focus, as Strata's insensitive field does, and the panel
/// keeps the keyboard (Escape still closes it).
pub(super) fn field<S: 'static>(
    cx: &Ctx<S>,
    look: &Look,
    state: &PaletteState,
    mode: &PaletteMode,
    editable: bool,
) -> El {
    let k = &look.kit;
    let access = cx.access;
    let mut field = text_input(&state.edit, k.ty.ui)
        .single_line()
        .key(super::field_key(&cx.key))
        .label(mode.field_label.clone())
        .placeholder(mode.placeholder.clone(), k.c.text_weak)
        .color(k.c.text_base)
        .caret(k.c.text_base, k.selection())
        // A query or a path isn't prose.
        .spellcheck(false)
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .read_only(!editable);
    if let Some(d) = &mode.description {
        field = field.description(d.clone());
    }
    if let Some(i) = state.highlighted().filter(|_| editable) {
        field = field.active_descendant(super::option_key(&cx.key, i));
    }
    if !editable {
        return field;
    }
    let (change, keys, menu) = (cx.emit.clone(), cx.emit.clone(), cx.emit.clone());
    let new_tab = cx.new_tab.clone();
    let reveal = cx.reveal.clone();
    let results = super::results_key(&cx.key);
    let key = cx.key.clone();
    field
        .on_edit(move |s: &mut S| &mut access(s).edit)
        .on_input_change(move |s: &mut S, tx: &mut Tx| {
            let polled = access(s).edited();
            route(&change, s, polled, tx);
        })
        .on_input_key(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| {
            let (polled, taken) = on_key(access(s), ev, tx, &new_tab, &reveal, &key, &results);
            route(&keys, s, polled, tx);
            taken
        })
        .on_secondary_press(move |s: &mut S, p: PointerPress, tx: &mut Tx| {
            let polled = Polled {
                changed: true,
                intents: vec![PaletteIntent::FieldMenu {
                    at: Point::new(p.x, p.y),
                }],
            };
            route(&menu, s, polled, tx);
        })
}

/// Whether `ev` is `chord`, or `chord` with `extra` held too.
fn matches_with(chord: &Chord, extra: ChordMod, ev: &KeyEvent, tx: &Tx) -> bool {
    chord.matches_tx(ev, tx) || (!chord.has(extra) && chord.with(extra).matches_tx(ev, tx))
}

/// A key in the field, before the editor sees it. True takes it. Rune hands Return and
/// Escape to an input-method composition first, so they never reach here while one runs.
pub(super) fn on_key(
    st: &mut PaletteState,
    ev: &KeyEvent,
    tx: &mut Tx,
    new_tab: &[Chord],
    reveal: &[Chord],
    key: &str,
    results: &str,
) -> (Polled, bool) {
    if st.edit.is_composing() {
        return (Polled::default(), false);
    }
    let m = ev.mods;
    // Strata's ESCAPE_ANY ignores the modifiers, so Shift+Escape closes too.
    if ev.key == Key::Escape {
        if tx.key_repeat() {
            return (Polled::default(), true);
        }
        return (st.escape(), true);
    }
    // Strata matches Alt+Enter with Hyper or Meta held too; Rune reports those as Super.
    if reveal
        .iter()
        .any(|c| matches_with(c, ChordMod::Super, ev, tx))
    {
        return (st.reveal_highlighted(tx), true);
    }
    // Strata's open_in_new_tab ignores Shift.
    if new_tab
        .iter()
        .any(|c| matches_with(c, ChordMod::Shift, ev, tx))
    {
        if tx.key_repeat() {
            return (Polled::default(), true);
        }
        return (st.open_highlighted(true, tx), true);
    }
    let plain = !(m.ctrl || m.alt || m.cmd || m.logo);
    match ev.key {
        // Enter and Shift+Enter (Strata's ENTER_ANY_SHIFT).
        Key::Enter if plain => {
            if tx.key_repeat() {
                return (Polled::default(), true);
            }
            (st.open_highlighted(false, tx), true)
        }
        Key::Up | Key::Down if plain && !m.shift => {
            let delta = if ev.key == Key::Up { -1 } else { 1 };
            let mut polled = Polled::default();
            if st.step(delta) {
                polled.changed = true;
                if let Some(i) = st.highlighted() {
                    st.mark_own_scroll();
                    tx.scroll_into_view_in(results, super::option_key(key, i));
                }
            }
            (polled, true)
        }
        _ => (Polled::default(), false),
    }
}
