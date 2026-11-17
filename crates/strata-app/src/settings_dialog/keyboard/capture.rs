// SPDX-License-Identifier: MIT

//! Recording a new key for a row: GTK's `start_capture`, `end_capture` and `captured`
//! (`src/ui/settings/keyboard.rs`) over `keys::capture`. While a capture is armed its button
//! has the keyboard and takes every press but a lone modifier, as pressed and before the
//! aliases (`commands::read_pressed`), ahead of the dialog's own keys. Escape ends it,
//! Backspace unbinds the row, and a key the checker refuses keeps it waiting with the reason.

use rune::{El, KeyEvent, Tx};
use rune_kit::{Button, ButtonVariant};
use strata_core::i18n::tr;
use strata_desktop::keys::{
    Active, Binding, Command, Key, KeyPress, Map,
    bindings::BACKSPACE,
    conflicts::{self, Severity},
};

use super::{
    messages::{key_label, message},
    policy,
    rows::RowKeys,
};
use crate::{App, buttons, commands, shell};

pub(super) struct Capturing {
    map: Map,
    command: Command,
    /// What the checker said of the last key it refused, and that key.
    refused: Vec<String>,
    pressed: Option<Binding>,
    /// Whether the capture's button has had the keyboard, after which losing it ends the
    /// capture.
    held: bool,
}

impl Capturing {
    fn is(&self, map: Map, command: Command) -> bool {
        self.map == map && self.command == command
    }
}

/// What the capture says of the last key it refused on this row.
pub(super) fn refusals(app: &App, map: Map, command: Command) -> &[String] {
    match &app.settings_dialog.keyboard.capture {
        Some(capturing) if capturing.is(map, command) => &capturing.refused,
        _ => &[],
    }
}

/// The row's Add button, or while it captures, the capture's button and Cancel.
pub(super) fn controls(
    app: &App,
    (map, command): (Map, Command),
    keys: &RowKeys,
    action: &str,
) -> Vec<El> {
    let kit = &app.look.kit;
    let capturing = app.settings_dialog.keyboard.capture.as_ref();
    let Some(capturing) = capturing.filter(|capturing| capturing.is(map, command)) else {
        let add = Button::new(tr("Add"))
            .variant(ButtonVariant::Neutral)
            .on_press_tx(move |app: &mut App, tx: &mut Tx| start(app, (map, command), tx))
            .build(kit)
            .label(rust_i18n::t!("Add a key for “%{action}”", action = action));
        return vec![buttons::on_return(keys.add.clone(), add)];
    };
    let shown = capturing
        .pressed
        .map_or_else(|| tr("Press a key…"), key_label);
    let mut description = capturing.refused.join(" ");
    if !description.is_empty() {
        description.push(' ');
    }
    description.push_str(&help());
    let capture = Button::new(shown)
        .variant(ButtonVariant::Neutral)
        .on_press(|_: &mut App| {})
        .build(kit)
        .label(rust_i18n::t!(
            "Press the new key for “%{action}”",
            action = action
        ))
        .description(description)
        .on_key(on_key);
    let add = keys.add.clone();
    let cancel = Button::new(tr("Cancel"))
        .variant(ButtonVariant::Neutral)
        .on_press_tx(move |app: &mut App, tx: &mut Tx| {
            app.settings_dialog.keyboard.end_capture();
            tx.focus(add.clone(), true);
        })
        .build(kit);
    vec![
        buttons::taking_return(keys.capture.clone(), capture),
        buttons::on_return(keys.cancel.clone(), cancel),
    ]
}

fn help() -> String {
    rust_i18n::t!(
        "%{escape} cancels, and %{backspace} removes every key of this action.",
        escape = key_label(Binding::plain(Key::Escape)),
        backspace = key_label(BACKSPACE)
    )
    .into_owned()
}

fn start(app: &mut App, (map, command): (Map, Command), tx: &mut Tx) {
    app.settings_dialog.keyboard.capture = Some(Capturing {
        map,
        command,
        refused: Vec::new(),
        pressed: None,
        held: false,
    });
    let visible = tx.focus_visible() || tx.by_keys();
    tx.focus_when_shown(RowKeys::of(map, command).capture, visible);
}

/// Ends the capture and gives the row's Add button the keyboard.
fn end_to_add(app: &mut App, tx: &mut Tx) {
    if let Some(capturing) = app.settings_dialog.keyboard.capture.take() {
        tx.focus(RowKeys::of(capturing.map, capturing.command).add, true);
    }
}

fn on_key(app: &mut App, event: &KeyEvent, tx: &mut Tx) -> bool {
    let press = commands::read_pressed(app, event, tx);
    if press.is_modifier() {
        return false;
    }
    captured(app, &press, tx);
    true
}

fn captured(app: &mut App, press: &KeyPress, tx: &mut Tx) {
    let Some(binding) = press.binding() else {
        return;
    };
    let Some((map, command)) = app
        .settings_dialog
        .keyboard
        .capture
        .as_ref()
        .map(|capturing| (capturing.map, capturing.command))
    else {
        return;
    };
    if binding.key() == Key::Escape {
        end_to_add(app, tx);
        return;
    }
    if binding == BACKSPACE {
        end_to_add(app, tx);
        shell::saved(app).change(|s| s.keymap.set_bindings(map, command, Vec::new()));
        return;
    }
    let keymap = shell::live_keys(app).keymap();
    let mut entry = keymap.bindings(map, command).to_vec();
    // The press also matches its digit-row position where the layout types a symbol there,
    // so a digit that is taken refuses it too.
    let mut matched = vec![binding];
    matched.extend(
        press
            .digit_position()
            .filter(|position| *position != binding),
    );
    if matched.iter().any(|binding| entry.contains(binding)) {
        end_to_add(app, tx);
        return;
    }
    let policy = policy(app);
    let refused: Vec<String> = matched
        .iter()
        .flat_map(|binding| conflicts::check(&keymap, map, command, *binding, policy))
        .filter(|finding| finding.severity == Severity::Error)
        .map(|finding| message(map, command, finding))
        .collect();
    if refused.is_empty() {
        end_to_add(app, tx);
        entry.push(binding);
        shell::saved(app).change(|s| s.keymap.set_bindings(map, command, entry));
        return;
    }
    if let Some(capturing) = app.settings_dialog.keyboard.capture.as_mut() {
        capturing.refused = refused;
        capturing.pressed = Some(binding);
    }
}

/// Ends the capture once its button, having had the keyboard, loses it to anything but
/// Cancel: GTK's `focus-widget` watch.
pub(super) fn follow_focus(app: &mut App, tx: &Tx) -> bool {
    let state = &mut app.settings_dialog.keyboard;
    let Some(capturing) = state.capture.as_mut() else {
        return false;
    };
    let keys = RowKeys::of(capturing.map, capturing.command);
    let focused = tx.focused();
    if focused == Some(keys.capture.as_str()) {
        capturing.held = true;
        return false;
    }
    if capturing.held && focused != Some(keys.cancel.as_str()) {
        state.capture = None;
        return true;
    }
    false
}
