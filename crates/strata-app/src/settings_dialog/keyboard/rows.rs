// SPDX-License-Identifier: MIT

//! One command's row: its name, the notes on its keys, a chip per key with its remove
//! button, and Add, the capture and Reset. GTK's `Row::new`, `Row::render` and `Row::chip`
//! (`src/ui/settings/keyboard.rs`).

use std::collections::HashMap;

use rune::{El, Tx, col, row, text};
use rune_kit::{Button, ButtonVariant, IconButton, IconButtonVariant};
use strata_core::i18n::tr;
use strata_desktop::keys::{
    Active, Binding, Command, CommandKind, Key, Map,
    bindings::{ENTER, ESCAPE, FOCUS_NEXT, FOCUS_PREVIOUS},
    conflicts::{self, Finding, Kind, Severity},
};
use strata_kit::{icons::IconName, look::Look};

use super::{
    Page, capture,
    messages::{key_label, message},
    names::{slot_action, title},
    sections::trigger,
};
use crate::{App, buttons, shell};

/// Every finding of `map`'s table, by command.
pub(super) fn audit(page: &Page<'_>, map: Map) -> HashMap<Command, Vec<Finding>> {
    let mut found: HashMap<Command, Vec<Finding>> = HashMap::new();
    for (command, finding) in conflicts::audit(&page.keymap, map, page.policy) {
        found.entry(command).or_default().push(finding);
    }
    found
}

/// The element keys of one row's controls.
pub(super) struct RowKeys {
    pub(super) add: String,
    pub(super) capture: String,
    pub(super) cancel: String,
    base: String,
}

impl RowKeys {
    pub(super) fn of(map: Map, command: Command) -> Self {
        let tag = match map {
            Map::Default => "default",
            Map::Tenxer => "tenxer",
        };
        let base = format!("settings-keymap-{tag}-{}", command.id());
        Self {
            add: format!("{base}-add"),
            capture: format!("{base}-capture"),
            cancel: format!("{base}-cancel"),
            base,
        }
    }
}

/// A key on the row, and what the checker says of it there.
struct Chip {
    binding: Binding,
    inactive: bool,
    notes: Vec<String>,
}

/// What `Row::render` works out before it draws.
struct Rendered {
    prefix: Option<Binding>,
    chips: Vec<Chip>,
    notes: Vec<(String, Severity)>,
    turn_off_typing: bool,
}

pub(super) fn command_row(page: &Page<'_>, map: Map, command: Command, found: &[Finding]) -> El {
    let look = &page.app.look;
    let keys = RowKeys::of(map, command);
    let rendered = render(page, map, command, found);
    let copy = copy_column(page, (map, command), &rendered);
    let controls = controls_row(page, (map, command), &keys, &rendered);
    row()
        .key(keys.base.clone())
        .w_full()
        .min_w(0.0)
        .px(18.0)
        .py(16.0)
        .gap(look.space.xl)
        .items_center()
        .child(copy)
        .child(controls)
}

fn render(page: &Page<'_>, map: Map, command: Command, found: &[Finding]) -> Rendered {
    let entry = page.keymap.bindings(map, command);
    let trigger = match (map, command.kind()) {
        (Map::Tenxer, CommandKind::Step(group)) => trigger(group),
        _ => None,
    };
    let prefix = trigger.and_then(|t| page.keymap.bindings(Map::Tenxer, t).first().copied());
    let unreachable = trigger.filter(|_| prefix.is_none());
    let mut rendered = Rendered {
        prefix,
        chips: Vec::new(),
        notes: Vec::new(),
        turn_off_typing: false,
    };
    if let Some(trigger) = unreachable.filter(|_| !entry.is_empty()) {
        let note = rust_i18n::t!(
            "“%{group}” has no key, so this action cannot run.",
            group = title(trigger)
        );
        rendered.notes.push((note.into_owned(), Severity::Warning));
    }
    for binding in entry {
        let chip = chip_of((map, command), *binding, found, &mut rendered);
        let inactive = chip.inactive || unreachable.is_some();
        rendered.chips.push(Chip { inactive, ..chip });
    }
    if command == ESCAPE && !entry.contains(&Binding::plain(Key::Escape)) {
        let note = rust_i18n::t!(
            "%{keys} always works here as well.",
            keys = key_label(Binding::plain(Key::Escape))
        );
        push_note(&mut rendered.notes, note.into_owned(), Severity::Warning);
    }
    rendered
}

