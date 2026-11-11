//! The gallery page for C20c Tabbed form and settings search: its scenes, demo state, view and key
//! and poll hooks. Scene names start with `c20c-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-1-leaves/C20c-tabbed-form-settings-search.md`.
//!
//! Two demos, one at a time:
//!
//! - **Settings:** a mock settings window over Strata's own search index (ported in
//!   `settings_search::fixture`): the search field over a navigation of pages, and the shown
//!   page's sections and rows, each with a switch, or C20a's NumberField for Thumbnail workers
//!   and Text size. Escape with the field empty closes the window (`c20c-closed`).
//! - **Editor:** the custom-action editor's notebook (General, Script, Behavior) over a footer
//!   with Cancel and Save. Save with an empty Name, or with neither Files nor Folders, marks
//!   the page and moves the keyboard to the field, with Strata's error line above the tabs.

use std::collections::HashSet;

use rune::{Cx, El, KeyEvent, LiveRegion, ScrollSpec, TextEdit, Tx, col, row, text, text_input};
use rune_kit::{Button, ButtonVariant, Field as KitField, Switch};
use strata_components::context_menu::{FieldMenu, FieldMenuState};
use strata_components::controls::number_field::{NumberField, NumberFieldState};
use strata_components::controls::settings_search::fixture;
use strata_components::controls::settings_search::{
    IndexRow, Navigate, SearchIndex, SettingsSearch, SettingsSearchState,
};
use strata_components::controls::slider::Range;
use strata_components::controls::tabbed_form::{
    FormLabels, FormPage, Mounting, TabbedForm, TabbedFormState,
};
use strata_kit::look::Look;

use crate::{Gallery, Scene};

/// The page scroll view the search scrolls to its top.
pub const PAGE_SCROLL: &str = "c20c-page";

/// The page's one text field with C21's menu: the search field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchField;

/// C20c's demo state, held in `Gallery::c20c`.
pub struct Demo {
    /// The editor shows instead of the settings window.
    pub editor: bool,
    // Settings.
    pub open: bool,
    pub index: SearchIndex,
    pub search: SettingsSearchState,
    pub page: String,
    /// Switches turned on, by row key.
    pub on: HashSet<String>,
    pub workers: NumberFieldState,
    pub text_size: NumberFieldState,
    pub menu: FieldMenuState<SearchField>,
    /// Jump to the best row on the first poll (the highlight scenes).
    pub jump_on_start: bool,
    /// Every navigation the search asked for, oldest first ("about", "general > units").
    pub heard: Vec<String>,
    // Editor.
    pub tabs: TabbedFormState,
    pub mounting: Mounting,
    pub name: TextEdit,
    pub description: TextEdit,
    pub files: bool,
    pub folders: bool,
    pub confirm: bool,
    /// Pages with an invalid field at the last Save.
    pub errors: [bool; 3],
    /// Strata's error line, from the last Save.
    pub error: String,
    pub saved: usize,
    /// The clock at the last poll, in seconds.
    pub now: f64,
}

