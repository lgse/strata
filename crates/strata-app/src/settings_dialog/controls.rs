// SPDX-License-Identifier: MIT

//! Shared Settings rows, switches, segmented toggles, number fields, and GTK-compatible choice menus.

use std::rc::Rc;

use rune::{El, Key, KeyEvent, ScrollSpec, TextEdit, Tx, col, el, icon, row, text};
use rune_kit::{
    Button, ButtonVariant, IconButton, IconButtonVariant, Placement, Switch, TextInput, popover,
};
use strata_components::controls::{
    number_field::{NumberField, NumberFieldState},
    settings_search::{Navigate, SearchIndex, SearchLabels, SettingsSearch},
};
use strata_core::i18n::{count, tr};
use strata_desktop::settings::Preferences;
use strata_kit::{icons::IconName, look::Look};

use super::{PageId, search_index::translated};
use crate::{App, buttons, shell};

pub(super) const PAGE_SCROLL: &str = "settings-page";

#[derive(Clone, Copy)]
pub(super) struct PageCtx<'a> {
    pub(super) app: &'a App,
    pub(super) search: &'a SettingsSearch<App>,
}

pub(super) fn search_handle() -> SettingsSearch<App> {
    SettingsSearch::new(
        "settings-search",
        |app: &App| &app.settings_dialog.search,
        |app: &mut App| &mut app.settings_dialog.search,
        |app: &App| &app.settings_dialog.index,
    )
    .labels(SearchLabels {
        field: tr("Search settings"),
        clear: tr("Clear search"),
        results_title: tr("Search results"),
        no_match: tr("No settings match your search."),
        count: Rc::new(|n| count("results", n)),
    })
    .page_scroll(PAGE_SCROLL)
    .on_navigate(|app: &mut App, to: Navigate, _| {
        if let Some(page) = PageId::from_key(&to.page) {
            app.settings_dialog.set_page(page);
        }
    })
    .on_escape(|app: &mut App, _| {
        super::close_settings(app);
        true
    })
}

pub(super) type NumberFieldAccess = (
    fn(&App) -> &NumberFieldState,
    fn(&mut App) -> &mut NumberFieldState,
);
pub(super) type PickHandler = Rc<dyn Fn(&mut App, &mut Tx)>;
pub(super) type SegmentChoice = (String, bool, PickHandler);

pub(super) fn number_field(
    key: &'static str,
    label_key: &'static str,
    access: NumberFieldAccess,
    on_commit: fn(&mut App, f64, &mut Tx),
) -> NumberField<App> {
    NumberField::new(key, access.0, access.1)
        .label(tr(label_key))
        .on_change(on_commit)
}

pub(super) fn section_heading(ctx: PageCtx<'_>, section_key: &str, heading: &str) -> Option<El> {
    if !ctx.search.shows_section(ctx.app, section_key) {
        return None;
    }
    let look = &ctx.app.look;
    Some(
        text(tr(heading))
            .font(look.ty.heading)
            .color(look.kit.c.text_muted)
            .pt(look.space.md)
            .pb(look.space.xs),
    )
}

pub(super) fn group_card(look: &Look, rows: Vec<Option<El>>) -> Option<El> {
    let visible: Vec<El> = rows.into_iter().flatten().collect();
    if visible.is_empty() {
        return None;
    }
    let c = &look.kit.c;
    let mut card = col()
        .w_full()
        .radius(look.kit.radius.md)
        .fill(c.layer_02)
        .border(1.0, c.border_muted);
    for (idx, item) in visible.into_iter().enumerate() {
        let wrapped = if idx > 0 {
            col()
                .w_full()
                .border_sides([1.0, 0.0, 0.0, 0.0], c.border_muted)
                .child(item)
        } else {
            item
        };
        card = card.child(wrapped);
    }
    Some(card)
}

pub(super) struct RowSpec<'a> {
    pub(super) key: &'a str,
    pub(super) title: &'a str,
    pub(super) description: &'a str,
    pub(super) note: Option<&'a str>,
    pub(super) indented: bool,
    pub(super) disabled: bool,
}

impl<'a> RowSpec<'a> {
    pub(super) const fn new(key: &'a str, title: &'a str, description: &'a str) -> Self {
        Self {
            key,
            title,
            description,
            note: None,
            indented: false,
            disabled: false,
        }
    }

    pub(super) const fn note(mut self, note: Option<&'a str>) -> Self {
        self.note = note;
        self
    }

    pub(super) const fn indented(mut self) -> Self {
        self.indented = true;
        self
    }

