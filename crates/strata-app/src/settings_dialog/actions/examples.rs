// SPDX-License-Identifier: MIT

use rune::{Color, Cursor, El, Key, KeyEvent, ScrollSpec, TextEdit, Tx, col, icon, row, text};
use rune_kit::{IconButton, IconButtonVariant, Placement, TextInput, popover};
use strata_components::scroll::Bar;
use strata_core::{
    i18n::tr,
    model::action::examples::{ACTION_EXAMPLES, ActionExample},
};
use strata_kit::{icons::IconName, look::Look};

use super::editor::EditorSession;
use crate::{
    App, buttons,
    settings_dialog::controls::{
        SCROLLBAR_EDGE_INSET, SCROLLBAR_END_INSET, bounded_scroll_with_bar, settings_bar_look,
    },
};

pub(super) const EXAMPLES_BTN_KEY: &str = "settings-action-editor-library";
pub(super) const EXAMPLES_SEARCH_KEY: &str = "settings-action-library-search";
const EXAMPLES_CLEAR_KEY: &str = "settings-action-library-clear";

pub(super) fn library_button(look: &Look, open: bool) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let glyph = look
        .icon_named("strata-library")
        .unwrap_or_else(|| look.icon(IconName::FileCode));
    let (fill, border, ink) = if open {
        (kit.alpha(c.bg_accent, 0.22), c.border_active, c.text_accent)
    } else {
        (
            Color::TRANSPARENT,
            kit.alpha(c.std_border, c.std_border.a * 0.54),
            c.text_base,
        )
    };
    let btn = row()
        .role("button")
        .label(tr("Library"))
        .description(tr("Choose a bundled script template"))
        .checked(open)
        .min_h(26.0)
        .px(10.0)
        .py(4.0)
        .gap(6.0)
        .items_center()
        .radius(look.round(5.0))
        .fill(fill)
        .border(1.0, border)
        .hover(kit.alpha(c.bg_accent, 0.15))
        .press(kit.alpha(c.bg_accent, 0.22))
        .focusable(true)
        .focus_ring(2.0, 1.0, kit.alpha(c.border_focus, 0.50))
        .cursor(Cursor::Pointer)
        .child(icon(glyph, 15.0).color(c.icon_accent).a11y_hidden())
        .child(text(tr("Library")).font(kit.ty.ui).color(ink).a11y_hidden())
        .on_press_tx(|app: &mut App, tx: &mut Tx| toggle_popover(app, tx));
    buttons::on_return(EXAMPLES_BTN_KEY, btn)
}

fn toggle_popover(app: &mut App, tx: &mut Tx) {
    let Some(editor) = app.settings_dialog.actions.editor.as_mut() else {
        return;
    };
    editor.examples_open = !editor.examples_open;
    editor.pending_example = None;
    if editor.examples_open {
        let len = editor.examples_query.text.len();
        editor.examples_query.set_selection(0, len);
        tx.focus_when_shown(EXAMPLES_SEARCH_KEY, tx.focus_visible() || tx.by_keys());
    } else {
        editor.examples_bar.release();
    }
}

pub(super) fn close_popover(app: &mut App, tx: &mut Tx) {
    if let Some(editor) = app.settings_dialog.actions.editor.as_mut() {
        editor.examples_open = false;
        editor.examples_bar.release();
        editor.pending_example = None;
        editor.examples_query.focused = false;
    }
    tx.focus(EXAMPLES_BTN_KEY, true);
}

