// SPDX-License-Identifier: MIT

//! Interactive Global Search Exclusions editor in General settings.

use rune::{
    Color, Cursor, El, Key, KeyEvent, LiveRegion, Mods, ScrollSpec, TextEdit, Tx, col, icon, row,
    text,
};
use rune_kit::{Button, ButtonVariant, TextInput};
use strata_components::scroll::Bar;
use strata_core::i18n::tr;
use strata_fs::services::SearchExclusions;
use strata_kit::{icons::IconName, look::Look};

use super::{
    controls::{
        PageCtx, SCROLLBAR_EDGE_INSET, SCROLLBAR_END_INSET, bounded_scroll_with_bar,
        control_border, panel_border, settings_bar, settings_bar_look,
    },
    folder_picker, general_more,
};
use crate::{App, buttons, shell};

#[cfg(test)]
mod tests;

pub(super) const INPUT_KEY: &str = "settings-search-exclusions-input";
pub(super) const BROWSE_KEY: &str = "settings-search-exclusions-browse";
pub(super) const ADD_KEY: &str = "settings-search-exclusions-add";
pub(super) const ERROR_KEY: &str = "settings-search-exclusions-error";
pub(super) const EMPTY_KEY: &str = "settings-search-exclusions-empty";
const SCROLL_KEY: &str = "settings-search-exclusions-scroll";
const SCROLLBAR_KEY: &str = "settings-search-exclusions-scrollbar";
const LIST_MAX_HEIGHT: f32 = 180.0;

pub(super) struct ExclusionsState {
    pub(super) input: TextEdit,
    pub(super) error: Option<String>,
    list_bar: Bar,
}

impl ExclusionsState {
    pub(super) fn new() -> Self {
        Self {
            input: TextEdit::default(),
            error: None,
            list_bar: settings_bar(),
        }
    }

    pub(super) fn focused(&self) -> bool {
        self.input.focused
    }

    pub(super) fn release_bar(&self) {
        self.list_bar.release();
    }

    pub(super) fn closed(&mut self) {
        self.input.replace_text("");
        self.error = None;
        self.list_bar.release();
    }
}

pub(super) fn build_row(ctx: PageCtx<'_>) -> Option<El> {
    let app = ctx.app;
    if !ctx.search.shows_row(app, "search-exclusions") {
        return None;
    }
    let look = &app.look;
    let kit = &look.kit;
    let state = &app.settings_dialog.exclusions;
    let exclusions = &shell::preferences(app).search_exclusions;

    let mut content = col()
        .w_full()
        .min_w(0.0)
        .px(18.0)
        .py(16.0)
        .gap(look.space.sm)
        .child(header_copy(ctx))
        .child(input_row(ctx, state));

    if let Some(error) = &state.error {
        content = content.child(
            text(error.clone())
                .key(ERROR_KEY)
                .font(kit.ty.small)
                .color(kit.c.danger.fg)
                .live_region(LiveRegion::Polite)
                .wrap(),
        );
    }

    content = content.child(exclusions_list(look, &state.list_bar, exclusions));
    ctx.search.row(app, look, "search-exclusions", content)
}

fn header_copy(ctx: PageCtx<'_>) -> El {
    let look = &ctx.app.look;
    let kit = &look.kit;
    col()
        .gap(3.0)
        .child(ctx.search.title(
            ctx.app,
            look,
            &tr("Global search exclusions"),
            kit.ty.ui_medium,
        ))
        .child(
            text(tr(
                "Exclude folder names or directory paths from global search.",
            ))
            .font(kit.ty.small)
            .color(kit.c.text_muted)
            .wrap(),
        )
}

fn input_row(ctx: PageCtx<'_>, state: &ExclusionsState) -> El {
    let look = &ctx.app.look;
    let kit = &look.kit;
    let mut field = TextInput::new(&state.input)
        .label(tr("Search exclusion"))
        .placeholder(tr("Folder name (e.g. .venv) or ~/path…"))
        .width(None)
        .invalid(state.error.is_some())
        .on_edit(|app: &mut App| &mut app.settings_dialog.exclusions.input)
        .on_change(|app: &mut App, _: &mut Tx| {
            app.settings_dialog.exclusions.error = None;
        })
        .on_key(|app: &mut App, ev: &KeyEvent, _: &mut Tx| {
            if ev.key == Key::Enter && ev.mods == Mods::default() {
                add_exclusion(app);
                return true;
            }
            false
        });
    if let Some(error) = &state.error {
        field = field.description(error.clone());
    }
    let input = field.build(kit).key(INPUT_KEY);

    let browse = Button::new(tr("Browse…"))
        .variant(ButtonVariant::Neutral)
        .on_press_tx(browse_folder)
        .build(kit);
    let add = Button::new(tr("Add"))
        .variant(ButtonVariant::Neutral)
        .on_press_tx(|app: &mut App, _: &mut Tx| add_exclusion(app))
        .build(kit);
    let browse_btn = buttons::on_return(BROWSE_KEY, browse);
    let add_btn = buttons::on_return(ADD_KEY, add);

    if ctx.layout.compact_content {
        col()
            .w_full()
            .min_w(0.0)
            .gap(look.space.sm)
            .child(input.w_full().min_w(0.0))
            .child(
                row()
                    .w_full()
                    .min_w(0.0)
                    .gap(look.space.sm)
                    .justify_end()
                    .items_center()
                    .child(browse_btn)
                    .child(add_btn),
            )
    } else {
        row()
            .w_full()
            .min_w(0.0)
            .gap(look.space.sm)
            .items_center()
            .child(input.grow(1.0).min_w(0.0))
            .child(browse_btn)
            .child(add_btn)
    }
}