    pub(super) const fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

pub(super) fn option_row(ctx: PageCtx<'_>, spec: RowSpec<'_>, control: El) -> Option<El> {
    if !ctx.search.shows_row(ctx.app, spec.key) {
        return None;
    }
    let look = &ctx.app.look;
    let c = &look.kit.c;
    let title_str = translated(spec.title);
    let desc_str = translated(spec.description);
    let mut title_line = row().gap(look.space.xs).items_center();
    if spec.indented {
        title_line = title_line.child(
            icon(look.icon(IconName::Enter), look.icon_size.sm)
                .color(c.icon_muted)
                .a11y_hidden(),
        );
    }
    title_line =
        title_line.child(
            ctx.search
                .title(ctx.app, look, &title_str, look.kit.ty.ui_medium),
        );
    let mut copy = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(look.space.xxs)
        .child(title_line);
    if !desc_str.is_empty() {
        copy = copy.child(
            text(desc_str)
                .font(look.kit.ty.small)
                .color(c.text_muted)
                .wrap(),
        );
    }
    if let Some(extra) = spec.note.filter(|s| !s.is_empty()) {
        copy = copy.child(
            text(tr(extra))
                .font(look.kit.ty.small)
                .color(c.text_muted)
                .wrap(),
        );
    }
    let mut line = row()
        .w_full()
        .min_w(0.0)
        .px(look.space.lg)
        .py(look.space.md)
        .gap(look.space.lg)
        .items_center()
        .child(copy)
        .child(control);
    if spec.disabled {
        line = line.layer_opacity(0.55);
    }
    ctx.search.row(ctx.app, look, spec.key, line)
}

pub(super) fn switch_row(
    ctx: PageCtx<'_>,
    spec: RowSpec<'_>,
    checked: bool,
    on_toggle: impl Fn(&mut App, bool, &mut Tx) + 'static,
) -> Option<El> {
    if !ctx.search.shows_row(ctx.app, spec.key) {
        return None;
    }
    let title_str = translated(spec.title);
    let desc_str = translated(spec.description);
    let switch_key = format!("settings-switch-{}", spec.key);
    let mut toggle = Switch::new(checked)
        .label(title_str)
        .hide_label()
        .disabled(spec.disabled)
        .on_toggle_tx(move |app: &mut App, tx: &mut Tx| {
            on_toggle(app, !checked, tx);
        })
        .build(&ctx.app.look.kit)
        .key(switch_key);
    if !desc_str.is_empty() {
        toggle = toggle.description(desc_str);
    }
    option_row(ctx, spec, toggle)
}

pub(super) fn pref_switch(
    ctx: PageCtx<'_>,
    spec: RowSpec<'_>,
    checked: bool,
    write: fn(&mut Preferences, bool),
) -> Option<El> {
    switch_row(ctx, spec, checked, move |app, on, _| {
        shell::saved(app).change(|s| write(s, on));
    })
}

pub(super) struct ChoiceOption {
    pub(super) label: String,
    pub(super) subtitle: Option<String>,
    pub(super) selected: bool,
    pub(super) on_pick: PickHandler,
}

impl ChoiceOption {
    pub(super) fn new(
        label: impl Into<String>,
        selected: bool,
        on_pick: impl Fn(&mut App, &mut Tx) + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            subtitle: None,
            selected,
            on_pick: Rc::new(on_pick),
        }
    }

    pub(super) fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }
}

pub(super) struct ChoiceSpec<'a> {
    pub(super) id: &'static str,
    pub(super) title: &'a str,
    pub(super) current: String,
    pub(super) disabled: bool,
    pub(super) options: Vec<ChoiceOption>,
}

struct PopupSpec<'a> {
    id: &'static str,
    title: &'a str,
    anchor_key: &'a str,
    options: Vec<ChoiceOption>,
}

