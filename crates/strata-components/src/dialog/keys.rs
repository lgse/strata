//! What the dialog does with a key, as plain decisions the panel's key handler acts on.

use rune::{Key, KeyEvent, Mods, TextEdit, Tx};
use rune_kit::TextArea;

/// What the panel's key handler does with a key focus inside the dialog passed on. Rune runs
/// that handler before its default button and its own focus navigation, and never while a
/// text field has focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Route {
    /// Leave the key to Rune: the default button, focus navigation, the window.
    Pass,
    /// Press the focused control: Enter does what Space does.
    Press,
    /// Move focus to the footer button at this index.
    Focus(usize),
    /// Take the key and do nothing.
    Consume,
}

/// How the dialog routes `ev`, with `focused` the key of the focused element (`Tx::focused`),
/// `panel` the panel's own key and `footer` the footer buttons that can take focus, in
/// document order.
pub(crate) fn route(
    ev: &KeyEvent,
    repeat: bool,
    focused: Option<&str>,
    panel: &str,
    footer: &[String],
) -> Route {
    if ev.mods != Mods::default() {
        return Route::Pass;
    }
    match ev.key {
        Key::Enter => match focused {
            // Nothing in the dialog has focus: Return goes to the primary, the default button.
            None => Route::Pass,
            Some(f) if f == panel => Route::Pass,
            // A held Return presses once.
            Some(_) if repeat => Route::Consume,
            Some(_) => Route::Press,
        },
        Key::Left | Key::Right => {
            let Some(i) = focused.and_then(|f| footer.iter().position(|k| k == f)) else {
                return Route::Pass;
            };
            match ev.key {
                Key::Left if i == 0 => Route::Consume,
                Key::Left => Route::Focus(i - 1),
                _ if i + 1 == footer.len() => Route::Consume,
                _ => Route::Focus(i + 1),
            }
        }
        _ => Route::Pass,
    }
}

/// A multi-line field's key handler that types a newline on a plain Return:
/// `TextArea::on_key(newline_on_enter())`. Prefer [`text_area`], which wires it.
///
/// Rune's multi-line editor treats a plain Return in a paragraph as submit, as a chat
/// composer does, and a field with nothing to submit passes it to the default button: the
/// dialog's primary. The handler sends Shift+Return instead, the editor's own newline, so the
/// edit goes through the field as typing does: its `on_change` (a length limit, say), the
/// caret's scroll and the edited mark. Rune applies the sent key on the next frame.
///
/// A rune-kit field keeps one key handler: one set after this replaces it.
pub fn newline_on_enter<S: 'static>() -> impl Fn(&mut S, &KeyEvent, &mut Tx) -> bool + 'static {
    |_s, ev, tx| {
        if ev.key != Key::Enter || ev.mods != Mods::default() {
            return false;
        }
        tx.send_key(KeyEvent {
            key: Key::Enter,
            mods: Mods {
                shift: true,
                ..Mods::default()
            },
        });
        true
    }
}

/// A multi-line field for a dialog, editing the `TextEdit` `access` reaches, with Return
/// typing a newline ([`newline_on_enter`]) instead of pressing the dialog's primary. Use it
/// for every multi-line field in a dialog; add a label, an `on_change` and the rest as on any
/// `TextArea`, but no other `on_key`.
pub fn text_area<S: 'static>(
    edit: &TextEdit,
    access: impl for<'a> Fn(&'a mut S) -> &'a mut TextEdit + 'static,
) -> TextArea {
    TextArea::new(edit)
        .on_edit(access)
        .on_key(newline_on_enter::<S>())
}