/// A key's chip, its findings added to the row's notes.
fn chip_of(
    (map, command): (Map, Command),
    binding: Binding,
    found: &[Finding],
    rendered: &mut Rendered,
) -> Chip {
    let mut own: Vec<Finding> = found
        .iter()
        .filter(|finding| finding.binding == binding.normalized())
        .copied()
        .collect();
    // A key that never fires has nothing else worth saying.
    if own.iter().any(|finding| finding.kind == Kind::NeverMatches) {
        own.retain(|finding| finding.kind == Kind::NeverMatches);
    }
    let notes: Vec<String> = own.iter().map(|f| message(map, command, *f)).collect();
    for (finding, text) in own.iter().zip(&notes) {
        rendered.turn_off_typing |= finding.kind == Kind::TypeToSearchShadowed;
        push_note(&mut rendered.notes, text.clone(), finding.severity);
    }
    if multiline_limit(command, binding) {
        let note = rust_i18n::t!(
            "In text boxes with several lines, %{keys} does not start a new line or insert a tab.",
            keys = key_label(binding)
        );
        push_note(&mut rendered.notes, note.into_owned(), Severity::Warning);
    }
    Chip {
        binding,
        inactive: own.iter().any(|finding| finding.inactive()),
        notes,
    }
}

/// A key other than Enter or Tab that a multi-line text box reads as them.
fn multiline_limit(command: Command, binding: Binding) -> bool {
    [ENTER, FOCUS_NEXT, FOCUS_PREVIOUS].contains(&command) && command.canonical() != Some(binding)
}

fn push_note(notes: &mut Vec<(String, Severity)>, text: String, severity: Severity) {
    if !notes.iter().any(|(listed, _)| *listed == text) {
        notes.push((text, severity));
    }
}

/// The row's name, a slot's action, the notes on its keys and a capture's refusals.
fn copy_column(page: &Page<'_>, (map, command): (Map, Command), rendered: &Rendered) -> El {
    let look = &page.app.look;
    let c = &look.kit.c;
    let mut copy = col().grow(1.0).min_w(0.0).gap(look.space.xxs).child(
        text(title(command))
            .font(look.kit.ty.ui_medium)
            .color(c.text_base)
            .wrap(),
    );
    if let Some(action) = slot_action(command, page.catalog.as_deref()) {
        copy = copy.child(
            text(action)
                .font(look.kit.ty.small)
                .color(c.text_muted)
                .wrap(),
        );
    }
    for (note, severity) in &rendered.notes {
        copy = copy.child(finding_label(look, note, *severity));
    }
    for refusal in capture::refusals(page.app, map, command) {
        copy = copy.child(finding_label(look, refusal, Severity::Error));
    }
    if rendered.turn_off_typing && shell::preferences(page.app).type_to_search {
        copy = copy.child(turn_off_typing(look, &RowKeys::of(map, command).base));
    }
    copy
}

pub(super) fn finding_label(look: &Look, note: &str, severity: Severity) -> El {
    let c = &look.kit.c;
    let color = match severity {
        Severity::Error => c.danger.fg,
        Severity::Warning => c.warning.fg,
    };
    text(note.to_owned())
        .font(look.kit.ty.small)
        .color(color)
        .wrap()
}

