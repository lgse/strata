//! The gallery page for K3 Icon catalogs: its scenes, demo state, view and key and poll hooks.
//! Scene names start with `k3-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-1-leaves/K3-icon-catalogs.md`.
//!
//! Four sheets, each drawn from the look's catalog, as a component would:
//!
//! - `lucide`: every icon that isn't Catppuccin's (Strata's Lucide set and this repository's
//!   additions), named, in the accent.
//! - `catppuccin`: the Catppuccin file-type icons in Brand colors: Latte on the light look,
//!   Mocha on the dark one (`catppuccin::for_background`).
//! - `theme-colors`: the same icons in Theme colors (`lucide::SET`), in the accent.
//! - `weights`: a folder, Lucide's document and four Catppuccin icons at 16, 32, 64 and 128
//!   points, in both colourings, to compare stroke weight across sets and sizes.
//!
//! The `raster` review scenes fill the window with Catppuccin icons at 16 or 128 points, in
//! Brand colors (resvg colour images) or Theme colors (the alpha atlas). Their first frame's
//! CPU time, which `--snap` prints, is the cost of rasterizing that many icons:
//!
//! ```text
//! cargo run --profile perf -p strata-gallery -- --scene k3-raster-brand-large-dark --snap out.png
//! ```

use rune::{Cx, El, Icon, KeyEvent, Tx, col, el, icon, row, text};
use strata_kit::icons::{IconSet, catppuccin, lucide};
use strata_kit::look::Look;

use crate::{Gallery, Scene};

/// K3's demo state, held in `Gallery::k3`.
#[derive(Default)]
pub struct Demo {
    pub sheet: Sheet,
}

/// What the page shows.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Sheet {
    #[default]
    Lucide,
    Catppuccin,
    ThemeColors,
    Weights,
    /// Catppuccin icons at this many points, as many as fit.
    Raster(f32),
}

/// The Brand colors catalog for the gallery's appearance: the fallback look has no theme, so
/// the dark look stands for a dark background.
pub fn brand(dark: bool) -> IconSet {
    catppuccin::for_background(dark)
}

