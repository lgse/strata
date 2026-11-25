// SPDX-License-Identifier: MIT

use gio::prelude::*;
use rune::{Cursor, El, KeyEvent, ScrollSpec, TextEdit, Tx, col, icon, row, text, text_input};
use rune_kit::Checkbox;
use strata_components::dialog::{Focus, StrataDialog, WIDTH};
use strata_core::i18n::tr;
use strata_kit::{icons::IconName, look::Look};

use super::{ChooserSpec, OpenWithContext};
use crate::{App, shell, slots::Frame};

const LIST_HEIGHT: f32 = 240.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum AppSection {
    Recent,
    Recommended,
    Other,
}

impl AppSection {
    const ALL: [Self; 3] = [Self::Recent, Self::Recommended, Self::Other];

    fn title(self) -> &'static str {
        match self {
            Self::Recent => "Recently Used",
            Self::Recommended => "Recommended Applications",
            Self::Other => "Other Applications",
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Self::Recent => "recent",
            Self::Recommended => "recommended",
            Self::Other => "other",
        }
    }
}

pub(super) struct AppEntry {
    pub(super) app: gio::AppInfo,
    pub(super) display_name: String,
    pub(super) description: Option<String>,
    pub(super) section: AppSection,
    pub(super) haystack: String,
}

impl AppEntry {
    fn new(app: gio::AppInfo, section: AppSection) -> Self {
        let display_name = app.display_name().to_string();
        let description = app.description().map(|value| value.to_string());
        let haystack = format!(
            "{} {} {} {} {}",
            display_name,
            app.name(),
            description.as_deref().unwrap_or_default(),
            app.executable().unwrap_or_default().to_string_lossy(),
            app.id().unwrap_or_default(),
        )
        .to_lowercase();
        Self {
            app,
            display_name,
            description,
            section,
            haystack,
        }
    }
}

pub(super) struct Form {
    pub(super) generation: u64,
    pub(super) files: Vec<gio::File>,
    pub(super) content_types: Vec<String>,
    pub(super) entries: Vec<AppEntry>,
    pub(super) context: OpenWithContext,
    pub(super) search: TextEdit,
    pub(super) selected: Option<usize>,
    pub(super) always_use: bool,
}

impl Form {
    pub(super) fn new(generation: u64, spec: ChooserSpec) -> Self {
        let ChooserSpec {
            files,
            content_types,
            choices,
            context,
        } = spec;
        let mut entries = Vec::new();
        for (section, apps) in [
            (AppSection::Recent, choices.recent),
            (AppSection::Recommended, choices.recommended),
            (AppSection::Other, choices.other),
        ] {
            entries.extend(apps.into_iter().map(|app| AppEntry::new(app, section)));
        }
        let has_apps = !entries.is_empty();
        let mut search = TextEdit::new("");
        search.focused = has_apps;
        Self {
            generation,
            files,
            content_types,
            entries,
            context,
            search,
            selected: has_apps.then_some(0),
            always_use: false,
        }
    }

    pub(super) fn has_apps(&self) -> bool {
        !self.entries.is_empty()
    }

    pub(super) fn field_focused(&self) -> bool {
        self.has_apps() && self.search.focused
    }

    pub(super) fn filtered_indices(&self) -> Vec<usize> {
        let query = self.search.text.trim().to_lowercase();
        (0..self.entries.len())
            .filter(|&index| query.is_empty() || self.entries[index].haystack.contains(&query))
            .collect()
    }

    pub(super) fn sync_selection(&mut self) {
        let visible = self.filtered_indices();
        if visible.is_empty() {
            self.selected = None;
        } else if self.selected.is_none_or(|sel| !visible.contains(&sel)) {
            self.selected = Some(visible[0]);
        }
    }

    pub(super) fn step_selection(&mut self, down: bool) -> Option<usize> {
        let visible = self.filtered_indices();
        let current = self.selected?;
        let pos = visible.iter().position(|&idx| idx == current)?;
        let next = if down {
            visible.get(pos + 1).copied().unwrap_or(current)
        } else if pos > 0 {
            visible[pos - 1]
        } else {
            current
        };
        self.selected = Some(next);
        Some(next)
    }
}

