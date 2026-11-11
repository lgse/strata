//! The gallery page for K4 User overrides: its scenes, demo state, view and key and poll hooks.
//! Scene names start with `k4-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-1-leaves/K4-user-overrides.md`.
//!
//! The page shows an override file applied to a bundled theme (Omarchy Light in light, Tokyo
//! Night in dark): the file with its line numbers down the left, its problems as a user reads
//! them in the middle, and on the right what the overrides reach. That is the real
//! PlacesSidebar (C8), file rows drawn from the list and collection slots, and each token the
//! file sets with the theme's value beside the override's. The migration scenes start from a
//! user stylesheet instead, and show what `migrate_css` wrote and what it left.
//!
//! `o` turns the overrides off and on again through `theme::switch`, as a reload does, so
//! the colours animate. The `k4-file-*` review scenes read any file, as Strata reads the user's
//! (an `.css` path is migrated first):
//!
//! ```text
//! STRATA_K4_FILE=~/.config/strata/overrides.toml mise run gallery -- --scene k4-file-dark
//! ```

use rune::{Color, Cx, El, Key, KeyEvent, Mods, ScrollSpec, Tx, col, el, icon, row, text};
use strata_components::sidebar::{
    Device, DeviceGroup, ItemId, Pin, Place, PlacesSidebar, SidebarData, SidebarState,
};
use strata_kit::icons::IconName;
use strata_kit::look::Look;
use strata_kit::overrides::{Migration, Overrides, Scope, migrate_css};
use strata_kit::shape::LookParams;
use strata_kit::slots;
use strata_kit::theme::{self, bundled_theme};
use strata_kit::tokens::Value;

use crate::{Gallery, Scene};

/// The page's override file: a token of each kind, slot tables, colours taken from another
/// token, both scopes, and one row of each problem a user can make.
pub const FIXTURE: &str = r##"# The gallery's overrides.toml
radius.md = 2
color.border_focus = "#ff9e64"

[sidebar.row]
height = 34

[sidebar.selected]
fill = "alpha(@color.border_focus, 0.25)"
icon = "@color.border_focus"

[list.row]
height = 32
length = 30

[collection.row]
radius = 10
inset = 12

[light]
fill.highlight = "#ffd8b8"
color.text_base = "nope"

[dark]
fill.highlight = "#5a3a20"
row.columns = "28px"
collection.cut.opacity = 1.5
"##;

/// The migration scenes' user stylesheet.
pub const STYLESHEET: &str = "/* ~/.config/strata/style.css */
.file-list row {
  margin: 1px 12px;
  border-radius: 10px;
}

.sidebar-row:hover,
.sidebar-device:hover {
  background: alpha(@strata_accent, 0.15);
}

.sidebar-row.active {
  background: #ff9e6440;
  color: @strata_text;
}

.sidebar-row.active image {
  color: #ff9e64;
}

.sidebar-shell {
  border-radius: 14px;
}

@define-color strata_accent #ff9e64;
";

/// Which file the page shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Shown {
    /// [`FIXTURE`].
    #[default]
    Overrides,
    /// [`STYLESHEET`], migrated.
    Migration,
    /// A file from `STRATA_K4_FILE`, read when the scene opens.
    File(String),
}

/// K4's demo state, held in `Gallery::k4`.
#[derive(Default)]
pub struct Demo {
    pub shown: Shown,
    /// The text on the left: the override file, or the stylesheet that was migrated.
    pub source: String,
    pub overrides: Overrides,
    /// What the stylesheet became, on a migration scene.
    pub migration: Option<Migration>,
    /// The overrides are applied; `o` toggles it.
    pub on: bool,
    /// The theme's look before the overrides, which the token list compares against.
    pub base: Look,
    pub sidebar: SidebarState,
    places: Vec<Place>,
    pins: Vec<Pin>,
    devices: Vec<DeviceGroup>,
    current: Option<ItemId>,
}

