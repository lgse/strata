// SPDX-License-Identifier: MIT

//! The action editor's icon picker: a row of 8 common icons and a "More icons" popover with the
//! remaining 16 bundled Lucide icons, matching `src/ui/settings/actions/icon_chooser.rs`.

use rune::{Color, Cursor, El, Key, KeyEvent, Tx, col, icon, row};
use rune_kit::{Placement, popover};
use strata_core::i18n::tr;
use strata_kit::{icons::IconName, look::Look};

use super::editor::EditorSession;
use crate::{
    App, buttons,
    custom_actions::{ACTION_ICON_SLUGS, action_icon_glyph, is_known_action_icon},
};

pub(super) const MORE_ICONS_BTN_KEY: &str = "settings-action-editor-more-icons";

const QUICK_ICONS: [&str; 8] = [
    "play",
    "terminal",
    "file-code",
    "scissors",
    "copy",
    "file-archive",
    "image",
    "refresh",
];

pub(super) fn build_row(look: &Look, session: &EditorSession) -> El {
    let current = current_slug(session);
    let mut items: Vec<El> = QUICK_ICONS
        .iter()
        .map(|&slug| icon_choice_button(look, slug, current == slug))
        .collect();
    items.push(more_icons_button(look, current, session.more_icons_open));
    row()
        .key("settings-action-editor-icons")
        .role("group")
        .label(tr("Icon"))
        .gap(6.0)
        .items_center()
        .flex_wrap()
        .children(items)
}

pub(super) fn build_popover(look: &Look, session: &EditorSession, layer: &mut Vec<El>) {
    if !session.more_icons_open {
        return;
    }
    let current = current_slug(session);
    let rest: Vec<&'static str> = ACTION_ICON_SLUGS
        .iter()
        .copied()
        .filter(|slug| !QUICK_ICONS.contains(slug))
        .collect();
    let mut rows = Vec::new();
    for chunk in rest.chunks(4) {
        let cells: Vec<El> = chunk
            .iter()
            .map(|&slug| icon_choice_button(look, slug, current == slug))
            .collect();
        rows.push(row().gap(6.0).children(cells));
    }
    let body = col()
        .key("settings-action-editor-more-icons-popover")
        .role("dialog")
        .label(tr("More icons"))
        .padding(8.0)
        .gap(6.0)
        .focus_return(MORE_ICONS_BTN_KEY)
        .on_key(|app: &mut App, ev: &KeyEvent, tx: &mut Tx| {
            if ev.key == Key::Escape {
                close_popover(app, tx);
                true
            } else {
                false
            }
        })
        .modal_tx(true, close_popover)
        .children(rows);
    let surface = popover::raised(&look.kit, body, look.round(8.0));
    layer.push(popover::anchored(
        &look.kit,
        surface,
        MORE_ICONS_BTN_KEY,
        Placement::Bottom,
        look.space.xs,
    ));
}

fn current_slug(session: &EditorSession) -> &str {
    session
        .selected_icon
        .as_deref()
        .filter(|slug| is_known_action_icon(slug))
        .unwrap_or("play")
}

fn icon_choice_button(look: &Look, slug: &'static str, selected: bool) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let (fill, border, ink) = choice_palette(look, selected);
    let name = action_icon_name(slug);
    let accessible = rust_i18n::t!("%{icon} icon", icon = name).into_owned();
    let btn = row()
        .role("button")
        .label(accessible)
        .checked(selected)
        .size(34.0, 34.0)
        .center()
        .radius(look.round(6.0))
        .fill(fill)
        .border(1.0, border)
        .hover(look.fill.highlight)
        .press(kit.alpha(c.bg_accent, 0.28))
        .focusable(true)
        .focus_ring(2.0, -2.0, c.border_focus)
        .cursor(Cursor::Pointer)
        .help(name)
        .child(
            icon(action_icon_glyph(look, Some(slug)), 18.0)
                .color(ink)
                .a11y_hidden(),
        )
        .on_press_tx(move |app: &mut App, tx: &mut Tx| {
            let was_open = if let Some(editor) = app.settings_dialog.actions.editor.as_mut() {
                editor.selected_icon = Some(slug.to_owned());
                std::mem::replace(&mut editor.more_icons_open, false)
            } else {
                false
            };
            if was_open {
                tx.focus(MORE_ICONS_BTN_KEY, true);
            }
        });
    buttons::on_return(format!("settings-action-icon-{slug}"), btn)
}

fn more_icons_button(look: &Look, current: &str, open: bool) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let from_more = !QUICK_ICONS.contains(&current);
    let (fill, border, ink) = choice_palette(look, from_more || open);
    let glyph = if from_more {
        action_icon_glyph(look, Some(current))
    } else {
        look.icon(IconName::Plus)
    };
    let tooltip = if from_more {
        action_icon_name(current)
    } else {
        tr("More icons")
    };
    let btn = row()
        .role("button")
        .label(tr("More icons"))
        .description(tr("Choose another bundled Lucide icon"))
        .checked(open)
        .size(34.0, 34.0)
        .center()
        .radius(look.round(6.0))
        .fill(fill)
        .border(1.0, border)
        .hover(look.fill.highlight)
        .press(kit.alpha(c.bg_accent, 0.28))
        .focusable(true)
        .focus_ring(2.0, -2.0, c.border_focus)
        .cursor(Cursor::Pointer)
        .help(tooltip)
        .child(icon(glyph, 18.0).color(ink).a11y_hidden())
        .on_press_tx(|app: &mut App, tx: &mut Tx| {
            let Some(editor) = app.settings_dialog.actions.editor.as_mut() else {
                return;
            };
            editor.more_icons_open = !editor.more_icons_open;
            if editor.more_icons_open
                && let Some(first) = ACTION_ICON_SLUGS
                    .iter()
                    .copied()
                    .find(|slug| !QUICK_ICONS.contains(slug))
            {
                tx.focus_when_shown(
                    format!("settings-action-icon-{first}"),
                    tx.focus_visible() || tx.by_keys(),
                );
            }
        });
    buttons::on_return(MORE_ICONS_BTN_KEY, btn)
}

fn choice_palette(look: &Look, selected: bool) -> (Color, Color, Color) {
    let kit = &look.kit;
    let c = &kit.c;
    if selected {
        (
            kit.alpha(c.bg_accent, 0.22),
            kit.alpha(c.bg_accent, 0.70),
            c.icon_accent,
        )
    } else {
        (
            kit.alpha(c.bg_base, 0.25),
            kit.alpha(c.std_border, c.std_border.a * 0.40),
            c.icon_accent,
        )
    }
}

fn close_popover(app: &mut App, tx: &mut Tx) {
    if let Some(editor) = app.settings_dialog.actions.editor.as_mut() {
        editor.more_icons_open = false;
    }
    tx.focus(MORE_ICONS_BTN_KEY, true);
}

/// Semantic keys; a missing entry falls back to the slug instead of the key.
pub(super) fn action_icon_name(slug: &str) -> String {
    let key = format!("action_icon.{slug}");
    let name = tr(&key);
    if name == key { slug.to_owned() } else { name }
}