pub(super) fn build_popover(look: &Look, session: &EditorSession, layer: &mut Vec<El>) {
    if !session.examples_open {
        return;
    }
    let kit = &look.kit;
    let c = &kit.c;
    let divider = kit.alpha(c.std_border, c.std_border.a * 0.35);
    let query = session.examples_query.text.to_lowercase();
    let visible: Vec<&'static ActionExample> = ACTION_EXAMPLES
        .iter()
        .filter(|example| matches(example, &query, session.examples_category))
        .collect();
    let first_match = visible.first().copied();
    let mut body = col()
        .key("settings-action-library-popover")
        .role("dialog")
        .label(tr("Script library"))
        .w(440.0)
        .focus_return(EXAMPLES_BTN_KEY)
        .on_key(move |app: &mut App, ev: &KeyEvent, tx: &mut Tx| {
            popover_key(app, ev, tx, first_match)
        })
        .modal_tx(true, close_popover)
        .child(search_bar(look, &session.examples_query, divider))
        .child(category_bar(look, session.examples_category, divider))
        .child(template_list(look, &session.examples_bar, &visible));
    if let Some(pending) = session.pending_example {
        body = body.child(replace_confirmation(look, pending, divider));
    }
    let surface = strata_kit::transparency::raised_popover(look, body, look.round(9.0));
    layer.push(popover::anchored(
        kit,
        surface,
        EXAMPLES_BTN_KEY,
        Placement::Bottom,
        look.space.xs,
    ));
}

fn popover_key(
    app: &mut App,
    ev: &KeyEvent,
    tx: &mut Tx,
    first_match: Option<&'static ActionExample>,
) -> bool {
    match ev.key {
        Key::Escape => {
            close_popover(app, tx);
            true
        }
        Key::Enter if tx.focused() == Some(EXAMPLES_SEARCH_KEY) => {
            if let Some(example) = first_match {
                pick_example(app, example, tx);
            }
            true
        }
        Key::Down if tx.focused() == Some(EXAMPLES_SEARCH_KEY) => {
            if let Some(example) = first_match {
                tx.focus(example_key(example), true);
            }
            true
        }
        _ => false,
    }
}

fn search_bar(look: &Look, edit: &TextEdit, divider: Color) -> El {
    let kit = &look.kit;
    let label = tr("Search templates");
    let input = TextInput::new(edit)
        .label(label.clone())
        .placeholder(label)
        .on_edit(|app: &mut App| {
            &mut app
                .settings_dialog
                .actions
                .editor
                .as_mut()
                .expect("editor open")
                .examples_query
        })
        .on_change(|app: &mut App, _| {
            if let Some(editor) = app.settings_dialog.actions.editor.as_mut() {
                editor.pending_example = None;
            }
        })
        .build(kit)
        .key(EXAMPLES_SEARCH_KEY)
        .grow(1.0);
    let mut bar = row()
        .w_full()
        .padding(6.0)
        .gap(look.space.xs)
        .items_center()
        .border_sides([0.0, 0.0, 1.0, 0.0], divider)
        .child(input);
    if !edit.text.is_empty() {
        let clear = IconButton::new(look.icon(IconName::Close))
            .variant(IconButtonVariant::Ghost)
            .label(tr("Clear search"))
            .on_press_tx(|app: &mut App, tx: &mut Tx| {
                if let Some(editor) = app.settings_dialog.actions.editor.as_mut() {
                    editor.examples_query.replace_text("");
                    editor.pending_example = None;
                }
                tx.focus(EXAMPLES_SEARCH_KEY, true);
            })
            .build(kit);
        bar = bar.child(buttons::on_return(EXAMPLES_CLEAR_KEY, clear));
    }
    bar
}

fn category_bar(look: &Look, active: &'static str, divider: Color) -> El {
    let mut categories = vec!["All"];
    for example in ACTION_EXAMPLES {
        if !categories.contains(&example.category) {
            categories.push(example.category);
        }
    }
    let chips: Vec<El> = categories
        .into_iter()
        .map(|name| category_chip(look, name, name == active))
        .collect();
    row()
        .w_full()
        .px(12.0)
        .py(10.0)
        .gap(6.0)
        .flex_wrap()
        .border_sides([0.0, 0.0, 1.0, 0.0], divider)
        .children(chips)
}