/// The scene's theme at Strata's default settings.
fn base(dark: bool) -> Look {
    let id = if dark { "tokyo-night" } else { "omarchy-light" };
    bundled_theme(id)
        .unwrap_or_else(|| panic!("{id} is bundled"))
        .look()
        .with_params(&LookParams::default())
}

fn open(g: &mut Gallery, dark: bool, shown: Shown) {
    g.open("k4", dark);
    let (source, overrides, migration) = match &shown {
        Shown::Overrides => (FIXTURE.to_owned(), Overrides::parse(FIXTURE), None),
        Shown::Migration => {
            let m = migrate_css(STYLESHEET);
            (STYLESHEET.to_owned(), Overrides::parse(&m.toml), Some(m))
        }
        Shown::File(path) => read(path),
    };
    let base = base(dark);
    g.look = overrides.applied(base.clone());
    g.dark = g.look.kit.dark;
    g.k4 = Demo {
        shown,
        source,
        overrides,
        migration,
        on: true,
        base,
        sidebar: SidebarState::new(&g.look),
        places: vec![
            Place::folder("home", "Home", IconName::Home),
            Place::folder("documents", "Documents", IconName::Documents),
            Place::folder("downloads", "Downloads", IconName::Downloads),
            Place::trash("Trash", false),
        ],
        pins: vec![Pin::new("projects", "Projects")],
        devices: vec![DeviceGroup::new(vec![Device::new(
            "system",
            "System",
            IconName::Drive,
            "/",
        )])],
        current: Some(ItemId::Place("documents".into())),
    };
}

/// The host's part, for the review scenes: read the file, as Strata does, and migrate a
/// stylesheet first. A file that can't be read is no overrides.
fn read(path: &str) -> (String, Overrides, Option<Migration>) {
    let bytes = std::fs::read(path).unwrap_or_default();
    if path.ends_with(".css") {
        let css = String::from_utf8_lossy(&bytes).into_owned();
        let m = migrate_css(&css);
        (css, Overrides::parse(&m.toml), Some(m))
    } else {
        let text = String::from_utf8_lossy(&bytes).into_owned();
        (text, Overrides::from_bytes(&bytes), None)
    }
}