fn open(g: &mut Gallery, dark: bool, sheet: Sheet, icons: IconSet) {
    g.open("k3", dark);
    g.look.icons = icons;
    g.k3.sheet = sheet;
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("k3", dark)`, then fills in `g.k3`.
pub const SCENES: &[Scene] = &[
    ("k3-lucide-light", |g| {
        open(g, false, Sheet::Lucide, lucide::SET)
    }),
    ("k3-lucide-dark", |g| {
        open(g, true, Sheet::Lucide, lucide::SET)
    }),
    ("k3-catppuccin-light", |g| {
        open(g, false, Sheet::Catppuccin, brand(false))
    }),
    ("k3-catppuccin-dark", |g| {
        open(g, true, Sheet::Catppuccin, brand(true))
    }),
    ("k3-theme-colors-light", |g| {
        open(g, false, Sheet::ThemeColors, lucide::SET)
    }),
    ("k3-theme-colors-dark", |g| {
        open(g, true, Sheet::ThemeColors, lucide::SET)
    }),
    ("k3-weights-light", |g| {
        open(g, false, Sheet::Weights, brand(false))
    }),
    ("k3-weights-dark", |g| {
        open(g, true, Sheet::Weights, brand(true))
    }),
];

/// Scenes for review with `--scene` only, kept out of the goldens.
pub const REVIEW: &[Scene] = &[
    ("k3-raster-brand-small-light", |g| {
        open(g, false, Sheet::Raster(16.0), brand(false))
    }),
    ("k3-raster-brand-small-dark", |g| {
        open(g, true, Sheet::Raster(16.0), brand(true))
    }),
    ("k3-raster-brand-large-light", |g| {
        open(g, false, Sheet::Raster(128.0), brand(false))
    }),
    ("k3-raster-brand-large-dark", |g| {
        open(g, true, Sheet::Raster(128.0), brand(true))
    }),
    ("k3-raster-theme-small-light", |g| {
        open(g, false, Sheet::Raster(16.0), lucide::SET)
    }),
    ("k3-raster-theme-small-dark", |g| {
        open(g, true, Sheet::Raster(16.0), lucide::SET)
    }),
    ("k3-raster-theme-large-light", |g| {
        open(g, false, Sheet::Raster(128.0), lucide::SET)
    }),
    ("k3-raster-theme-large-dark", |g| {
        open(g, true, Sheet::Raster(128.0), lucide::SET)
    }),
];

pub fn view(g: &Gallery, _cx: &mut Cx<Gallery>) -> El {
    let look = g.look();
    let body = match g.k3.sheet {
        Sheet::Lucide => lucide_sheet(look),
        Sheet::Catppuccin => file_icons(look, "Brand colors"),
        Sheet::ThemeColors => file_icons(look, "Theme colors"),
        Sheet::Weights => weights(look, g.dark),
        Sheet::Raster(size) => raster(look, size),
    };
    col()
        .key("k3")
        .w_full()
        .h_full()
        .padding(look.space.xl)
        .child(body)
}

/// Keys no focused element took while this page is open: shortcuts, and keys with nothing
/// focused. True consumes the key; false lets the gallery's Escape go back to the index.
pub fn on_key(_g: &mut Gallery, _key: &KeyEvent, _tx: &mut Tx) -> bool {
    false
}

/// Runs at the start of every frame while this page is open, with the clock in seconds.
/// Deliver fake async results here; a worker wakes the loop with `Gallery::waker`. True when
/// state changed.
pub fn poll(_g: &mut Gallery, _t: f64, _tx: &mut Tx) -> bool {
    false
}

fn is_catppuccin(i: &Icon) -> bool {
    i.name.starts_with("strata-ctp-")
}

fn heading(look: &Look, s: String) -> El {
    text(s).font(look.ty.heading).color(look.kit.c.text_muted)
}

/// Columns on the Lucide sheet.
const LUCIDE_COLUMNS: usize = 5;
/// A Lucide cell's width, for the longest name.
const LUCIDE_CELL: f32 = 180.0;

fn lucide_sheet(look: &Look) -> El {
    let k = &look.kit;
    let icons: Vec<&'static Icon> = look
        .icons
        .all
        .iter()
        .filter(|i| !is_catppuccin(i))
        .collect();
    let cell = |i: &&'static Icon| {
        row()
            .w(LUCIDE_CELL)
            .gap(look.space.xs)
            .items_center()
            .child(icon(i, look.icon_size.md).color(k.c.icon_accent))
            .child(
                text(i.name.trim_start_matches("strata-"))
                    .font(k.ty.small)
                    .color(k.c.text_muted)
                    .truncate(),
            )
    };
    let per_column = icons.len().div_ceil(LUCIDE_COLUMNS);
    col()
        .gap(look.space.md)
        .child(heading(
            look,
            format!("{} · {} icons", look.icons.name, icons.len()),
        ))
        .child(
            row().gap(look.space.sm).children(
                icons
                    .chunks(per_column)
                    .map(|c| col().gap(look.space.xxs).children(c.iter().map(cell))),
            ),
        )
}

/// Catppuccin icons to a row on the file-icon sheets.
const FILE_ICON_ROW: usize = 27;

fn file_icons(look: &Look, colouring: &str) -> El {
    let icons: Vec<&'static Icon> = look.icons.all.iter().filter(|i| is_catppuccin(i)).collect();
    let title = format!(
        "{} · {colouring} · {} file icons",
        look.icons.name,
        icons.len()
    );
    col()
        .gap(look.space.md)
        .child(heading(look, title))
        .child(grid(look, &icons, look.icon_size.md, FILE_ICON_ROW))
}

/// `icons` at `size`, `per_row` to a row, tinted with the accent (brand icons ignore it).
fn grid(look: &Look, icons: &[&'static Icon], size: f32, per_row: usize) -> El {
    let gap = look.space.md;
    col().gap(gap).children(icons.chunks(per_row).map(|r| {
        row().gap(gap).children(
            r.iter()
                .map(|i| icon(i, size).color(look.kit.c.icon_accent)),
        )
    }))
}

/// Sizes on the weights sheet: List rows, then Icons-view tiles.
const WEIGHT_SIZES: [f32; 4] = [16.0, 32.0, 64.0, 128.0];
/// What the weights sheet draws: a folder and a document (Lucide) beside four Catppuccin icons.
const WEIGHT_ICONS: [&str; 6] = [
    "strata-folder",
    "strata-file-text",
    "strata-ctp-rust",
    "strata-ctp-python",
    "strata-ctp-docker",
    "strata-ctp-json",
];

fn weights(look: &Look, dark: bool) -> El {
    let colourings = [(brand(dark), "Brand colors"), (lucide::SET, "Theme colors")];
    col()
        .gap(look.space.lg)
        .children(colourings.into_iter().map(|(set, colouring)| {
            let icons: Vec<&'static Icon> = WEIGHT_ICONS
                .iter()
                .filter_map(|name| (set.named)(name))
                .collect();
            col()
                .gap(look.space.xs)
                .child(heading(look, format!("{} · {colouring}", set.name)))
                .children(WEIGHT_SIZES.map(|size| {
                    row().gap(look.space.lg).children(
                        icons
                            .iter()
                            .map(|i| icon(i, size).color(look.kit.c.icon_accent)),
                    )
                }))
        }))
}

fn raster(look: &Look, size: f32) -> El {
    let gap = look.space.md;
    let (w, h) = crate::SIZE;
    let room =
        |side: f32| (((side - 2.0 * look.space.xl + gap) / (size + gap)).floor() as usize).max(1);
    let (per_row, rows) = (room(w), room(h));
    let icons: Vec<&'static Icon> = look
        .icons
        .all
        .iter()
        .filter(|i| is_catppuccin(i))
        .take(per_row * rows)
        .collect();
    el().child(grid(look, &icons, size, per_row))
}