fn exclusions_list(look: &Look, list_bar: &Bar, exclusions: &[String]) -> El {
    let kit = &look.kit;
    let border = panel_border(look);
    let inner = if exclusions.is_empty() {
        col().w_full().min_w(0.0).px(12.0).py(10.0).child(
            text(tr(
                "No custom exclusions added. Common tool caches (.venv, node_modules, target, etc.) are excluded automatically.",
            ))
            .key(EMPTY_KEY)
            .font(kit.ty.small)
            .color(kit.c.text_muted)
            .wrap(),
        )
    } else {
        let rows: Vec<El> = exclusions
            .iter()
            .enumerate()
            .map(|(idx, item)| exclusion_row(look, idx, item, border))
            .collect();
        col().w_full().min_w(0.0).children(rows)
    };

    let viewport = col()
        .key(SCROLL_KEY)
        .w_full()
        .max_h(LIST_MAX_HEIGHT)
        .shrink()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .on_scroll_offset(list_bar.watch())
        .child(inner);

    let scrolled = bounded_scroll_with_bar(
        viewport,
        list_bar,
        (SCROLLBAR_KEY, SCROLL_KEY),
        settings_bar_look(look, SCROLLBAR_EDGE_INSET, SCROLLBAR_END_INSET),
    );
    col()
        .w_full()
        .min_w(0.0)
        .radius(look.round(8.0))
        .fill(look.fill.muted)
        .border(1.0, control_border(look))
        .child(scrolled)
}

fn exclusion_row(look: &Look, idx: usize, item: &str, border: Color) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let kind_label = tr(if SearchExclusions::is_directory_path(item) {
        "Directory"
    } else {
        "Folder name"
    });
    let remove = remove_button(look, idx, item);
    let mut line = row()
        .key(format!("settings-search-exclusion-row-{idx}"))
        .w_full()
        .min_w(0.0)
        .px(12.0)
        .py(8.0)
        .gap(9.0)
        .items_center()
        .child(
            icon(look.icon(IconName::Folder), 16.0)
                .color(c.icon_accent)
                .a11y_hidden(),
        )
        .child(
            text(item.to_owned())
                .font(kit.ty.ui)
                .color(c.text_base)
                .truncate()
                .grow(1.0)
                .min_w(0.0),
        )
        .child(text(kind_label).font(kit.ty.small).color(c.text_muted))
        .child(remove);
    if idx > 0 {
        line = line.border_sides([1.0, 0.0, 0.0, 0.0], border);
    }
    line
}

fn remove_button(look: &Look, idx: usize, item: &str) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let target = item.to_owned();
    let label = rust_i18n::t!("Remove exclusion %{item}", item = item).into_owned();
    let btn = row()
        .role("button")
        .label(label)
        .help(tr("Remove exclusion"))
        .size(28.0, 28.0)
        .center()
        .radius(look.round(5.0))
        .fill(Color::TRANSPARENT)
        .border(1.0, kit.alpha(c.danger.fg, 0.40))
        .hover(kit.alpha(c.danger.fg, 0.14))
        .press(kit.alpha(c.danger.fg, 0.22))
        .focusable(true)
        .focus_ring(2.0, 1.0, c.danger.fg)
        .cursor(Cursor::Pointer)
        .child(
            icon(look.icon(IconName::Trash), 16.0)
                .color(c.danger.icon)
                .a11y_hidden(),
        )
        .on_press_tx(move |app: &mut App, _: &mut Tx| {
            let item = target.clone();
            shell::saved(app).change(|s| s.search_exclusions.retain(|c| c != &item));
        });
    buttons::on_return(format!("settings-search-exclusion-remove-{idx}"), btn)
}

pub(super) fn validate_exclusion_input(
    raw: &str,
    current: &[String],
) -> Result<String, &'static str> {
    let normalized = SearchExclusions::normalize_entry(raw)?;
    let is_path = SearchExclusions::is_directory_path(&normalized);
    let is_duplicate = current
        .iter()
        .filter_map(|item| SearchExclusions::normalize_entry(item).ok())
        .any(|existing| {
            if is_path {
                existing == normalized
            } else {
                existing.eq_ignore_ascii_case(&normalized)
            }
        });
    if is_duplicate {
        return Err("This exclusion has already been added.");
    }
    Ok(normalized)
}

fn add_exclusion(app: &mut App) {
    let raw = app.settings_dialog.exclusions.input.text.clone();
    let current = shell::preferences(app).search_exclusions.clone();
    match validate_exclusion_input(&raw, &current) {
        Ok(normalized) => {
            app.settings_dialog.exclusions.error = None;
            shell::saved(app).change(|s| s.search_exclusions.push(normalized));
            app.settings_dialog.exclusions.input.replace_text("");
        }
        Err(message) => {
            app.settings_dialog.exclusions.error = Some(tr(message));
        }
    }
}

fn browse_folder(app: &mut App, _: &mut Tx) {
    folder_picker::pick_folder(app, tr("Select folder to exclude"), |app, tx, picked| {
        if !app.settings_dialog.is_open() {
            return false;
        }
        match picked {
            Ok(Some(path)) => {
                let display = general_more::abbreviate_home(&path);
                app.settings_dialog.exclusions.error = None;
                app.settings_dialog.exclusions.input.replace_text(&display);
                tx.focus(INPUT_KEY, true);
                true
            }
            Ok(None) => false,
            Err(message) => {
                app.settings_dialog.exclusions.error = Some(message);
                true
            }
        }
    });
}