fn file_scene(g: &mut Gallery, dark: bool) {
    let path = std::env::var("STRATA_K4_FILE").unwrap_or_default();
    open(g, dark, Shown::File(path));
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("k4", dark)`, then fills in `g.k4`.
pub const SCENES: &[Scene] = &[
    ("k4-overrides-light", |g| open(g, false, Shown::Overrides)),
    ("k4-overrides-dark", |g| open(g, true, Shown::Overrides)),
    ("k4-migration-light", |g| open(g, false, Shown::Migration)),
    ("k4-migration-dark", |g| open(g, true, Shown::Migration)),
];

/// Scenes for review with `--scene` only, kept out of the goldens: the file `STRATA_K4_FILE`
/// names.
pub const REVIEW: &[Scene] = &[
    ("k4-file-light", |g| file_scene(g, false)),
    ("k4-file-dark", |g| file_scene(g, true)),
];

/// Moves to the look with the overrides on or off, through the path a reload takes.
pub fn set_on(g: &mut Gallery, on: bool, tx: &mut Tx) {
    g.k4.on = on;
    let base = g.k4.base.clone();
    let next = if on {
        g.k4.overrides.applied(base)
    } else {
        base
    };
    theme::switch(&mut g.look, next, tx);
}

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let look = g.look();
    cx.reduced_motion(!look.animations);
    row()
        .key("k4")
        .w_full()
        .h_full()
        .child(source(g, look))
        .child(report(g, look))
        .child(
            col()
                .key("k4-specimen")
                .grow(1.0)
                .h_full()
                .min_w(0.0)
                .min_h(0.0)
                .padding(look.space.lg)
                .gap(look.space.lg)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(window(g, look))
                .child(token_list(g, look)),
        )
}

/// A column on the surface, with a title.
fn panel(look: &Look, key: &str, width: f32, title: String) -> El {
    let kit = &look.kit;
    col()
        .key(key.to_owned())
        .w(width)
        .h_full()
        .min_h(0.0)
        .padding(look.space.lg)
        .gap(look.space.xxs)
        .fill(kit.c.bg_deep)
        .border_sides([0.0, 1.0, 0.0, 0.0], kit.c.border_base)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            text(title)
                .font(look.ty.title)
                .color(kit.c.text_base)
                .truncate(),
        )
}

/// Lines of code with their numbers: the lines in `marked` on the danger fill.
fn lines(look: &Look, src: &str, marked: &[usize]) -> El {
    let kit = &look.kit;
    col()
        .w_full()
        .children(src.lines().enumerate().map(|(i, l)| {
            let n = i + 1;
            let mut r = row()
                .w_full()
                .gap(look.space.sm)
                .child(
                    row().w(look.space.xl + look.space.xs).child(
                        text(n.to_string())
                            .font(kit.ty.mono)
                            .color(kit.c.text_faint),
                    ),
                )
                .child(
                    text(l.to_owned())
                        .font(kit.ty.mono)
                        .color(kit.c.text_base)
                        .truncate()
                        .shrink(),
                );
            if marked.contains(&n) {
                r = r.fill(kit.c.danger.bg);
            }
            r
        }))
}

/// The file on the left, with each problem's line marked.
fn source(g: &Gallery, look: &Look) -> El {
    let d = &g.k4;
    let title = match (&d.shown, &d.migration) {
        (_, Some(_)) => "style.css",
        _ => "overrides.toml",
    };
    let marked: Vec<usize> = match &d.migration {
        Some(m) => m.unconverted.iter().map(|u| u.at.line).collect(),
        None => d.overrides.errors().iter().map(|e| e.at.line).collect(),
    };
    panel(look, "k4-source", 250.0, title.to_owned()).child(lines(look, &d.source, &marked))
}

/// The problems as Strata's settings row and log read them; for a migration, the file it
/// wrote and what it left.
fn report(g: &Gallery, look: &Look) -> El {
    let kit = &look.kit;
    let d = &g.k4;
    let message = |s: String, color: Color| text(s).font(kit.ty.small).color(color).wrap().w_full();
    let heading = |s: String| text(s).font(look.ty.heading).color(kit.c.text_muted);
    let state = if d.on { "on" } else { "off (o)" };
    let mut p = panel(look, "k4-report", 280.0, format!("Overrides {state}"));
    if let Some(m) = &d.migration {
        p = p
            .child(heading(format!("CONVERTED: {}", m.converted)))
            .child(lines(look, &m.toml, &[]))
            .child(el().h(look.space.md))
            .child(heading(format!("NOT CONVERTED: {}", m.unconverted.len())))
            .children(
                m.unconverted
                    .iter()
                    .map(|u| message(u.to_string(), kit.c.text_base)),
            )
            .child(el().h(look.space.md));
    }
    let errors = d.overrides.errors();
    p.child(heading(format!(
        "{} ROWS APPLY · {} PROBLEMS",
        d.overrides.len(),
        errors.len()
    )))
    .children(
        errors
            .iter()
            .map(|e| message(e.to_string(), kit.c.danger.fg)),
    )
}

/// The real sidebar beside file rows drawn from the list and collection slots.
fn window(g: &Gallery, look: &Look) -> El {
    let kit = &look.kit;
    let d = &g.k4;
    let sidebar = PlacesSidebar::new(
        &d.sidebar,
        |g: &mut Gallery| &mut g.k4.sidebar,
        SidebarData {
            places: &d.places,
            pins: &d.pins,
            devices: &d.devices,
            current: d.current.as_ref(),
        },
    )
    .key("k4-sidebar")
    .build(look);
    let rows: [(&str, Option<Color>, bool); 4] = [
        ("notes.md", None, false),
        ("photos (drop target)", Some(look.fill.highlight), false),
        ("budget.ods (focused)", None, true),
        ("todo.txt", None, false),
    ];
    let files = col()
        .key("k4-files")
        .grow(1.0)
        .min_w(0.0)
        .py(look.space.xs)
        .gap(look.space.xxs)
        .fill(kit.c.bg_base)
        .children(rows.into_iter().map(|(name, fill, focused)| {
            let mut r = row()
                .w_full()
                .h(slots::list::row_height(look))
                .px(look.space.md)
                .gap(look.space.md)
                .center_y()
                .radius(slots::collection::row_radius(look))
                .child(icon(look.icon(IconName::File), look.icon_size.md).color(kit.c.icon_accent))
                .child(
                    text(name.to_owned())
                        .font(kit.ty.ui_compact)
                        .color(kit.c.text_base)
                        .truncate()
                        .shrink(),
                );
            if let Some(fill) = fill {
                r = r.fill(fill);
            }
            if focused {
                // A focus glow in the focus colour, so a reload's colour motion shows on it.
                let glow = kit.c.border_focus.alpha(look.glow.opacity);
                r = r.key("k4-focused").border(2.0, kit.c.border_focus).shadow(
                    0.0,
                    look.glow.radius,
                    glow,
                );
            }
            row()
                .w_full()
                .px(slots::collection::row_inset(look))
                .child(r)
        }));
    row()
        .key("k4-window")
        .w_full()
        .h(260.0)
        .radius(kit.radius.lg)
        .border(1.0, kit.c.border_base)
        .clip_rounded(kit.radius.lg)
        .child(sidebar)
        .child(files)
}

/// Each token the file sets for this appearance: the theme's value, then the one drawn.
fn token_list(g: &Gallery, look: &Look) -> El {
    let kit = &look.kit;
    let d = &g.k4;
    let dark = look.kit.dark;
    let mut set: Vec<_> = d
        .overrides
        .tokens()
        .filter(|(scope, _)| match scope {
            Scope::All => true,
            Scope::Light => !dark,
            Scope::Dark => dark,
        })
        .map(|(_, t)| t)
        .collect();
    set.sort_by_key(|t| t.name());
    set.dedup_by_key(|t| t.name());
    let value = |v: Value| -> El {
        match v {
            Value::Color(c) => row()
                .gap(look.space.xs)
                .center_y()
                .child(
                    el().size(look.icon_size.md, look.icon_size.md)
                        .radius(kit.radius.xs)
                        .fill(c)
                        .border(1.0, kit.c.border_base),
                )
                .child(text(c.hex()).font(kit.ty.mono).color(kit.c.text_base)),
            Value::Length(x) | Value::Number(x) => text(format!("{x}"))
                .font(kit.ty.mono)
                .color(kit.c.text_base),
            Value::Flag(b) => text(b.to_string()).font(kit.ty.mono).color(kit.c.text_base),
            _ => text("?").font(kit.ty.mono).color(kit.c.text_base),
        }
    };
    col()
        .key("k4-tokens")
        .w_full()
        .gap(look.space.xxs)
        .child(
            text("SET FOR THIS APPEARANCE: THEME → OVERRIDE")
                .font(look.ty.heading)
                .color(kit.c.text_muted),
        )
        .children(set.into_iter().map(|t| {
            row()
                .w_full()
                .gap(look.space.md)
                .center_y()
                .child(
                    text(t.name())
                        .font(kit.ty.small)
                        .color(kit.c.text_base)
                        .w(190.0)
                        .truncate(),
                )
                .child(value(t.get(&d.base)))
                .child(text("→").font(kit.ty.small).color(kit.c.text_muted))
                .child(value(t.get(look)))
        }))
}

/// `o` turns the overrides off or on, animating as a reload does.
pub fn on_key(g: &mut Gallery, key: &KeyEvent, tx: &mut Tx) -> bool {
    match &key.key {
        Key::Text(t) if t.eq_ignore_ascii_case("o") && key.mods == Mods::default() => {
            let on = !g.k4.on;
            set_on(g, on, tx);
            true
        }
        _ => false,
    }
}

/// Runs at the start of every frame while this page is open, with the clock in seconds.
/// Deliver fake async results here; a worker wakes the loop with `Gallery::waker`. True when
/// state changed.
pub fn poll(_g: &mut Gallery, _t: f64, _tx: &mut Tx) -> bool {
    false
}