pub(super) fn dialog_key(generation: u64) -> String {
    format!("open-with-{generation}")
}

pub(super) fn search_key(generation: u64) -> String {
    format!("open-with-{generation}-search")
}

pub(super) fn row_key(generation: u64, index: usize) -> String {
    format!("open-with-{generation}-app-{index}")
}

pub(super) fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    let state = &app.open_with;
    let form = state.form.as_ref()?;
    let key = dialog_key(form.generation);
    let close_key = format!("{key}-close");
    let subtitle = tr(if form.files.len() == 1 {
        "Choose an application to open this item"
    } else {
        "Choose an application to open these items"
    });
    let has_apps = form.has_apps();
    let visible = form.filtered_indices();
    let has_matches = !visible.is_empty();
    let initial = if has_apps {
        Focus::First
    } else {
        Focus::Secondary
    };
    let mut dialog = StrataDialog::new(key, tr("Open With"), super::cancel)
        .on_dismiss_tx(move |app: &mut App, tx: &mut Tx| {
            super::dismiss_or_clear_search(app, tx, &close_key);
        })
        .open(state.open)
        .subtitle(subtitle)
        .icon(IconName::Open)
        .width(WIDTH)
        .children(body(&app.look, form, &visible))
        .secondary(tr("Cancel"), super::cancel)
        .primary_tx(tr("Open"), super::submit)
        .primary_enabled(has_apps && has_matches)
        .initial_focus(initial)
        .close_label(tr("Close dialog"));
    if let Some(view) = shell::view_focus(app) {
        dialog = dialog.return_focus(view);
    }
    dialog.build(&app.look, frame.layer);
    None
}

fn body(look: &Look, form: &Form, visible: &[usize]) -> Vec<El> {
    if !form.has_apps() {
        return vec![empty_message(look, &tr(form.context.empty_message()))];
    }
    let mut items = vec![search_field(look, form)];
    if visible.is_empty() {
        items.push(empty_message(
            look,
            &tr("No matching applications were found."),
        ));
    } else {
        items.push(app_list(look, form, visible));
    }
    if !form.content_types.is_empty() {
        items.push(always_use_checkbox(look, form));
    }
    items
}

fn empty_message(look: &Look, message: &str) -> El {
    col().w_full().py(look.space.xl).center().child(
        text(message.to_owned())
            .label(message.to_owned())
            .font(look.kit.ty.ui)
            .color(look.kit.c.text_muted)
            .wrap(),
    )
}

fn always_use_checkbox(look: &Look, form: &Form) -> El {
    let label = tr(if form.content_types.len() > 1 {
        "Always use for these file types"
    } else {
        "Always use for this file type"
    });
    Checkbox::new(form.always_use)
        .label(label)
        .on_toggle(super::toggle_always_use)
        .build(&look.kit)
        .key(format!("open-with-{}-always-use", form.generation))
}

fn search_field(look: &Look, form: &Form) -> El {
    let kit = &look.kit;
    let key = search_key(form.generation);
    let label = tr("Search applications…");
    let ink = kit.c.text_base;
    let input = text_input(&form.search, kit.ty.ui)
        .single_line()
        .key(key.clone())
        .label(label.clone())
        .placeholder(label, kit.c.text_faint)
        .color(ink)
        .caret(ink, kit.selection())
        .spellcheck(false)
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .on_edit(|app: &mut App| &mut app.open_with.form.as_mut().expect("open-with form").search)
        .on_input_change(|app: &mut App, _: &mut Tx| super::search_changed(app))
        .on_input_key(super::search_key_pressed);
    let inner = row()
        .key(format!("{key}-frame"))
        .w_full()
        .h(look.frame.control)
        .px(look.space.sm)
        .gap(look.space.xs)
        .items_center()
        .min_w(0.0)
        .child(
            icon(look.icon(IconName::Search), look.icon_size.sm)
                .color(kit.c.icon_muted)
                .a11y_hidden(),
        )
        .child(input)
        .role("textbox")
        .radius(kit.radius.md)
        .fill(kit.c.bg_base)
        .token_motion(kit.motion.control);
    if form.search.focused {
        inner.outline(1.0, -1.0, kit.c.border_focus)
    } else {
        kit.elevate(inner, kit.elevation.input())
    }
}

