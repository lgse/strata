//! The gallery page for C20d IconPicker: its scenes, demo state, view and key and poll hooks. Scene
//! names start with `c20d-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-4-integration/C20d-icon-picker.md`.
//!
//! Three layouts:
//!
//! - **Customize** stands in for Strata's Customize dialog (a C15 modal): a preview of the
//!   item's icon, Strata's 16 bundled icons inline (no search, tooltips, the current one
//!   marked), and "Choose Emoji…", which opens the bundled emoji in a popover. Escape with the
//!   popover open closes only the popover; the next closes the dialog. Strata's dialog, its
//!   colours and its Clear and Done policy are Strata's; this page keeps a log line
//!   (`c20d-log`) and a preview (`c20d-preview`) so scripts can read what was chosen.
//! - **Catalog** shows K3's catalog by name, searchable, inline and behind "Choose Icon…".
//! - **All** (review only) is the icons and the emoji in one inline picker.
//!
//! Emoji draw from the system's colour emoji font, which CI lacks and whose art moves between
//! versions, so scenes that draw emoji are review scenes: their scripts end on accessibility
//! trees, which no font changes.

use std::rc::Rc;

use rune::{Cx, El, KeyEvent, Tx, col, icon, row, text};
use rune_kit::{Button, ButtonVariant};
use strata_components::context_menu::{FieldMenu, FieldMenuState};
use strata_components::contract::Point;
use strata_components::controls::icon_picker::{
    Choice, DEFAULT_VERSION, IconPicker, IconPickerState, PickerCategory, PickerItem, PickerSource,
    PickerWords, drawn_text, remember,
};
use strata_components::dialog::DialogShell;
use strata_kit::icons::{IconName, lucide};
use strata_kit::look::Look;

use crate::{Gallery, Scene};

/// Strata's 16 customize choices (`CUSTOMIZATION_CHOICES`, `src/assets.rs` at lgse/strata
/// `upstream/main`): each icon's catalog name and its label.
pub const CHOICES: [(&str, &str); 16] = [
    ("strata-file-text", "Documents"),
    ("strata-download", "Downloads"),
    ("strata-file-code", "Code"),
    ("strata-file-archive", "Archive"),
    ("strata-image", "Pictures"),
    ("strata-video", "Videos"),
    ("strata-terminal", "Terminal"),
    ("strata-house", "Home"),
    ("strata-hard-drive", "Storage"),
    ("strata-network", "Network"),
    ("strata-monitor", "Computer"),
    ("strata-key", "Private"),
    ("strata-pin", "Pinned"),
    ("strata-play", "Media"),
    ("strata-settings", "Settings"),
    ("strata-list-checks", "Tasks"),
];

/// Which layout the page shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    #[default]
    Customize,
    Catalog,
    All,
}

/// The page's pickers, for the field menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    Emoji,
    Catalog,
    CatalogPopover,
    All,
}

/// C20d's demo state, held in `Gallery::c20d`.
pub struct Demo {
    pub layout: Layout,
    /// Strata's 16 icons, inline in the Customize stand-in.
    pub icons: IconPickerState,
    /// The bundled emoji, behind "Choose Emoji…".
    pub emoji: IconPickerState,
    /// K3's catalog, inline.
    pub catalog: IconPickerState,
    /// K3's catalog, behind "Choose Icon…".
    pub catalog_popover: IconPickerState,
    /// The icons and the emoji in one picker (review).
    pub all: IconPickerState,
    /// The Customize stand-in is open.
    pub customize: bool,
    /// The item's saved icon: what Strata writes to `[custom_icons]`.
    pub current: Option<Choice>,
    /// The recent emoji, newest first: what the host stores.
    pub recent: Vec<Choice>,
    pub fields: FieldMenuState<Which>,
    /// The last thing a picker reported: "Chose Pinned".
    pub log: String,
    /// Every choice heard, oldest first (the page's tests read it).
    pub chosen: Vec<Choice>,
    /// How many times a popover closed.
    pub closed: usize,
}

impl Default for Demo {
    fn default() -> Demo {
        let icons = Rc::new(strata_icons());
        let emoji = Rc::new(PickerSource::emoji(DEFAULT_VERSION));
        let catalog = Rc::new(PickerSource::catalog(&lucide::SET));
        let all = Rc::new(
            PickerSource::catalog(&lucide::SET).chain(PickerSource::emoji(DEFAULT_VERSION)),
        );
        let current = Some(Choice::icon("strata-pin"));
        let mut d = Demo {
            layout: Layout::Customize,
            icons: IconPickerState::new(icons),
            emoji: IconPickerState::new(emoji).id_base(100_000),
            catalog: IconPickerState::new(catalog.clone()).id_base(200_000),
            catalog_popover: IconPickerState::new(catalog).id_base(300_000),
            all: IconPickerState::new(all).id_base(400_000),
            customize: true,
            current,
            recent: Vec::new(),
            fields: FieldMenuState::new(),
            log: String::new(),
            chosen: Vec::new(),
            closed: 0,
        };
        d.sync_current();
        d
    }
}