fn turn_off_typing(look: &Look, base: &str) -> El {
    let button = Button::new(tr("Turn off type-to-search"))
        .variant(ButtonVariant::Neutral)
        .on_press_tx(|app: &mut App, _: &mut Tx| {
            shell::saved(app).change(|s| s.type_to_search = false);
        })
        .build(&look.kit);
    row()
        .pt(look.space.xxs)
        .child(buttons::on_return(format!("{base}-typing"), button))
}

/// The 10xer chord's key before a step, the chips, "Not bound", then Add or the capture and
/// Cancel, and Reset.
fn controls_row(
    page: &Page<'_>,
    (map, command): (Map, Command),
    keys: &RowKeys,
    rendered: &Rendered,
) -> El {
    let look = &page.app.look;
    let c = &look.kit.c;
    let action = title(command);
    let mut controls = row().gap(look.space.xs).items_center();
    if let Some(prefix) = rendered.prefix {
        controls = controls.child(
            text(key_label(prefix))
                .font(look.kit.ty.mono)
                .color(c.text_muted),
        );
    }
    for (index, chip) in rendered.chips.iter().enumerate() {
        controls = controls.child(chip_el(look, (map, command), &action, (index, chip)));
    }
    if rendered.chips.is_empty() {
        controls = controls.child(
            text(tr("Not bound"))
                .font(look.kit.ty.small)
                .color(c.text_muted),
        );
    }
    controls = controls.children(capture::controls(page.app, (map, command), keys, &action));
    if page.keymap.overrides(map, command) {
        controls = controls.child(reset_button(look, (map, command), keys, &action));
    }
    controls
}

fn chip_el(
    look: &Look,
    (map, command): (Map, Command),
    action: &str,
    (index, chip): (usize, &Chip),
) -> El {
    let kit = &look.kit;
    let label = key_label(chip.binding);
    let binding = chip.binding;
    let keys = RowKeys::of(map, command);
    let key = format!("{}-chip-{index}", keys.base);
    let add = keys.add;
    let mut remove = IconButton::new(look.icon(IconName::Close))
        .variant(IconButtonVariant::Ghost)
        .label(rust_i18n::t!(
            "Remove %{keys} from “%{action}”",
            keys = label.as_str(),
            action = action
        ))
        .on_press_tx(move |app: &mut App, tx: &mut Tx| {
            remove_key(app, (map, command), binding);
            tx.focus(add.clone(), true);
        })
        .build(kit);
    if !chip.notes.is_empty() {
        remove = remove.description(chip.notes.join(" "));
    }
    let ink = if chip.inactive {
        kit.c.text_muted
    } else {
        kit.c.text_base
    };
    row()
        .gap(look.space.xxs)
        .items_center()
        .pl(look.space.sm)
        .radius(kit.radius.sm)
        .fill(look.fill.muted)
        .border(1.0, kit.c.border_strong)
        .child(text(label).font(kit.ty.mono).color(ink))
        .child(buttons::on_return(key, remove))
}

fn remove_key(app: &mut App, (map, command): (Map, Command), binding: Binding) {
    app.settings_dialog.keyboard.end_capture();
    let keymap = shell::live_keys(app).keymap();
    let kept: Vec<Binding> = keymap
        .bindings(map, command)
        .iter()
        .copied()
        .filter(|kept| *kept != binding)
        .collect();
    shell::saved(app).change(|s| s.keymap.set_bindings(map, command, kept));
}

fn reset_button(look: &Look, (map, command): (Map, Command), keys: &RowKeys, action: &str) -> El {
    let add = keys.add.clone();
    let button = Button::new(tr("Reset"))
        .variant(ButtonVariant::Neutral)
        .on_press_tx(move |app: &mut App, tx: &mut Tx| {
            app.settings_dialog.keyboard.end_capture();
            shell::saved(app).change(|s| s.keymap.reset_binding(map, command));
            tx.focus(add.clone(), true);
        })
        .build(&look.kit)
        .label(rust_i18n::t!(
            "Restore the default keys of “%{action}”",
            action = action
        ));
    buttons::on_return(format!("{}-reset", keys.base), button)
}