impl Default for Demo {
    fn default() -> Demo {
        Demo {
            editor: false,
            open: true,
            index: fixture::index(),
            search: SettingsSearchState::new(),
            page: "general".into(),
            on: [
                "peeking",
                "columns-sizes",
                "folder-views",
                "watch-network-folders",
                "show-thumbnails",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            workers: NumberFieldState::new(4.0, Range::new(1.0, 16.0, 1.0)),
            text_size: NumberFieldState::new(13.0, Range::new(8.0, 48.0, 1.0)).suffix(" px"),
            menu: FieldMenuState::new(),
            jump_on_start: false,
            heard: Vec::new(),
            tabs: TabbedFormState::new(),
            mounting: Mounting::Kept,
            name: TextEdit::new("Resize images"),
            description: TextEdit::new("Scale pictures to a width"),
            files: true,
            folders: false,
            confirm: true,
            errors: [false; 3],
            error: String::new(),
            saved: 0,
            now: 0.0,
        }
    }
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("c20c", dark)`, then fills in `g.c20c`.
pub const SCENES: &[Scene] = &[
    ("c20c-form-light", |g| form(g, false)),
    ("c20c-form-dark", |g| form(g, true)),
    ("c20c-search-light", |g| search(g, false, "size")),
    ("c20c-search-dark", |g| search(g, true, "size")),
    ("c20c-no-results-light", |g| search(g, false, "zzzz")),
    ("c20c-no-results-dark", |g| search(g, true, "zzzz")),
    ("c20c-highlight-light", |g| highlight(g, false)),
    ("c20c-highlight-dark", |g| highlight(g, true)),
];

/// Scenes for review with `--scene` only, kept out of the goldens.
pub const REVIEW: &[Scene] = &[
    ("c20c-settings-light", |g| g.open("c20c", false)),
    ("c20c-settings-dark", |g| g.open("c20c", true)),
    ("c20c-editor-light", |g| editor(g, false, Mounting::Kept)),
    ("c20c-editor-dark", |g| editor(g, true, Mounting::Kept)),
    ("c20c-editor-rebuilt-light", |g| {
        editor(g, false, Mounting::Rebuilt)
    }),
    ("c20c-editor-rebuilt-dark", |g| {
        editor(g, true, Mounting::Rebuilt)
    }),
    ("c20c-highlight-animated-light", |g| {
        highlight_animated(g, false)
    }),
    ("c20c-highlight-animated-dark", |g| {
        highlight_animated(g, true)
    }),
];

fn editor(g: &mut Gallery, dark: bool, mounting: Mounting) {
    g.open("c20c", dark);
    g.c20c.editor = true;
    g.c20c.mounting = mounting;
}

/// After a Save with no name, looking at Behavior: General is marked.
fn form(g: &mut Gallery, dark: bool) {
    editor(g, dark, Mounting::Kept);
    g.c20c.name = TextEdit::new("");
    validate(&mut g.c20c);
    g.c20c.tabs.select(2);
}

fn search(g: &mut Gallery, dark: bool, query: &str) {
    g.open("c20c", dark);
    g.c20c.search.set_text(query);
}

/// After "thumbnail workers" and Enter, animations off so the highlight holds still.
fn highlight(g: &mut Gallery, dark: bool) {
    search(g, dark, "thumbnail workers");
    g.look.animations = false;
    g.c20c.jump_on_start = true;
}

/// The highlight with its fade (animations on).
fn highlight_animated(g: &mut Gallery, dark: bool) {
    highlight(g, dark);
    g.look.animations = true;
}

fn settings() -> SettingsSearch<Gallery> {
    SettingsSearch::new(
        "c20c-search",
        |g: &Gallery| &g.c20c.search,
        |g: &mut Gallery| &mut g.c20c.search,
        |g: &Gallery| &g.c20c.index,
    )
    .page_scroll(PAGE_SCROLL)
    .on_navigate(|g: &mut Gallery, to: Navigate, _| {
        g.c20c.heard.push(match &to.row {
            Some(r) => format!("{} > {r}", to.page),
            None => to.page.clone(),
        });
        g.c20c.page = to.page;
    })
    .on_menu(|g: &mut Gallery, at, tx| field_menu().open(g, SearchField, at, tx))
    .on_escape(|g: &mut Gallery, _| close(g))
}

/// Strata's Escape past an empty search field: Settings closes.
fn close(g: &mut Gallery) -> bool {
    std::mem::replace(&mut g.c20c.open, false)
}

/// C21's text menu on the search field; its edits search again.
fn field_menu() -> FieldMenu<Gallery, SearchField> {
    FieldMenu::new(
        "c20c-field-menu",
        |g: &Gallery| &g.c20c.menu,
        |g: &mut Gallery| &mut g.c20c.menu,
        |g: &mut Gallery, _| Some(g.c20c.search.edit_mut()),
        |g: &Gallery, _| Some(g.c20c.search.edit()),
    )
    .on_change(|g: &mut Gallery, _, tx| {
        settings().refresh(g, tx);
    })
}

fn workers() -> NumberField<Gallery> {
    NumberField::new(
        "c20c-workers",
        |g: &Gallery| &g.c20c.workers,
        |g: &mut Gallery| &mut g.c20c.workers,
    )
}

fn text_size() -> NumberField<Gallery> {
    NumberField::new(
        "c20c-text-size",
        |g: &Gallery| &g.c20c.text_size,
        |g: &mut Gallery| &mut g.c20c.text_size,
    )
}

/// The section headings, by key (Strata's are upper case).
fn section_title(key: &str) -> &'static str {
    match key {
        "language-units" => "LANGUAGE & UNITS",
        "startup" => "STARTUP",
        "browsing" => "BROWSING",
        "search" => "SEARCH & FILTERING",
        "performance" => "PERFORMANCE",
        "window" => "WINDOW BUTTONS",
        "text" => "TEXT",
        _ => "",
    }
}

fn shown_title(r: &IndexRow) -> String {
    r.title
        .clone()
        .unwrap_or_else(|| r.keywords.split(' ').next().unwrap_or_default().to_owned())
}

fn control(g: &Gallery, look: &Look, r: &IndexRow, title: &str) -> El {
    match r.key.as_str() {
        "thumbnail-workers" => workers().label(title).build(g, look),
        "text" => text_size().label(title).build(g, look),
        _ if r.group.is_some() => {
            // A Keyboard row shows its keys (the words after the action's names).
            let keys = fixture::SHORTCUTS
                .iter()
                .find(|(k, ..)| *k == r.key)
                .map_or("", |s| s.2);
            text(keys.to_owned())
                .font(look.kit.ty.small)
                .color(look.kit.c.text_muted)
        }
        _ => {
            let key = r.key.clone();
            Switch::new(g.c20c.on.contains(&r.key))
                .label(title)
                .hide_label()
                .on_toggle(move |g: &mut Gallery| {
                    if !g.c20c.on.remove(&key) {
                        g.c20c.on.insert(key.clone());
                    }
                })
                .build(&look.kit)
                .key(format!("c20c-switch-{}", r.key))
        }
    }
}

fn setting_row(
    g: &Gallery,
    look: &Look,
    search: &SettingsSearch<Gallery>,
    r: &IndexRow,
) -> Option<El> {
    let k = &look.kit;
    let title = shown_title(r);
    let content = row()
        .w_full()
        .min_w(0.0)
        .px(look.space.md)
        .py(look.space.sm)
        .gap(look.space.lg)
        .items_center()
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .child(search.title(g, look, &title, k.ty.ui)),
        )
        .child(control(g, look, r, &title));
    search.row(g, look, &r.key, content)
}

fn page_body(g: &Gallery, look: &Look, search: &SettingsSearch<Gallery>) -> El {
    let k = &look.kit;
    let d = &g.c20c;
    if d.search.no_results() {
        return search.empty(look);
    }
    let mut body = col().w_full().gap(look.space.xs);
    let rows: Vec<&IndexRow> = d.index.rows.iter().filter(|r| r.page == d.page).collect();
    // Sections in the order they first appear, each with its rows (Strata's order interleaves
    // them).
    let mut sections: Vec<Option<&str>> = Vec::new();
    for r in &rows {
        if !sections.contains(&r.section.as_deref()) {
            sections.push(r.section.as_deref());
        }
    }
    for section in sections {
        if let Some(s) = section.filter(|s| search.shows_section(g, s)) {
            body = body.child(
                text(section_title(s))
                    .key(format!("c20c-section-{s}"))
                    .font(look.ty.heading)
                    .color(k.c.text_muted)
                    .pt(look.space.md),
            );
        }
        for r in rows.iter().filter(|r| r.section.as_deref() == section) {
            if let Some(e) = setting_row(g, look, search, r) {
                body = body.child(e);
            }
        }
    }
    body
}

fn settings_view(
    g: &Gallery,
    look: &Look,
    search: &SettingsSearch<Gallery>,
    layer: &mut Vec<El>,
) -> El {
    let k = &look.kit;
    let d = &g.c20c;
    if !d.open {
        return text("Settings closed")
            .key("c20c-closed")
            .font(k.ty.ui)
            .color(k.c.text_muted);
    }
    let nav = col()
        .key("c20c-nav")
        .role("tablist")
        .label("Settings pages")
        .focus_group()
        .w_full()
        .gap(look.space.xxs)
        .children(
            fixture::PAGES
                .iter()
                .filter(|(key, _)| search.shows_page(g, key))
                .map(|(key, title)| {
                    let chosen = d.page == *key;
                    let page = key.to_string();
                    let e = row()
                        .key(format!("c20c-nav-{key}"))
                        .role("tab")
                        .label(*title)
                        .focusable(true)
                        .focus_selected(chosen)
                        .w_full()
                        .h(look.frame.control)
                        .px(look.space.md)
                        .items_center()
                        .radius(k.radius.md)
                        .hover(k.c.hover)
                        .on_press(move |g: &mut Gallery| g.c20c.page = page.clone())
                        .child(
                            text(*title)
                                .font(k.ty.ui)
                                .color(if chosen {
                                    k.c.text_base
                                } else {
                                    k.c.text_muted
                                })
                                .a11y_hidden(),
                        );
                    if chosen { e.fill(k.c.layer_02) } else { e }
                }),
        );
    let title = if d.search.no_results() {
        search.results_title()
    } else {
        fixture::PAGES
            .iter()
            .find(|(key, _)| *key == d.page)
            .map_or("", |p| p.1)
    };
    let side = col()
        .key("c20c-side")
        .w(210.0)
        .gap(look.space.md)
        .child(search.field(g, look))
        .child(nav);
    let main = col()
        .key("c20c-main")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .gap(look.space.md)
        .child(
            text(title)
                .key("c20c-title")
                .font(look.ty.title)
                .color(k.c.text_base),
        )
        .child(
            col()
                .key(PAGE_SCROLL)
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(page_body(g, look, search)),
        );
    let menu = field_menu().build(g, look, layer);
    row()
        .key("c20c-settings")
        .w_full()
        .h_full()
        .min_h(0.0)
        .gap(look.space.xl)
        .padding(look.space.xl)
        .radius(k.radius.lg)
        .fill(k.c.bg_base)
        .child(side)
        .child(main)
        .child(menu)
}

fn form_view() -> TabbedForm<Gallery> {
    TabbedForm::new(
        "c20c-editor",
        |g: &Gallery| &g.c20c.tabs,
        |g: &mut Gallery| &mut g.c20c.tabs,
    )
    .labels(FormLabels {
        tabs: "Action editor sections".into(),
        ..FormLabels::default()
    })
}

fn form_pages(d: &Demo) -> [FormPage; 3] {
    [
        FormPage::new("general", "General").errors(d.errors[0]),
        FormPage::new("script", "Script").errors(d.errors[1]),
        FormPage::new("behavior", "Behavior").errors(d.errors[2]),
    ]
}

/// Strata's checks on Save, in its order: the name, then the kinds. Returns the first invalid
/// field's page and key.
fn validate(d: &mut Demo) -> Option<(usize, &'static str)> {
    d.errors = [d.name.text.trim().is_empty(), false, !d.files && !d.folders];
    let first = if d.errors[0] {
        d.error = "Enter a name".into();
        Some((0, "c20c-name"))
    } else if d.errors[2] {
        d.error = "Choose Files, Folders, or both".into();
        Some((2, "c20c-files"))
    } else {
        d.error.clear();
        None
    };
    if first.is_none() {
        d.saved += 1;
    }
    first
}

fn text_field(
    look: &Look,
    key: &str,
    label: &str,
    edit: &TextEdit,
    access: fn(&mut Gallery) -> &mut TextEdit,
) -> El {
    let k = &look.kit;
    let input = text_input(edit, k.ty.ui)
        .single_line()
        .key(key.to_owned())
        .label(label.to_owned())
        .color(k.c.text_base)
        .caret(k.c.text_base, k.selection())
        .w_full()
        .on_edit(access);
    let boxed = row()
        .w_full()
        .h(look.frame.control)
        .px(look.space.sm)
        .items_center()
        .radius(k.radius.md)
        .fill(k.c.bg_base)
        .child(input);
    KitField::new()
        .label(label)
        .control(k.elevate(boxed, k.elevation.input()))
        .build(k)
}

fn switch_row(look: &Look, key: &str, label: &str, on: bool, flip: fn(&mut Demo)) -> El {
    let k = &look.kit;
    row()
        .w_full()
        .gap(look.space.lg)
        .items_center()
        .child(
            text(label.to_owned())
                .font(k.ty.ui)
                .color(k.c.text_base)
                .grow(1.0)
                .a11y_hidden(),
        )
        .child(
            Switch::new(on)
                .label(label)
                .hide_label()
                .on_toggle(move |g: &mut Gallery| flip(&mut g.c20c))
                .build(k)
                .key(key.to_owned()),
        )
}

fn editor_page(g: &Gallery, look: &Look, i: usize) -> El {
    let k = &look.kit;
    let d = &g.c20c;
    let body = col().w_full().gap(look.space.xl).padding(look.space.xl);
    match i {
        0 => body
            .child(text_field(look, "c20c-name", "Name", &d.name, |g| {
                &mut g.c20c.name
            }))
            .child(text_field(
                look,
                "c20c-description",
                "Description",
                &d.description,
                |g| &mut g.c20c.description,
            )),
        1 => body.child(
            col()
                .key("c20c-script")
                .w_full()
                .h(220.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .radius(k.radius.md)
                .fill(k.c.bg_base)
                .padding(look.space.md)
                .child(col().key("c20c-script-lines").children((1..=40).map(|n| {
                    text(format!("{n:>2}  convert \"$f\" -resize 1200x \"$f\""))
                        .font(look.kit.ty.mono)
                        .color(k.c.text_base)
                }))),
        ),
        _ => body
            .child(switch_row(look, "c20c-files", "Files", d.files, |d| {
                d.files ^= true
            }))
            .child(switch_row(
                look,
                "c20c-folders",
                "Folders",
                d.folders,
                |d| d.folders ^= true,
            ))
            .child(switch_row(
                look,
                "c20c-confirm",
                "Confirm before running",
                d.confirm,
                |d| d.confirm ^= true,
            )),
    }
}

fn editor_view(g: &Gallery, look: &Look) -> El {
    let k = &look.kit;
    let d = &g.c20c;
    let form = form_view().mounting(d.mounting);
    let pages = form_pages(d);
    let footer = row()
        .key("c20c-footer")
        .w_full()
        .gap(look.space.md)
        .justify_end()
        .child(
            Button::new("Cancel")
                .variant(ButtonVariant::Ghost)
                .build(k)
                .key("c20c-cancel"),
        )
        .child(
            Button::new("Save changes")
                .variant(ButtonVariant::Submit)
                .on_press_tx(|g: &mut Gallery, tx: &mut Tx| {
                    if let Some((page, field)) = validate(&mut g.c20c) {
                        g.c20c.tabs.show_error(page, field, tx);
                    }
                })
                .build(k)
                .key("c20c-save"),
        );
    let error = text(d.error.clone())
        .key("c20c-error")
        .role("status")
        .live_region(LiveRegion::Polite)
        .font(k.ty.ui)
        .color(k.c.danger.fg);
    col()
        .key("c20c-editor-dialog")
        .w(620.0)
        .gap(look.space.md)
        .padding(look.space.xl)
        .radius(k.radius.lg)
        .fill(k.c.bg_base)
        .child(text("Edit action").font(look.ty.title).color(k.c.text_base))
        .child(error)
        .child(form.build(g, look, &pages, |g, i| editor_page(g, look, i)))
        .child(footer)
}

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let look = g.look();
    cx.reduced_motion(!look.animations);
    let k = &look.kit;
    let mut layer = Vec::new();
    let content = if g.c20c.editor {
        editor_view(g, look)
    } else {
        settings_view(g, look, &settings(), &mut layer)
    };
    rune_kit::root(
        col()
            .key("c20c")
            .w_full()
            .h_full()
            .padding(look.space.xl)
            .fill(k.c.bg_deep)
            .items_center()
            .child(content),
        layer,
    )
}

/// Keys no focused element took while this page is open: shortcuts, and keys with nothing
/// focused. True consumes the key; false lets the gallery's Escape go back to the index.
/// Escape with nothing focused (or from the search field's `on_escape`) closes Settings;
/// Ctrl+Page Up and Down switch the editor's pages.
pub fn on_key(g: &mut Gallery, key: &KeyEvent, tx: &mut Tx) -> bool {
    if g.c20c.menu.is_open() {
        return false;
    }
    if g.c20c.editor {
        return form_view().window_key(g, 3, key, tx);
    }
    key.key == rune::Key::Escape && close(g)
}

/// Runs at the start of every frame while this page is open, with the clock in seconds. Runs the
/// search (and a scene's first jump), the form's carry of the keyboard and the number fields' blur
/// commits. True when state changed.
pub fn poll(g: &mut Gallery, t: f64, tx: &mut Tx) -> bool {
    g.c20c.now = t;
    let search = settings();
    let mut changed = search.tick(g, t, tx);
    if std::mem::take(&mut g.c20c.jump_on_start) {
        changed |= search.jump(g, tx);
    }
    changed |= g.c20c.tabs.tick(tx);
    changed |= workers().poll(g, tx);
    changed |= text_size().poll(g, tx);
    changed |= field_menu().check(g);
    changed
}
