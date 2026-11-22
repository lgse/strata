// SPDX-License-Identifier: MIT

//! Shared Settings rows, switches, segmented toggles, number fields, and GTK-compatible choice menus.

use std::rc::Rc;

use rune::{Color, Cursor, El, Key, KeyEvent, ScrollSpec, TextEdit, Tx, col, el, icon, row, text};
use rune_kit::{IconButton, IconButtonVariant, Placement, TextInput, popover};
use strata_components::{
    controls::{
        number_field::{NumberField, NumberFieldState},
        settings_search::{Navigate, SearchIndex, SearchLabels, SettingsSearch},
        switch::Switch,
    },
    scroll::{Bar, BarLook, BarMode, anchor},
};
use strata_core::i18n::{count, tr};
use strata_desktop::settings::Preferences;
use strata_kit::{icons::IconName, look::Look};

use super::{PageId, search_index::translated};
use crate::{App, buttons, shell};

pub(super) const PAGE_SCROLL: &str = "settings-page";
pub(super) const SCROLLBAR_EDGE_INSET: f32 = 2.0;
pub(super) const SCROLLBAR_END_INSET: f32 = 4.0;

/// A Settings scroll bar: shown whenever its viewport overflows, hidden when it fits.
pub(super) fn settings_bar() -> Bar {
    let bar = Bar::new();
    bar.set_mode(BarMode::Always);
    bar
}

/// Standard Strata C24 accent bar look for Settings viewports, inset from borders and corners.
pub(super) fn settings_bar_look(look: &Look, edge: f32, ends: f32) -> BarLook {
    let mut bar_look = BarLook::app(look);
    bar_look.edge = edge;
    bar_look.ends = ends;
    bar_look
}