fn category_chip(look: &Look, name: &'static str, active: bool) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let (fill, border, ink) = if active {
        (kit.alpha(c.bg_accent, 0.20), c.border_active, c.text_base)
    } else {
        (
            Color::TRANSPARENT,
            kit.alpha(c.std_border, c.std_border.a * 0.40),
            c.text_muted,
        )
    };
    let label = tr(name);
    let radius = look.round(14.0).min(11.0);
    let chip = row()
        .role("button")
        .label(label.clone())
        .checked(active)
        .min_h(22.0)
        .px(9.0)
        .center()
        .radius(radius)
        .fill(fill)
        .border(1.0, border)
        .hover(look.fill.highlight)
        .focusable(true)
        .focus_ring(2.0, -2.0, c.border_focus)
        .cursor(Cursor::Pointer)
        .child(text(label).font(kit.ty.small).color(ink).a11y_hidden())
        .on_press(move |app: &mut App| {
            if let Some(editor) = app.settings_dialog.actions.editor.as_mut() {
                editor.examples_category = name;
                editor.pending_example = None;
            }
        });
    buttons::on_return(format!("settings-action-library-cat-{name}"), chip)
}

fn template_list(look: &Look, bar: &Bar, visible: &[&'static ActionExample]) -> El {
    let mut inner = col()
        .w_full()
        .min_w(0.0)
        .pl(6.0)
        .pr(12.0)
        .pt(6.0)
        .pb(10.0)
        .gap(4.0);
    if visible.is_empty() {
        inner = inner.child(
            text(tr("No templates match your search."))
                .font(look.kit.ty.small)
                .color(look.kit.c.text_muted)
                .padding(8.0)
                .wrap(),
        );
    } else {
        for &example in visible {
            inner = inner.child(template_row(look, example));
        }
    }
    let viewport = col()
        .key("settings-action-library-list")
        .w_full()
        .max_h(270.0)
        .shrink()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .on_scroll_offset(bar.watch())
        .child(inner);
    bounded_scroll_with_bar(
        viewport,
        bar,
        (
            "settings-action-library-scrollbar",
            "settings-action-library-list",
        ),
        settings_bar_look(look, SCROLLBAR_EDGE_INSET, SCROLLBAR_END_INSET),
    )
}

fn template_row(look: &Look, example: &'static ActionExample) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let name = tr(example.name);
    let inputs = tr(example.inputs);
    let requirement_text = tr(example.requirements);
    let req_line = rust_i18n::t!("Requires: %{value1}", value1 = requirement_text).into_owned();
    let note = |copy: String| {
        text(copy)
            .font(kit.ty.small)
            .color(c.text_muted)
            .wrap()
            .a11y_hidden()
    };
    let content = col()
        .w_full()
        .gap(4.0)
        .child(template_row_heading(look, &name, example.runtime().label()))
        .child(note(tr(example.description)))
        .child(note(inputs.clone()))
        .child(note(req_line));
    let btn = col()
        .role("button")
        .label(name)
        .description(format!("{requirement_text} · {inputs}"))
        .w_full()
        .px(8.0)
        .py(10.0)
        .radius(look.round(6.0))
        .hover(c.hover)
        .press(c.pressed)
        .focusable(true)
        .focus_ring(2.0, -2.0, c.border_focus)
        .cursor(Cursor::Pointer)
        .child(content)
        .on_press_tx(move |app: &mut App, tx: &mut Tx| pick_example(app, example, tx));
    buttons::on_return(example_key(example), btn)
}

fn template_row_heading(look: &Look, name: &str, runtime_label: &str) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let badge = row()
        .px(5.0)
        .py(1.0)
        .radius(look.round(4.0))
        .border(1.0, kit.alpha(c.std_border, c.std_border.a * 0.40))
        .child(
            text(runtime_label.to_owned())
                .font(kit.ty.small)
                .color(c.text_muted)
                .a11y_hidden(),
        );
    row()
        .w_full()
        .gap(10.0)
        .items_center()
        .child(
            text(name.to_owned())
                .font(kit.ty.ui_medium)
                .color(c.text_base)
                .grow(1.0)
                .a11y_hidden(),
        )
        .child(badge)
}

