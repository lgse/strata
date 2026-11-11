//! The gallery page for C20b ColorPicker: its scenes, demo state, view and key and poll hooks.
//! Scene names start with `c20b-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-1-leaves/C20b-color-picker.md`.
//!
//! The page lays the picker out as Strata uses GTK's: inline as the custom folder colour
//! editor (Strata's seven presets as the theme row, recent colours under them, and a folder
//! icon that recolours live), and as the theme editor's colour buttons, each opening the
//! picker in a popover. The translucent scene swaps the inline picker for one with alpha. What
//! each change reports shows in the log line (`c20b-log`), so scripts can tell a live drag from
//! a commit.

use rune::{Cx, El, KeyEvent, Tx, col, icon, row, text};
use strata_components::context_menu::{FieldMenu, FieldMenuState};
use strata_components::contract::Point;
use strata_components::controls::color_picker::{
    ColorChange, ColorPicker, ColorPickerState, RECENT_MAX, Srgba, Swatch, SwatchRow, remember,
};
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use crate::{Gallery, Scene};

/// The page's pickers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// The inline custom folder colour, without alpha.
    Folder,
    /// The inline picker with alpha (the translucent scenes).
    Glass,
    Background,
    Surface,
    Text,
    Accent,
}

/// The theme editor's buttons, in order.
pub const EDITOR: [Pick; 4] = [Pick::Background, Pick::Surface, Pick::Text, Pick::Accent];

const ALL: [Pick; 6] = [
    Pick::Folder,
    Pick::Glass,
    Pick::Background,
    Pick::Surface,
    Pick::Text,
    Pick::Accent,
];

impl Pick {
    fn name(self) -> &'static str {
        match self {
            Pick::Folder => "Folder color",
            Pick::Glass => "Overlay color",
            Pick::Background => "Background",
            Pick::Surface => "Surface",
            Pick::Text => "Text",
            Pick::Accent => "Accent",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Pick::Folder => "c20b-folder",
            Pick::Glass => "c20b-glass",
            Pick::Background => "c20b-background",
            Pick::Surface => "c20b-surface",
            Pick::Text => "c20b-text",
            Pick::Accent => "c20b-accent",
        }
    }
}

/// Strata's folder colour presets (`FolderColor::hex`).
pub const PRESETS: [(&str, Srgba); 7] = [
    ("Red", Srgba::rgb(0xe5, 0x48, 0x4d)),
    ("Orange", Srgba::rgb(0xf7, 0x6b, 0x15)),
    ("Yellow", Srgba::rgb(0xe5, 0xa5, 0x0a)),
    ("Green", Srgba::rgb(0x30, 0xa4, 0x6c)),
    ("Blue", Srgba::rgb(0x00, 0x90, 0xff)),
    ("Purple", Srgba::rgb(0x8e, 0x4e, 0xc6)),
    ("Gray", Srgba::rgb(0x8b, 0x8d, 0x98)),
];

/// C20b's demo state, held in `Gallery::c20b`.
pub struct Demo {
    pub folder: ColorPickerState,
    pub glass: ColorPickerState,
    pub background: ColorPickerState,
    pub surface: ColorPickerState,
    pub text: ColorPickerState,
    pub accent: ColorPickerState,
    /// The inline picker shows alpha (the translucent scenes).
    pub translucent: bool,
    /// Every picker disabled.
    pub disabled: bool,
    /// Recent colours, newest first: what the host stores.
    pub recent: Vec<Srgba>,
    pub fields: FieldMenuState<Pick>,
    /// The last change any picker reported: "Folder color Live #34d399".
    pub log: String,
    /// Every change reported, oldest first (the page's tests read it).
    pub heard: Vec<String>,
    /// What the host saved, oldest first: each Commit's colour and each Cancel's, as Strata's
    /// theme editor would write them.
    pub saved: Vec<(Pick, Srgba)>,
}

impl Default for Demo {
    fn default() -> Demo {
        let opaque = |c: Srgba| ColorPickerState::new(c);
        Demo {
            // Strata's default custom colour.
            folder: opaque(Srgba::rgb(0x34, 0xd3, 0x99)),
            // Strata's translucent accent probe, rgba(79, 214, 255, 0.5).
            glass: ColorPickerState::new(Srgba::WHITE)
                .alpha(true)
                .with_value(Srgba::rgba(0x4f, 0xd6, 0xff, 0x80)),
            background: opaque(Srgba::rgb(0x1e, 0x1e, 0x2e)),
            surface: opaque(Srgba::rgb(0x31, 0x32, 0x44)),
            text: opaque(Srgba::rgb(0xcd, 0xd6, 0xf4)),
            accent: opaque(Srgba::rgb(0x89, 0xb4, 0xfa)),
            translucent: false,
            disabled: false,
            recent: vec![
                Srgba::rgb(0xf3, 0x8b, 0xa8),
                Srgba::rgb(0xa6, 0xe3, 0xa1),
                Srgba::rgb(0x80, 0x80, 0x80),
            ],
            fields: FieldMenuState::new(),
            log: String::new(),
            heard: Vec::new(),
            saved: Vec::new(),
        }
    }
}