/// Wraps a `max_h`-bounded scroll viewport with its C24 accent scrollbar without forcing full
/// height, so short lists shrink to their content while overflowing lists show the bar.
pub(super) fn bounded_scroll_with_bar(
    viewport: El,
    bar: &Bar,
    (bar_key, scroll_key): (&str, &str),
    bar_look: BarLook,
) -> El {
    let bar_el = bar
        .build::<App>(bar_key, scroll_key, bar_look)
        .wheel_scrolls(scroll_key.to_owned());
    col()
        .w_full()
        .shrink()
        .min_h(0.0)
        .child(viewport)
        .child(anchor(0.0, 0.0, bar_el))
}

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
    .on_navigate(|app: &mut App, to: Navigate, tx: &mut Tx| {
        if let Some(page) = PageId::from_key(&to.page) {
            let tenxer_mode = shell::preferences(app).tenxer_mode;
            let changed = app.settings_dialog.select_page(page, tenxer_mode);
            if changed && to.row.is_none() {
                tx.scroll_to_instant(PAGE_SCROLL, 0.0, 0.0);
            }
            if page == PageId::Actions {
                super::actions::ensure_observing(app);
            }
            if page == PageId::Integrations {
                super::integrations::ensure_scanned(app);
            }
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

/// Blends `base` toward `other` by `t` in sRGB, matching GTK CSS `mix(base, other, t)`.
/// Rune's `Color::mix` interpolates in linear light, which lifts dark-to-light mixes much
/// brighter than GTK's sRGB mix.
pub(super) fn mix_srgb(base: Color, other: Color, t: f32) -> Color {
    let ch = |a: f32, b: f32| {
        let sa = rune::color::srgb_encode(a);
        let sb = rune::color::srgb_encode(b);
        rune::color::srgb_decode(sa + (sb - sa) * t)
    };
    Color {
        r: ch(base.r, other.r),
        g: ch(base.g, other.g),
        b: ch(base.b, other.b),
        a: base.a + (other.a - base.a) * t,
    }
}

/// Card background matching GTK's `.settings-group { background: mix(@strata_bg, @strata_muted, 0.25); }`.
pub(super) fn card_surface(look: &Look) -> Color {
    mix_srgb(look.kit.c.bg_base, look.fill.muted, 0.25)
}

/// Panel/card/divider border matching GTK's `alpha(@strata_border, 0.55)`.
pub(super) fn panel_border(look: &Look) -> Color {
    let border = look.kit.c.std_border;
    look.kit.alpha(border, border.a * 0.55)
}

/// Form control border matching GTK's `.settings-dialog .form-control { border: 1px solid alpha(@strata_border, 0.9); }`.
pub(super) fn control_border(look: &Look) -> Color {
    let border = look.kit.c.std_border;
    look.kit.alpha(border, border.a * 0.90)
}

pub(super) fn first_section_heading(
    ctx: PageCtx<'_>,
    section_key: &str,
    heading: &str,
) -> Option<El> {
    heading_with_top(ctx, section_key, heading, 0.0)
}

pub(super) fn section_heading(ctx: PageCtx<'_>, section_key: &str, heading: &str) -> Option<El> {
    heading_with_top(ctx, section_key, heading, 24.0)
}

fn heading_with_top(
    ctx: PageCtx<'_>,
    section_key: &str,
    heading: &str,
    top_pad: f32,
) -> Option<El> {
    if !ctx.search.shows_section(ctx.app, section_key) {
        return None;
    }
    let look = &ctx.app.look;
    Some(
        text(tr(heading))
            .font(look.ty.heading)
            .color(look.kit.c.text_muted)
            .px(4.0)
            .pt(top_pad)
            .pb(10.0),
    )
}

pub(super) fn group_card(look: &Look, rows: Vec<Option<El>>) -> Option<El> {
    let visible: Vec<El> = rows.into_iter().flatten().collect();
    if visible.is_empty() {
        return None;
    }
    let border = panel_border(look);
    let mut card = col()
        .w_full()
        .min_w(0.0)
        .radius(look.round(10.0))
        .fill(card_surface(look))
        .border(1.0, border);
    for (idx, item) in visible.into_iter().enumerate() {
        let wrapped = if idx > 0 {
            col()
                .w_full()
                .min_w(0.0)
                .border_sides([1.0, 0.0, 0.0, 0.0], border)
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
                .color(look.kit.alpha(c.icon_accent, 0.65))
                .a11y_hidden(),
        );
    }
    title_line =
        title_line.child(
            ctx.search
                .title(ctx.app, look, &title_str, look.kit.ty.ui_medium),
        );
    let mut copy = col().grow(1.0).min_w(0.0).gap(3.0).child(title_line);
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
        .pl(if spec.indented { 28.0 } else { 18.0 })
        .pr(18.0)
        .py(16.0)
        .gap(look.space.xl)
        .items_center()
        .child(copy)
        .child(control);
    if spec.disabled {
        line = line.layer_opacity(0.45);
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
        .disabled(spec.disabled)
        .on_toggle_tx(move |app: &mut App, tx: &mut Tx| {
            on_toggle(app, !checked, tx);
        });
    if !desc_str.is_empty() {
        toggle = toggle.description(desc_str);
    }
    option_row(ctx, spec, toggle.build(&ctx.app.look).key(switch_key))
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

fn choice_colors(look: &Look, disabled: bool, is_open: bool) -> (Color, Color, Color) {
    let c = &look.kit.c;
    if disabled {
        (c.surface_disabled, c.border_disabled, c.text_muted)
    } else if is_open {
        (look.fill.highlight, c.border_active, c.text_accent)
    } else {
        (look.fill.muted, control_border(look), c.text_base)
    }
}

pub(super) fn choice_menu(app: &App, spec: ChoiceSpec<'_>, menu_layer: &mut Vec<El>) -> El {
    let look = &app.look;
    let kit = &look.kit;
    let c = &kit.c;
    let id = spec.id;
    let is_open = !spec.disabled && app.settings_dialog.open_choice == Some(id);
    let title = tr(spec.title);
    let current = spec.current;
    let toggle_key = format!("settings-choice-{id}-toggle");
    let first_opt_key = format!("settings-choice-{id}-opt-0");
    let (fill, border, ink) = choice_colors(look, spec.disabled, is_open);
    let mut toggle = row()
        .role("button")
        .label(title.clone())
        .description(current.clone())
        .checked(is_open)
        .min_w(160.0)
        .min_h(32.0)
        .px(12.0)
        .py(6.0)
        .gap(8.0)
        .items_center()
        .radius(look.round(8.0))
        .fill(fill)
        .border(1.0, border)
        .focus_ring(2.0, -2.0, c.border_focus)
        .focusable(!spec.disabled)
        .disabled(spec.disabled)
        .child(
            el().grow(1.0).center().child(
                text(current.clone())
                    .font(kit.ty.ui)
                    .color(ink)
                    .a11y_hidden(),
            ),
        )
        .child(
            icon(look.icon(IconName::ChevronDown), 14.0)
                .color(ink)
                .a11y_hidden(),
        );
    toggle = if spec.disabled {
        toggle.layer_opacity(0.45)
    } else {
        toggle
            .cursor(Cursor::Pointer)
            .hover(c.hover)
            .press(look.fill.highlight)
            .on_press_tx(move |app: &mut App, tx: &mut Tx| {
                toggle_choice_popup(app, id, &first_opt_key, tx);
            })
    };
    let inner = buttons::on_return(toggle_key.clone(), toggle);
    if is_open {
        let popup = PopupSpec {
            id,
            title: &title,
            anchor_key: &toggle_key,
            options: spec.options,
        };
        build_choice_popup(look, &app.settings_dialog.choice_bar, popup, menu_layer);
    }
    el().key(format!("settings-choice-{id}"))
        .role("button")
        .label(title)
        .description(current)
        .focusable(false)
        .child(inner)
}

fn close_choice(app: &mut App) {
    app.settings_dialog.open_choice = None;
    app.settings_dialog.choice_bar.release();
}

fn toggle_choice_popup(app: &mut App, id: &'static str, first_opt_key: &str, tx: &mut Tx) {
    if app.settings_dialog.open_choice == Some(id) {
        close_choice(app);
    } else {
        app.settings_dialog.choice_bar.release();
        app.settings_dialog.open_choice = Some(id);
        if !first_opt_key.is_empty() {
            tx.focus_when_shown(first_opt_key.to_owned(), tx.focus_visible() || tx.by_keys());
        }
    }
}

fn build_choice_popup(
    look: &Look,
    choice_bar: &Bar,
    spec: PopupSpec<'_>,
    menu_layer: &mut Vec<El>,
) {
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
    let scrolled = choice_scroll_list(look, choice_bar, id, rows);
    let anchor_for_dismiss = spec.anchor_key.to_owned();
    let anchor_for_keys = spec.anchor_key.to_owned();
    let body = col()
        .key(format!("settings-choice-{id}-menu"))
        .role("menu")
        .label(spec.title.to_owned())
        .min_w(196.0)
        .focus_return(spec.anchor_key.to_owned())
        .on_key(move |app: &mut App, ev: &KeyEvent, tx: &mut Tx| {
            choice_menu_key(app, ev, tx, (&stops, &anchor_for_keys))
        })
        .modal_tx(true, move |app: &mut App, tx: &mut Tx| {
            close_choice(app);
            tx.focus(anchor_for_dismiss.clone(), true);
        })
        .child(scrolled);
    let surface = popover::raised(kit, body, kit.radius.md);
    menu_layer.push(popover::anchored(
        kit,
        surface,
        spec.anchor_key,
        Placement::Bottom,
        look.space.xs,
    ));
}

fn choice_scroll_list(look: &Look, choice_bar: &Bar, id: &'static str, rows: Vec<El>) -> El {
    let inner = col()
        .w_full()
        .min_w(0.0)
        .pl(look.space.xs)
        .pr(look.space.lg)
        .pt(look.space.xs)
        .pb(look.space.md)
        .gap(look.space.xxs)
        .children(rows);
    let scroll_key = format!("settings-choice-{id}-scroll");
    let bar_key = format!("settings-choice-{id}-scrollbar");
    let viewport = col()
        .key(scroll_key.clone())
        .w_full()
        .max_h(320.0)
        .shrink()
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .on_scroll_offset(choice_bar.watch())
        .child(inner);
    bounded_scroll_with_bar(
        viewport,
        choice_bar,
        (&bar_key, &scroll_key),
        settings_bar_look(look, SCROLLBAR_EDGE_INSET, SCROLLBAR_END_INSET),
    )
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
        .hover(c.hover)
        .focus_ring(2.0, -2.0, c.border_focus)
        .focusable(true)
        .role("menuitemradio")
        .label(opt.label)
        .checked(selected)
        .on_press_tx(move |app: &mut App, tx: &mut Tx| {
            close_choice(app);
            on_pick(app, tx);
            tx.focus(anchor.clone(), true);
        })
        .child(copy)
        .children(selected.then(|| icon(check, 16.0).color(c.icon_accent).a11y_hidden()));
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
        close_choice(app);
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
}

pub(super) fn search_field(app: &App, spec: SearchField) -> El {
    let look = &app.look;
    let edit = (spec.edit)(app);
    let (key, edit_mut) = (spec.key, spec.edit_mut);
    let mut input = TextInput::new(edit)
        .label(spec.label.clone())
        .placeholder(spec.label)
        .width(spec.width)
        .on_edit(edit_mut)
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

pub(super) struct NamedSegment {
    pub(super) visible: String,
    pub(super) accessible: String,
    pub(super) active: bool,
    pub(super) on_pick: PickHandler,
}

pub(super) fn segmented_toggles(
    look: &Look,
    key_prefix: &str,
    group_label: Option<String>,
    choices: Vec<SegmentChoice>,
) -> El {
    let named = choices
        .into_iter()
        .map(|(label, active, on_pick)| NamedSegment {
            accessible: label.clone(),
            visible: label,
            active,
            on_pick,
        })
        .collect();
    segmented_toggles_named(look, key_prefix, group_label, named)
}

pub(super) fn segmented_toggles_named(
    look: &Look,
    key_prefix: &str,
    group_label: Option<String>,
    choices: Vec<NamedSegment>,
) -> El {
    let kit = &look.kit;
    let c = &kit.c;
    let count = choices.len().max(1);
    let items: Vec<El> = choices
        .into_iter()
        .enumerate()
        .map(|(idx, seg)| {
            let (fill, hover_fill, ink) = if seg.active {
                (
                    kit.alpha(c.bg_accent, 0.22),
                    kit.alpha(c.bg_accent, 0.26),
                    c.text_accent,
                )
            } else {
                (Color::TRANSPARENT, c.hover, c.text_muted)
            };
            let mut label = text(seg.visible).font(kit.ty.ui).color(ink).a11y_hidden();
            if !seg.active {
                label = label.group_color(c.text_base);
            }
            let on_pick = seg.on_pick;
            let btn = row()
                .grid_cell(idx)
                .role("button")
                .label(seg.accessible)
                .checked(seg.active)
                .min_h(28.0)
                .px(14.0)
                .py(3.0)
                .center()
                .radius(look.round(4.0))
                .fill(fill)
                .hover_group()
                .hover(hover_fill)
                .press(kit.alpha(c.bg_accent, 0.28))
                .focusable(true)
                .focus_ring(2.0, -2.0, c.border_focus)
                .cursor(Cursor::Pointer)
                .child(label)
                .on_press_tx(move |app: &mut App, tx: &mut Tx| on_pick(app, tx));
            buttons::on_return(format!("{key_prefix}-{idx}"), btn)
        })
        .collect();
    let border = kit.alpha(c.std_border, c.std_border.a * 0.45);
    let mut bar = row()
        .key(key_prefix.to_owned())
        .equal_columns(count)
        .padding(2.0)
        .radius(kit.radius.md)
        .fill(kit.alpha(c.bg_base, 0.25))
        .border(1.0, border)
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