impl Demo {
    /// The Customize pickers mark the item's saved choice. The catalog's stand alone.
    fn sync_current(&mut self) {
        let current = self.current.clone();
        for st in [&mut self.icons, &mut self.emoji, &mut self.all] {
            st.set_current(current.as_ref());
        }
    }

    pub fn state(&self, w: Which) -> &IconPickerState {
        match w {
            Which::Emoji => &self.emoji,
            Which::Catalog => &self.catalog,
            Which::CatalogPopover => &self.catalog_popover,
            Which::All => &self.all,
        }
    }

    pub fn state_mut(&mut self, w: Which) -> &mut IconPickerState {
        match w {
            Which::Emoji => &mut self.emoji,
            Which::Catalog => &mut self.catalog,
            Which::CatalogPopover => &mut self.catalog_popover,
            Which::All => &mut self.all,
        }
    }

    /// What the emoji button reads: Strata's "Choose Emoji…", or "Emoji" and the saved emoji,
    /// drawn through `drawn_text` (anomalyco/rune#102).
    pub fn emoji_label(&self) -> String {
        match &self.current {
            Some(c @ Choice::Emoji(_)) => format!("Emoji  {}", drawn_text(c)),
            _ => "Choose Emoji…".to_owned(),
        }
    }

    /// The saved choice's name, for the preview.
    pub fn current_name(&self) -> String {
        let Some(c) = &self.current else {
            return "Default".to_owned();
        };
        for st in [&self.icons, &self.emoji, &self.catalog] {
            if let Some(i) = st.source().find(c) {
                return st.source().items()[i].name.to_string();
            }
        }
        c.text().to_owned()
    }
}