pub(super) fn choice_menu(app: &App, spec: ChoiceSpec<'_>, menu_layer: &mut Vec<El>) -> El {
    let look = &app.look;
    let kit = &look.kit;
    let id = spec.id;
    let is_open = !spec.disabled && app.settings_dialog.open_choice == Some(id);
    let title = tr(spec.title);
    let current = spec.current;
    let toggle_key = format!("settings-choice-{id}-toggle");
    let first_opt_key = format!("settings-choice-{id}-opt-0");
    let chevron = look.icon(IconName::ChevronDown);
    let shown_val = current.clone();
    let toggle = Button::new(current.clone())
        .variant(ButtonVariant::Neutral)
        .disabled(spec.disabled)
        .label_with(move |font, ink| {
            row()
                .gap(8.0)
                .items_center()
                .child(text(shown_val).font(font).color(ink).a11y_hidden())
                .child(icon(chevron, 14.0).color(ink).a11y_hidden())
        })
        .on_press_tx(move |app: &mut App, tx: &mut Tx| {
            if app.settings_dialog.open_choice == Some(id) {
                app.settings_dialog.open_choice = None;
            } else {
                app.settings_dialog.open_choice = Some(id);
                if !first_opt_key.is_empty() {
                    tx.focus_when_shown(first_opt_key.clone(), tx.focus_visible() || tx.by_keys());
                }
            }
        })
        .build(kit)
        .label(title.clone())
        .description(current.clone())
        .checked(is_open);
    let inner = buttons::on_return(toggle_key.clone(), toggle);
    if is_open {
        let popup = PopupSpec {
            id,
            title: &title,
            anchor_key: &toggle_key,
            options: spec.options,
        };
        build_choice_popup(look, popup, menu_layer);
    }
    el().key(format!("settings-choice-{id}"))
        .role("button")
        .label(title)
        .description(current)
        .focusable(false)
        .child(inner)
}

fn build_choice_popup(look: &Look, spec: PopupSpec<'_>, menu_layer: &mut Vec<El>) {
    let kit = &look.kit;
    let id = spec.id;
    let stops: Vec<String> = (0..spec.options.len())
        .map(|idx| format!("settings-choice-{id}-opt-{idx}"))
        .collect();
    let rows: Vec<El> = spec
        .options
        .into_iter()
        .enumerate()
        .map(|(idx, opt)| build_choice_option(look, &stops[idx], opt, spec.anchor_key))
        .collect();
    let list = col()
        .min_w(196.0)
        .max_h(320.0)
        .padding(look.space.xs)
        .gap(look.space.xxs)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .children(rows);
    let anchor_for_dismiss = spec.anchor_key.to_owned();
    let anchor_for_keys = spec.anchor_key.to_owned();
    let body = list
        .key(format!("settings-choice-{id}-menu"))
        .role("menu")
        .label(spec.title.to_owned())
        .focus_return(spec.anchor_key.to_owned())
        .on_key(move |app: &mut App, ev: &KeyEvent, tx: &mut Tx| {
            choice_menu_key(app, ev, tx, (&stops, &anchor_for_keys))
        })
        .modal_tx(true, move |app: &mut App, tx: &mut Tx| {
            app.settings_dialog.open_choice = None;
            tx.focus(anchor_for_dismiss.clone(), true);
        });
    let surface = popover::raised(kit, body, kit.radius.md);
    menu_layer.push(popover::anchored(
        kit,
        surface,
        spec.anchor_key,
        Placement::Bottom,
        look.space.xs,
    ));
}

fn build_choice_option(look: &Look, key: &str, opt: ChoiceOption, anchor_key: &str) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let check = look.icon(IconName::Check);
    let name = opt.label.clone();
    let sub = opt.subtitle.clone();
    let sub_for_a11y = opt.subtitle;
    let selected = opt.selected;
    let on_pick = opt.on_pick;
    let anchor = anchor_key.to_owned();
    let mut copy = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(2.0)
        .child(text(name).font(kit.ty.ui).color(c.text_base));
    if let Some(sub) = sub {
        copy = copy.child(
            text(sub)
                .font(kit.ty.small)
                .color(c.text_muted)
                .a11y_hidden(),
        );
    }
    let mut item = row()
        .w_full()
        .min_h(look.frame.control)
        .px(look.space.sm)
        .py(look.space.xs)
        .gap(10.0)
        .items_center()
        .radius(kit.radius.sm)
        .hover(c.layer_02)
        .focus_ring(1.0, -1.0, c.border_focus)
        .focusable(true)
        .role("menuitemradio")
        .label(opt.label)
        .checked(selected)
        .on_press_tx(move |app: &mut App, tx: &mut Tx| {
            app.settings_dialog.open_choice = None;
            on_pick(app, tx);
            tx.focus(anchor.clone(), true);
        })
        .child(copy)
        .children(selected.then(|| icon(check, 16.0).color(c.text_base).a11y_hidden()));
    if let Some(desc) = sub_for_a11y {
        item = item.description(desc);
    }
    buttons::on_return(key.to_owned(), item)
}