impl Demo {
    pub fn state(&self, p: Pick) -> &ColorPickerState {
        match p {
            Pick::Folder => &self.folder,
            Pick::Glass => &self.glass,
            Pick::Background => &self.background,
            Pick::Surface => &self.surface,
            Pick::Text => &self.text,
            Pick::Accent => &self.accent,
        }
    }

    pub fn state_mut(&mut self, p: Pick) -> &mut ColorPickerState {
        match p {
            Pick::Folder => &mut self.folder,
            Pick::Glass => &mut self.glass,
            Pick::Background => &mut self.background,
            Pick::Surface => &mut self.surface,
            Pick::Text => &mut self.text,
            Pick::Accent => &mut self.accent,
        }
    }
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("c20b", dark)`, then fills in `g.c20b`.
pub const SCENES: &[Scene] = &[
    ("c20b-opaque-light", |g| g.open("c20b", false)),
    ("c20b-opaque-dark", |g| g.open("c20b", true)),
    ("c20b-translucent-light", |g| translucent(g, false)),
    ("c20b-translucent-dark", |g| translucent(g, true)),
    ("c20b-popover-light", |g| popover(g, false)),
    ("c20b-popover-dark", |g| popover(g, true)),
    ("c20b-disabled-light", |g| disabled(g, false)),
    ("c20b-disabled-dark", |g| disabled(g, true)),
    ("c20b-invalid-light", |g| invalid(g, false)),
    ("c20b-invalid-dark", |g| invalid(g, true)),
];

/// Scenes for review with `--scene` only, kept out of the goldens.
pub const REVIEW: &[Scene] = &[];

fn translucent(g: &mut Gallery, dark: bool) {
    g.open("c20b", dark);
    g.c20b.translucent = true;
}

/// The Accent button's popover open, as a press on it leaves it.
fn popover(g: &mut Gallery, dark: bool) {
    g.open("c20b", dark);
    g.c20b.accent.open();
}

fn disabled(g: &mut Gallery, dark: bool) {
    g.open("c20b", dark);
    g.c20b.disabled = true;
}

/// The folder colour's hex field holding text that isn't a colour yet.
fn invalid(g: &mut Gallery, dark: bool) {
    g.open("c20b", dark);
    let f = &mut g.c20b.folder;
    f.edit_mut().replace_text("#34d39");
    f.edited();
}

fn heard(g: &mut Gallery, line: String) {
    g.c20b.heard.push(line.clone());
    g.c20b.log = line;
}

type Get = fn(&Gallery) -> &ColorPickerState;
type GetMut = fn(&mut Gallery) -> &mut ColorPickerState;

/// The picker for `p`, as the view and the poll both build it.
fn picker(p: Pick) -> ColorPicker<Gallery> {
    let (get, get_mut): (Get, GetMut) = match p {
        Pick::Folder => (|g| &g.c20b.folder, |g| &mut g.c20b.folder),
        Pick::Glass => (|g| &g.c20b.glass, |g| &mut g.c20b.glass),
        Pick::Background => (|g| &g.c20b.background, |g| &mut g.c20b.background),
        Pick::Surface => (|g| &g.c20b.surface, |g| &mut g.c20b.surface),
        Pick::Text => (|g| &g.c20b.text, |g| &mut g.c20b.text),
        Pick::Accent => (|g| &g.c20b.accent, |g| &mut g.c20b.accent),
    };
    ColorPicker::new(p.key(), get, get_mut)
        .label(p.name())
        .on_change(move |g: &mut Gallery, c: Srgba, change: ColorChange, _| {
            // A popover's Cancel undoes the Commits before it: the host saves it as well.
            if change != ColorChange::Live {
                g.c20b.saved.push((p, c));
            }
            heard(g, format!("{} {change:?} {c}", p.name()));
        })
        .on_close(move |g: &mut Gallery, changed, _| {
            if changed {
                let c = g.c20b.state(p).value();
                remember(&mut g.c20b.recent, c, RECENT_MAX);
            }
            heard(g, format!("{} closed, changed {changed}", p.name()));
        })
}

/// The swatch rows: Strata's presets, then the recent colours.
fn rows(g: &Gallery) -> Vec<SwatchRow> {
    vec![
        SwatchRow::new(
            "Theme",
            PRESETS
                .iter()
                .map(|&(name, c)| Swatch::new(c).label(name))
                .collect(),
        ),
        SwatchRow::new(
            "Recent",
            g.c20b.recent.iter().map(|&c| Swatch::new(c)).collect(),
        ),
    ]
}

fn field_edit_mut(g: &mut Gallery, p: Pick) -> Option<&mut rune::TextEdit> {
    (!g.c20b.disabled).then(|| g.c20b.state_mut(p).edit_mut())
}

fn field_edit(g: &Gallery, p: Pick) -> Option<&rune::TextEdit> {
    (!g.c20b.disabled).then(|| g.c20b.state(p).edit())
}

/// C21's text menu on the pickers' hex fields.
fn field_menu() -> FieldMenu<Gallery, Pick> {
    FieldMenu::new(
        "c20b-field-menu",
        |g: &Gallery| &g.c20b.fields,
        |g: &mut Gallery| &mut g.c20b.fields,
        field_edit_mut,
        field_edit,
    )
    .on_change(|g: &mut Gallery, p, _| g.c20b.state_mut(p).edited())
    .on_close(|g: &mut Gallery, p| {
        g.c20b.state_mut(p).hold(false);
        g.c20b.log = format!("{} menu closed", p.name());
    })
}

fn open_menu(p: Pick) -> impl Fn(&mut Gallery, Point, &mut Tx) -> bool + 'static {
    move |g: &mut Gallery, at, tx: &mut Tx| {
        field_menu().open(g, p, at, tx);
        g.c20b.fields.is_open()
    }
}

fn heading(look: &Look, words: &str) -> El {
    text(words.to_owned())
        .font(look.ty.heading)
        .color(look.kit.c.text_muted)
}

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let look = g.look();
    cx.reduced_motion(!look.animations);
    let k = &look.kit;
    let disabled = g.c20b.disabled;
    let mut layer = Vec::new();

    let inline = if g.c20b.translucent {
        Pick::Glass
    } else {
        Pick::Folder
    };
    let colour = g.c20b.state(inline).value();
    // Strata's custom colour dialog recolours its header icon as the colour moves.
    let preview = icon(look.icon(IconName::Folder), 56.0)
        .key("c20b-preview")
        .color(colour.to_color())
        .a11y_hidden();
    let custom = col()
        .key("c20b-custom")
        .gap(look.space.md)
        .child(heading(look, "CUSTOM FOLDER COLOR"))
        .child(
            row()
                .gap(look.space.xl)
                .items_start()
                .child(
                    picker(inline)
                        .swatches(rows(g))
                        .disabled(disabled)
                        .on_menu(open_menu(inline))
                        .build(g, look),
                )
                .child(preview),
        );

    let mut editor = col()
        .key("c20b-editor")
        .w(200.0)
        .gap(look.space.md)
        .child(heading(look, "ADD A THEME"));
    for p in EDITOR {
        let button = picker(p)
            .swatches(rows(g))
            .disabled(disabled)
            .on_menu(open_menu(p))
            .button(g, look, &mut layer);
        editor = editor.child(
            row()
                .key(format!("{}-row", p.key()))
                .w_full()
                .gap(look.space.sm)
                .items_center()
                .child(button)
                .child(
                    text(p.name())
                        .font(k.ty.ui)
                        .color(k.c.text_base)
                        .a11y_hidden(),
                ),
        );
    }

    // Built after the popovers, so its layer sits over theirs: an open popover's modal keeps
    // what comes after it in the tree, the field menu its hex field opens included.
    let menu = field_menu().build(g, look, &mut layer);

    let log = text(if g.c20b.log.is_empty() {
        "\u{2014}".to_owned()
    } else {
        g.c20b.log.clone()
    })
    .key("c20b-log")
    .font(k.ty.small)
    .color(k.c.text_muted);

    rune_kit::root(
        col()
            .key("c20b")
            .w_full()
            .h_full()
            .px(look.space.xl)
            .py(look.space.xl)
            .gap(look.space.xl)
            .fill(k.c.layer_01)
            .child(
                text("Color picker")
                    .font(look.ty.title)
                    .color(k.c.text_base),
            )
            .child(
                row()
                    .w_full()
                    .gap(look.space.xxl)
                    .items_start()
                    .child(custom)
                    .child(editor),
            )
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
/// Commits a hex field that lost the keyboard, reports a popover a press outside closed,
/// closes a disabled picker's popover, and closes the field menu if its field went. True when
/// state changed.
pub fn poll(g: &mut Gallery, _t: f64, tx: &mut Tx) -> bool {
    let mut changed = false;
    for p in ALL {
        changed |= picker(p).disabled(g.c20b.disabled).poll(g, tx);
    }
    changed |= field_menu().check(g);
    changed
}