/// Strata's 16 icons in one category.
pub fn strata_icons() -> PickerSource {
    PickerSource::new(
        vec![PickerCategory::new(
            "icons",
            "Icons",
            Choice::icon("strata-icons"),
        )],
        CHOICES
            .iter()
            .map(|&(name, label)| PickerItem::new(Choice::icon(name), label, 0))
            .collect(),
    )
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("c20d", dark)`, then fills in `g.c20d`. None draws an emoji.
pub const SCENES: &[Scene] = &[
    ("c20d-icons-light", |g| g.open("c20d", false)),
    ("c20d-icons-dark", |g| g.open("c20d", true)),
    ("c20d-catalog-light", |g| catalog(g, false, "")),
    ("c20d-catalog-dark", |g| catalog(g, true, "")),
    ("c20d-search-light", |g| catalog(g, false, "fold")),
    ("c20d-search-dark", |g| catalog(g, true, "fold")),
    ("c20d-no-results-light", |g| catalog(g, false, "zzzz")),
    ("c20d-no-results-dark", |g| catalog(g, true, "zzzz")),
    ("c20d-popover-light", |g| catalog_popover(g, false)),
    ("c20d-popover-dark", |g| catalog_popover(g, true)),
];

/// Scenes for review with `--scene` only, kept out of the goldens: they draw emoji.
pub const REVIEW: &[Scene] = &[
    ("c20d-emoji-light", |g| emoji(g, false, "")),
    ("c20d-emoji-dark", |g| emoji(g, true, "")),
    ("c20d-emoji-search-light", |g| emoji(g, false, "happy")),
    ("c20d-emoji-search-dark", |g| emoji(g, true, "happy")),
    ("c20d-recent-light", |g| recent(g, false)),
    ("c20d-recent-dark", |g| recent(g, true)),
    ("c20d-all-light", |g| all(g, false)),
    ("c20d-all-dark", |g| all(g, true)),
];

fn catalog(g: &mut Gallery, dark: bool, query: &str) {
    g.open("c20d", dark);
    g.c20d.layout = Layout::Catalog;
    g.c20d.catalog.set_query(query);
}

fn catalog_popover(g: &mut Gallery, dark: bool) {
    g.open("c20d", dark);
    g.c20d.layout = Layout::Catalog;
    g.c20d.catalog_popover.open();
}

/// The Customize stand-in with "Choose Emoji…" open, as a press on it leaves it.
fn emoji(g: &mut Gallery, dark: bool, query: &str) {
    g.open("c20d", dark);
    g.c20d.emoji.open();
    g.c20d.emoji.set_query(query);
}

/// Three recent emoji, then the emoji popover open on them.
fn recent(g: &mut Gallery, dark: bool) {
    g.open("c20d", dark);
    let d = &mut g.c20d;
    for e in ["🚀", "🎉", "😀"] {
        remember(&mut d.recent, Choice::emoji(e));
    }
    d.emoji.set_recent(&d.recent);
    d.emoji.open();
}

fn all(g: &mut Gallery, dark: bool) {
    g.open("c20d", dark);
    g.c20d.layout = Layout::All;
}

/// Saves a choice as Strata does on a pick: the item's icon changes at once.
fn chose(g: &mut Gallery, choice: &Choice) {
    let d = &mut g.c20d;
    d.current = Some(choice.clone());
    if choice.is_emoji() {
        remember(&mut d.recent, choice.clone());
        let recent = d.recent.clone();
        d.emoji.set_recent(&recent);
        d.all.set_recent(&recent);
    }
    d.sync_current();
    d.chosen.push(choice.clone());
    d.log = format!("Chose {}", d.current_name());
}

type Get = fn(&Gallery) -> &IconPickerState;
type GetMut = fn(&mut Gallery) -> &mut IconPickerState;

fn look(g: &Gallery) -> &Look {
    g.look()
}

/// English words, the grid named `grid` and a popover `popover`.
fn words(grid: &str, popover: &str) -> PickerWords {
    let mut w = PickerWords::default();
    w.grid = grid.into();
    w.popover = popover.into();
    w
}

/// Strata's 16, inline: no search, tooltips, 8 a row in 2 rows.
pub fn icons() -> IconPicker<Gallery> {
    IconPicker::new("c20d-icons", |g| &g.c20d.icons, |g| &mut g.c20d.icons, look)
        .search(false)
        .tooltips(true)
        .rows(2)
        .words(words("Icon", "Choose an icon"))
        .on_choose(|g: &mut Gallery, c, _| chose(g, c))
}

/// A searchable picker for `w`, with C21's text menu on its field.
pub fn picker(w: Which) -> IconPicker<Gallery> {
    let (key, get, get_mut, grid, popover): (&str, Get, GetMut, &str, &str) = match w {
        Which::Emoji => (
            "c20d-emoji",
            |g| &g.c20d.emoji,
            |g| &mut g.c20d.emoji,
            "Emoji",
            "Choose an emoji",
        ),
        Which::Catalog => (
            "c20d-catalog",
            |g| &g.c20d.catalog,
            |g| &mut g.c20d.catalog,
            "Icons",
            "Choose an icon",
        ),
        Which::CatalogPopover => (
            "c20d-catalog-popover",
            |g| &g.c20d.catalog_popover,
            |g| &mut g.c20d.catalog_popover,
            "Icons",
            "Choose an icon",
        ),
        Which::All => (
            "c20d-all",
            |g| &g.c20d.all,
            |g| &mut g.c20d.all,
            "Icons and emoji",
            "Choose an icon",
        ),
    };
    IconPicker::new(key, get, get_mut, look)
        .words(words(grid, popover))
        .on_choose(|g: &mut Gallery, c, _| chose(g, c))
        .on_close(|g: &mut Gallery, _| g.c20d.closed += 1)
        // An empty field's Escape goes on to the host: the log line says it arrived.
        .on_escape(|g: &mut Gallery, _| g.c20d.log = "Escape went on".to_owned())
        .on_menu(move |g: &mut Gallery, at: Point, tx: &mut Tx| {
            field_menu().open(g, w, at, tx);
        })
}

/// C21's text menu on the pickers' search fields.
fn field_menu() -> FieldMenu<Gallery, Which> {
    FieldMenu::new(
        "c20d-field-menu",
        |g: &Gallery| &g.c20d.fields,
        |g: &mut Gallery| &mut g.c20d.fields,
        |g: &mut Gallery, w| Some(g.c20d.state_mut(w).edit_mut()),
        |g: &Gallery, w| Some(g.c20d.state(w).edit()),
    )
    .on_change(|g: &mut Gallery, w, tx| picker(w).edited(g, tx))
}

fn heading(look: &Look, words: &str) -> El {
    text(words.to_owned())
        .font(look.ty.heading)
        .color(look.kit.c.text_muted)
}

/// The item's icon as the dialog's 56 px preview draws it, named for scripts by what it shows.
fn preview(g: &Gallery) -> El {
    let look = g.look();
    let k = &look.kit;
    let art: El = match &g.c20d.current {
        Some(c @ Choice::Emoji(_)) => text(drawn_text(c).into_owned())
            .font(rune::TextStyle::new(48.0, 400.0).lh(56.0))
            .a11y_hidden(),
        Some(Choice::Icon(name)) => icon(
            look.icon_named(name)
                .unwrap_or_else(|| look.icon(IconName::Folder)),
            56.0,
        )
        .color(k.c.icon_accent)
        .a11y_hidden(),
        None => icon(look.icon(IconName::Folder), 56.0)
            .color(k.c.icon_accent)
            .a11y_hidden(),
    };
    col()
        .key("c20d-preview")
        .role("img")
        .label(format!("Preview: {}", g.c20d.current_name()))
        .gap(look.space.xs)
        .items_center()
        .child(art)
        .child(
            text("Projects")
                .font(k.ty.ui)
                .color(k.c.text_base)
                .a11y_hidden(),
        )
}

fn customize(g: &Gallery, layer: &mut Vec<El>) -> El {
    let look = g.look();
    let k = &look.kit;
    // The popover's layer goes after the dialog's, so it sits over the dialog.
    let mut over = Vec::new();
    let label = g.c20d.emoji_label();
    let button = picker(Which::Emoji).button(
        g,
        look,
        &label,
        Some("Choose any emoji for this item"),
        &mut over,
    );
    let body = col()
        .key("c20d-customize")
        .px(look.space.xl)
        .py(look.space.xl)
        .gap(look.space.lg)
        .child(
            text("Customize Folder")
                .font(look.ty.title)
                .color(k.c.text_base),
        )
        .child(row().w_full().center().child(preview(g)))
        .child(heading(look, "ICON"))
        .child(icons().build(g, look))
        .child(button)
        .child(
            row().w_full().justify_end().child(
                Button::new("Done")
                    .variant(ButtonVariant::Submit)
                    .on_press(|g: &mut Gallery| g.c20d.customize = false)
                    .build(k)
                    .key("c20d-done"),
            ),
        );
    DialogShell::new("c20d-dialog", "Customize Folder", |g: &mut Gallery| {
        g.c20d.customize = false
    })
    .open(g.c20d.customize)
    .reduced_motion(true)
    .width(420.0)
    .child(body)
    .build(look, layer);
    layer.extend(over);
    col()
        .gap(look.space.md)
        .child(heading(look, "CUSTOMIZE STAND-IN"))
        .child(
            Button::new("Customize…")
                .variant(ButtonVariant::Neutral)
                .on_press(|g: &mut Gallery| g.c20d.customize = true)
                .build(k)
                .key("c20d-customize-open"),
        )
}

fn catalog_layout(g: &Gallery, layer: &mut Vec<El>) -> El {
    let look = g.look();
    let button = picker(Which::CatalogPopover).button(g, look, "Choose Icon…", None, layer);
    row()
        .gap(look.space.xxl)
        .items_start()
        .child(
            col()
                .gap(look.space.md)
                .child(heading(look, "ICON CATALOG"))
                .child(picker(Which::Catalog).build(g, look)),
        )
        .child(
            col()
                .gap(look.space.md)
                .child(heading(look, "IN A POPOVER"))
                .child(button),
        )
}

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let look = g.look();
    cx.reduced_motion(!look.animations);
    let k = &look.kit;
    let mut layer = Vec::new();
    let content = match g.c20d.layout {
        Layout::Customize => customize(g, &mut layer),
        Layout::Catalog => catalog_layout(g, &mut layer),
        Layout::All => picker(Which::All).build(g, look),
    };
    // Built last, so its layer sits over the popovers: an open popover's modal keeps what
    // comes after it in the tree, the menu its field opens included.
    let menu = field_menu().build(g, look, &mut layer);
    let log = text(if g.c20d.log.is_empty() {
        "\u{2014}".to_owned()
    } else {
        g.c20d.log.clone()
    })
    .key("c20d-log")
    .font(k.ty.small)
    .color(k.c.text_muted);
    rune_kit::root(
        col()
            .key("c20d")
            .w_full()
            .h_full()
            .px(look.space.xl)
            .py(look.space.xl)
            .gap(look.space.xl)
            .fill(k.c.layer_01)
            .child(text("Icon picker").font(look.ty.title).color(k.c.text_base))
            .child(content)
            .child(log)
            .child(menu),
        layer,
    )
}

/// Keys no focused element took while this page is open: shortcuts, and keys with nothing
/// focused. True consumes the key; false lets the gallery's Escape go back to the index.
pub fn on_key(_g: &mut Gallery, _key: &KeyEvent, _tx: &mut Tx) -> bool {
    false
}

/// Runs at the start of every frame while this page is open, with the clock in seconds.
/// Reports a popover a press outside closed, runs the grids' controllers, and closes the field
/// menu if its field went. True when state changed.
pub fn poll(g: &mut Gallery, _t: f64, tx: &mut Tx) -> bool {
    let mut changed = icons().poll(g, tx);
    for w in [
        Which::Emoji,
        Which::Catalog,
        Which::CatalogPopover,
        Which::All,
    ] {
        changed |= picker(w).poll(g, tx);
    }
    changed |= field_menu().check(g);
    changed
}