fn choice_menu_key(
    app: &mut App,
    ev: &KeyEvent,
    tx: &mut Tx,
    (stops, anchor): (&[String], &str),
) -> bool {
    if ev.key == Key::Escape {
        app.settings_dialog.open_choice = None;
        tx.focus(anchor.to_owned(), true);
        return true;
    }
    let Some(last) = stops.len().checked_sub(1) else {
        return false;
    };
    let at = tx
        .focused()
        .and_then(|focused| stops.iter().position(|stop| stop == focused));
    let next = match ev.key {
        Key::Down => at.map_or(0, |i| if i == last { 0 } else { i + 1 }),
        Key::Up => at.map_or(last, |i| if i == 0 { last } else { i - 1 }),
        Key::Tab if ev.mods.shift => at.map_or(last, |i| if i == 0 { last } else { i - 1 }),
        Key::Tab => at.map_or(0, |i| if i == last { 0 } else { i + 1 }),
        Key::Home => 0,
        Key::End => last,
        _ => return false,
    };
    tx.focus(stops[next].clone(), true);
    true
}

/// A page's own search field and its Clear search button: GTK's `settings::search_field`
/// (`src/ui/settings.rs`), which the theme library and the keybind search share.
pub(super) struct SearchField {
    pub(super) key: &'static str,
    pub(super) clear_key: &'static str,
    pub(super) label: String,
    pub(super) edit: fn(&App) -> &TextEdit,
    pub(super) edit_mut: fn(&mut App) -> &mut TextEdit,
    /// A fixed width, or none to fill the row.
    pub(super) width: Option<f32>,
    pub(super) on_change: fn(&mut App),
}

pub(super) fn search_field(app: &App, spec: SearchField) -> El {
    let look = &app.look;
    let edit = (spec.edit)(app);
    let (key, edit_mut, on_change) = (spec.key, spec.edit_mut, spec.on_change);
    let mut input = TextInput::new(edit)
        .label(spec.label.clone())
        .placeholder(spec.label)
        .width(spec.width)
        .on_edit(edit_mut)
        .on_change(move |app: &mut App, _: &mut Tx| on_change(app))
        .build(&look.kit)
        .key(key);
    let mut field = row().gap(look.space.xs).items_center();
    if spec.width.is_none() {
        input = input.grow(1.0);
        field = field.w_full();
    }
    field = field.child(input);
    if !edit.text.is_empty() {
        let clear = IconButton::new(look.icon(IconName::Close))
            .variant(IconButtonVariant::Ghost)
            .label(tr("Clear search"))
            .on_press_tx(move |app: &mut App, tx: &mut Tx| {
                edit_mut(app).replace_text("");
                tx.focus(key, true);
            })
            .build(&look.kit);
        field = field.child(buttons::on_return(spec.clear_key, clear));
    }
    field
}

pub(super) fn segmented_toggles(
    look: &Look,
    key_prefix: &str,
    group_label: Option<String>,
    choices: Vec<SegmentChoice>,
) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let items: Vec<El> = choices
        .into_iter()
        .enumerate()
        .map(|(idx, (label, active, on_pick))| {
            let variant = if active {
                ButtonVariant::Submit
            } else {
                ButtonVariant::Ghost
            };
            let btn = Button::new(label)
                .variant(variant)
                .on_press_tx(move |app: &mut App, tx: &mut Tx| on_pick(app, tx))
                .build(kit)
                .checked(active);
            buttons::on_return(format!("{key_prefix}-{idx}"), btn)
        })
        .collect();
    let mut bar = row()
        .key(key_prefix.to_owned())
        .gap(look.space.xxs)
        .padding(look.space.xxs)
        .radius(kit.radius.md)
        .fill(c.bg_base)
        .border(1.0, c.border_muted)
        .children(items);
    if let Some(label) = group_label {
        bar = bar.role("group").label(label);
    }
    bar
}

fn index_changed(current: &SearchIndex, fresh: &SearchIndex, same_locale: bool) -> bool {
    !same_locale || fresh.pages != current.pages || fresh.rows != current.rows
}

pub(super) fn sync_search_index_if_needed(
    index: &mut SearchIndex,
    locale: &mut String,
    avail: super::search_index::Availability,
) {
    let now_locale = rust_i18n::locale().to_string();
    let fresh = super::search_index::build_index(index.revision, avail);
    if index_changed(index, &fresh, *locale == now_locale) {
        let mut updated = fresh;
        updated.revision = index.revision + 1;
        *index = updated;
        *locale = now_locale;
    }
}
