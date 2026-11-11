//! The entry: a combobox around rune-kit's text input, with Navigate and Cancel, and the keys
//! that drive the suggestions.

use rune::{El, Key, KeyEvent, Tx, row};
use rune_kit::TextInput;
use strata_kit::icons::IconName;

use super::build::{Ctx, field_key, option_key, route};
use super::completion::Command;
use super::state::Polled;
use crate::header;

pub(super) fn view<S: 'static>(cx: &Ctx<'_, S>) -> El {
    let look = cx.look;
    let k = &look.kit;
    let state = cx.state;
    let access = cx.access;
    let open = state.suggestions_open();
    let active = state
        .highlighted()
        .filter(|_| open)
        .map(|i| option_key(cx.key, i));
    let (change_emit, key_emit) = (cx.emit.clone(), cx.emit.clone());
    let complete = cx.complete.to_vec();
    let field = TextInput::new(&state.edit)
        .width(None)
        .height(cx.row())
        .label(cx.labels.entry.clone())
        .placeholder(cx.labels.placeholder.clone())
        .active_descendant(active)
        .on_edit(move |s: &mut S| &mut access(s).edit)
        .on_change(move |s: &mut S, tx: &mut Tx| {
            let polled = access(s).edited();
            route(&change_emit, s, polled, tx);
        })
        .on_key(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| {
            let (polled, taken) = key(access(s), ev, tx, &complete);
            route(&key_emit, s, polled, tx);
            taken
        })
        .build(k);
    let button = |icon: IconName, label: &str, suffix: &str, cancel: bool| {
        let emit = cx.emit.clone();
        // The header's action look; its tooltip names it.
        header::action(look, icon, label)
            .on_press_tx(move |s: &mut S, tx: &mut Tx| {
                let st = access(s);
                let polled = if cancel {
                    st.cancel()
                } else {
                    let text = st.edit.text.clone();
                    st.submit(text)
                };
                route(&emit, s, polled, tx);
            })
            .build(k)
            .key(format!("{}-{suffix}", cx.key))
            // The entry keeps the keyboard, so pressing the button isn't a press outside it.
            // A focusable button would take the keyboard on a press, and nothing tells the bar
            // where it went (U-C6-3), so they are pointer-only: Enter and Escape do the same.
            .focusable(false)
            .preserve_focus_on_press(true)
    };
    // A text input is always a text box to Rune's accessibility (U-C6-1), so the combobox is
    // the group around it, expanded while the suggestions show.
    row()
        .key(format!("{}-entry", cx.key))
        .role("combobox")
        .label(cx.labels.entry.clone())
        .expanded(open)
        .w_full()
        .h(cx.height())
        .items_center()
        .gap(look.space.xxs)
        .child(
            row()
                .key(field_key(cx.key))
                .grow(1.0)
                .min_w(0.0)
                .child(field),
        )
        .child(button(
            IconName::Check,
            &cx.labels.navigate,
            "navigate",
            false,
        ))
        .child(button(IconName::Close, &cx.labels.cancel, "cancel", true))
}

/// A key in the entry, before the editor sees it. True takes it.
fn key(
    st: &mut super::LocationBarState,
    ev: &KeyEvent,
    tx: &mut Tx,
    complete: &[rune::Chord],
) -> (Polled, bool) {
    // Rune gives Escape and Enter to a composition first; the rest of the keys an input
    // method is using stay the editor's.
    if st.edit.is_composing() {
        return (Polled::default(), false);
    }
    let m = ev.mods;
    let plain = !(m.ctrl || m.alt || m.cmd || m.logo);
    if ev.key == Key::Escape {
        if tx.key_repeat() {
            return (Polled::default(), true);
        }
        // No `tx.blur()`: the entry leaves the tree, and Rune drops its focus with it.
        return (st.cancel(), true);
    }
    if complete.iter().any(|c| c.matches_tx(ev, tx)) {
        return (st.command(Command::Complete).0, true);
    }
    // Shift doesn't change these, as Strata's bindings ignore it.
    let command = match ev.key {
        Key::Enter if plain => Command::Activate,
        Key::Down if plain => Command::Down,
        Key::Up if plain => Command::Step(-1),
        Key::PageDown if plain => Command::Step(5),
        Key::PageUp if plain => Command::Step(-5),
        _ => return (Polled::default(), false),
    };
    if command == Command::Activate && tx.key_repeat() {
        return (Polled::default(), true);
    }
    let (mut polled, proceeds) = st.command(command);
    if !proceeds {
        return (polled, true);
    }
    match command {
        Command::Activate => {
            let text = st.edit.text.clone();
            let submitted = st.submit(text);
            polled.changed |= submitted.changed;
            polled.intents.extend(submitted.intents);
            (polled, true)
        }
        // Up and the paging keys with no list showing are the editor's.
        _ => (polled, false),
    }
}