fn app_list(look: &Look, form: &Form, visible: &[usize]) -> El {
    let kit = &look.kit;
    let mut rows = Vec::new();
    for section in AppSection::ALL {
        let indices: Vec<usize> = visible
            .iter()
            .copied()
            .filter(|&idx| form.entries[idx].section == section)
            .collect();
        if indices.is_empty() {
            continue;
        }
        rows.push(section_heading(look, form.generation, section));
        rows.push(
            col()
                .role("none")
                .focus_group()
                .w_full()
                .gap(look.space.xxs)
                .children(indices.into_iter().map(|index| app_row(look, form, index))),
        );
    }
    col()
        .key(format!("open-with-{}-scroll", form.generation))
        .w_full()
        .max_h(LIST_HEIGHT)
        .min_h(0.0)
        .radius(kit.radius.md)
        .border(1.0, kit.c.border_muted)
        .fill(kit.c.bg_base)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .key(format!("open-with-{}-list", form.generation))
                .role("listbox")
                .label(tr("Applications"))
                .w_full()
                .padding(look.space.xs)
                .gap(look.space.xxs)
                .children(rows),
        )
}

fn section_heading(look: &Look, generation: u64, section: AppSection) -> El {
    let title = tr(section.title());
    row()
        .key(format!("open-with-{generation}-heading-{}", section.slug()))
        .role("listitem")
        .focusable(false)
        .w_full()
        .px(look.space.sm)
        .py(look.space.xs)
        .child(
            text(title.clone())
                .label(title)
                .font(look.kit.ty.small_medium)
                .color(look.kit.c.text_muted),
        )
}

fn app_row(look: &Look, form: &Form, index: usize) -> El {
    let kit = &look.kit;
    let entry = &form.entries[index];
    let is_selected = form.selected == Some(index);
    let key = row_key(form.generation, index);
    let mut labels = col().grow(1.0).shrink().min_w(0.0).child(
        text(entry.display_name.clone())
            .font(kit.ty.ui)
            .color(kit.c.text_base)
            .truncate()
            .a11y_hidden(),
    );
    if let Some(desc) = entry
        .description
        .as_deref()
        .filter(|value| !value.is_empty() && *value != entry.display_name)
    {
        labels = labels.child(
            text(desc.to_owned())
                .font(kit.ty.small)
                .color(kit.c.text_muted)
                .truncate()
                .a11y_hidden(),
        );
    }
    let fill = if is_selected {
        kit.c.layer_02
    } else {
        kit.c.bg_base
    };
    let mut item = row()
        .key(key)
        .role("option")
        .label(entry.display_name.clone())
        .focus_selected(is_selected)
        .w_full()
        .px(look.space.sm)
        .py(look.space.xs)
        .gap(look.space.sm)
        .items_center()
        .radius(kit.radius.sm)
        .fill(fill)
        .cursor(Cursor::Pointer)
        .child(
            icon(look.icon(IconName::FileCode), look.icon_size.lg)
                .color(kit.c.icon_base)
                .a11y_hidden(),
        )
        .child(labels)
        .on_press_tx(move |app: &mut App, tx: &mut Tx| super::select_row(app, index, tx))
        .on_double_press(move |app: &mut App| super::activate_row(app, index))
        .on_key(move |app: &mut App, ev: &KeyEvent, tx: &mut Tx| {
            super::list_key_pressed(app, index, ev, tx)
        })
        .focusable(is_selected);
    if let Some(desc) = &entry.description {
        item = item.description(desc.clone());
    }
    item
}