fn replace_confirmation(look: &Look, pending: &'static ActionExample, divider: Color) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let question = rust_i18n::t!(
        "Replace your %{value1} draft with %{value2}? Code can be undone in the editor.",
        value1 = pending.runtime().label(),
        value2 = tr(pending.name)
    )
    .into_owned();
    let keep = small_button(
        look,
        "settings-action-library-keep",
        tr("Keep draft"),
        false,
    )
    .on_press(|app: &mut App| {
        if let Some(editor) = app.settings_dialog.actions.editor.as_mut() {
            editor.pending_example = None;
        }
    });
    let replace = small_button(
        look,
        "settings-action-library-replace",
        tr("Replace script"),
        true,
    )
    .on_press_tx(move |app: &mut App, tx: &mut Tx| apply_confirmed(app, pending, tx));
    col()
        .key("settings-action-library-confirm")
        .w_full()
        .px(12.0)
        .py(10.0)
        .gap(8.0)
        .border_sides([1.0, 0.0, 0.0, 0.0], divider)
        .child(text(question).font(kit.ty.small).color(c.text_muted).wrap())
        .child(
            row()
                .w_full()
                .justify_end()
                .gap(8.0)
                .child(keep)
                .child(replace),
        )
}

fn small_button(look: &Look, key: &'static str, label: String, accent: bool) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let (fill, border, ink) = if accent {
        (c.info.bg, c.border_active, c.text_accent)
    } else {
        (Color::TRANSPARENT, c.border_weak, c.text_base)
    };
    let btn = row()
        .role("button")
        .label(label.clone())
        .min_h(26.0)
        .px(10.0)
        .py(4.0)
        .center()
        .radius(look.round(5.0))
        .fill(fill)
        .border(1.0, border)
        .hover(if accent {
            kit.alpha(c.bg_accent, 0.18)
        } else {
            c.hover
        })
        .press(c.pressed)
        .focusable(true)
        .focus_ring(2.0, 1.0, kit.alpha(c.border_focus, 0.50))
        .cursor(Cursor::Pointer)
        .child(text(label).font(kit.ty.ui).color(ink).a11y_hidden());
    buttons::on_return(key, btn)
}

fn pick_example(app: &mut App, example: &'static ActionExample, tx: &mut Tx) {
    let replaces = app
        .settings_dialog
        .actions
        .editor
        .as_ref()
        .is_some_and(|editor| editor.replaces_draft(example.runtime()));
    if replaces {
        if let Some(editor) = app.settings_dialog.actions.editor.as_mut() {
            editor.pending_example = Some(example);
        }
        tx.focus_when_shown("settings-action-library-replace", true);
    } else {
        apply_confirmed(app, example, tx);
    }
}

fn apply_confirmed(app: &mut App, example: &'static ActionExample, tx: &mut Tx) {
    let later = app.settings_dialog.actions.later.downgrade();
    if let Some(editor) = app.settings_dialog.actions.editor.as_mut() {
        editor.apply_example(example, later);
        tx.focus_when_shown("field", true);
    }
}

pub(super) fn matches(example: &ActionExample, query: &str, category: &str) -> bool {
    if category != "All" && example.category != category {
        return false;
    }
    let fields = [
        example.name,
        example.description,
        example.requirements,
        example.category,
        example.inputs,
    ];
    // English stays searchable alongside the displayed language.
    let text = fields
        .iter()
        .flat_map(|field| [(*field).to_owned(), tr(field)])
        .chain([example.runtime().label().to_owned()])
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    query.split_whitespace().all(|word| text.contains(word))
}

fn example_key(example: &ActionExample) -> String {
    let slug: String = example
        .name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    format!("settings-action-library-item-{slug}")
}
