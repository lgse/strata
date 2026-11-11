//! The gallery page for W2 Look contract: its scenes, demo state, view and key and poll hooks.
//! Scene names start with `w2-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-0-groundwork/W2-look-contract.md`.
//!
//! Three sheets, all read from `Gallery::look`: the core tokens (colour swatches by registry
//! name, the type ramp, spacing and sizes, radii, glow, the ladders, roundness and the
//! animations switch), every semantic icon in each catalog, and the slot tokens. K3's
//! catalogs replace the placeholders behind the icon sheet with no change here.
//!
//! The slot sheet is a review scene only. Each component's work package adds slot tokens, so
//! a golden listing them would move on every such branch and collide between parallel ones;
//! a slot's values show in its own page's goldens instead.

use rune::{Color, Cx, El, KeyEvent, ScrollSpec, TextStyle, Tx, col, el, icon, row, text};
use strata_kit::icons::{self, IconName, IconSet};
use strata_kit::look::Look;
use strata_kit::slots;
use strata_kit::tokens::{self, Kind, Value};

use crate::{Gallery, Scene};

/// W2's demo state, held in `Gallery::w2`.
#[derive(Default)]
pub struct Demo {
    pub sheet: Sheet,
}

/// What the page shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sheet {
    #[default]
    Tokens,
    Icons,
    Slots,
}

fn open(g: &mut Gallery, dark: bool, sheet: Sheet) {
    g.open("w2", dark);
    g.w2.sheet = sheet;
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("w2", dark)`, then fills in `g.w2`.
pub const SCENES: &[Scene] = &[
    ("w2-tokens-light", |g| open(g, false, Sheet::Tokens)),
    ("w2-tokens-dark", |g| open(g, true, Sheet::Tokens)),
    ("w2-icons-light", |g| open(g, false, Sheet::Icons)),
    ("w2-icons-dark", |g| open(g, true, Sheet::Icons)),
];

/// Scenes for review with `--scene` only, kept out of the goldens.
/// The slot tokens are here, not in a golden: see the module doc.
pub const REVIEW: &[Scene] = &[
    ("w2-slots-light", |g| open(g, false, Sheet::Slots)),
    ("w2-slots-dark", |g| open(g, true, Sheet::Slots)),
];

pub fn view(g: &Gallery, _cx: &mut Cx<Gallery>) -> El {
    let look = g.look();
    match g.w2.sheet {
        Sheet::Tokens => tokens_sheet(look),
        Sheet::Icons => icons_sheet(look),
        Sheet::Slots => slots_sheet(look),
    }
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

/// Swatches per column: the Kit colours, then the syntax colours.
const COLOR_ROWS: usize = 20;
const SYNTAX_ROWS: usize = 6;

/// Columns of icons per catalog. The catalogs sit side by side.
const ICON_COLUMNS: usize = 4;

/// The width of a length's label, before its bar.
const LENGTH_LABEL: f32 = 130.0;

fn tokens_sheet(look: &Look) -> El {
    let s = &look.space;
    row()
        .key("w2-tokens")
        .w_full()
        .h_full()
        .padding(s.xl)
        .gap(s.xxl)
        .child(
            col()
                .gap(s.lg)
                .child(section(look, "color", swatches(look, "color.", COLOR_ROWS)))
                .child(section(
                    look,
                    "syntax",
                    swatches(look, "syntax.", SYNTAX_ROWS),
                ))
                .child(section(look, "radius", radii(look)))
                .child(section(look, "ladders", ladders(look))),
        )
        .child(
            col()
                .gap(s.lg)
                .child(section(look, "type", type_ramp(look)))
                .child(
                    row()
                        .gap(s.xl)
                        .child(section(
                            look,
                            "space",
                            lengths(look, &["space.", "content."]),
                        ))
                        .child(section(
                            look,
                            "sizes",
                            lengths(look, &["frame.", "row.", "icon_size."]),
                        )),
                )
                .child(
                    row()
                        .gap(s.xxl)
                        .child(section(look, "glow", glow(look)))
                        .child(section(look, "shape and motion", shape_and_motion(look))),
                ),
        )
}

/// A titled group of samples.
fn section(look: &Look, title: &str, body: El) -> El {
    let k = &look.kit;
    col()
        .key(format!("w2-{}", title.replace(' ', "-")))
        .gap(look.space.sm)
        .child(text(title).font(look.ty.heading).color(k.c.text_muted))
        .child(body)
}

/// A line of small text, the sheet's labels.
fn note(look: &Look, s: String) -> El {
    text(s).font(look.kit.ty.small).color(look.kit.c.text_base)
}

/// A swatch per colour token whose name starts with `prefix`, labelled with the rest of
/// its name, `per_column` to a column.
fn swatches(look: &Look, prefix: &str, per_column: usize) -> El {
    let colours: Vec<(&'static str, Color)> = tokens::all()
        .filter_map(|t| match t.get(look) {
            Value::Color(c) => t.name().strip_prefix(prefix).map(|n| (n, c)),
            _ => None,
        })
        .collect();
    row()
        .gap(look.space.lg)
        .children(colours.chunks(per_column).map(|column| {
            col()
                .gap(look.space.xxs)
                .children(column.iter().map(|&(name, c)| swatch(look, name, c)))
        }))
}

fn swatch(look: &Look, name: &str, colour: Color) -> El {
    row()
        .gap(look.space.xs)
        .items_center()
        .child(chip(look, colour))
        .child(note(look, name.to_owned()))
}

/// A small square of `colour`.
fn chip(look: &Look, colour: Color) -> El {
    let k = &look.kit;
    el().size(look.icon_size.sm, look.icon_size.sm)
        .fill(colour)
        .radius(k.radius.xs)
        .border(1.0, k.c.border_base)
}

/// A sample line in each style of Kit's ramp and Strata's.
fn type_ramp(look: &Look) -> El {
    let t = &look.kit.ty;
    let styles: [(&str, TextStyle); 14] = [
        ("title", look.ty.title),
        ("heading", look.ty.heading),
        ("body", t.body),
        ("body_medium", t.body_medium),
        ("ui", t.ui),
        ("ui_medium", t.ui_medium),
        ("ui_compact", t.ui_compact),
        ("text_small", t.text_small),
        ("text_small_medium", t.text_small_medium),
        ("text_xsmall", t.text_xsmall),
        ("text_xsmall_medium", t.text_xsmall_medium),
        ("small", t.small),
        ("small_medium", t.small_medium),
        ("mono", t.mono),
    ];
    col().children(styles.into_iter().map(|(name, style)| {
        text(format!("{name} {}/{}", style.size, style.line_height))
            .font(style)
            .color(look.kit.c.text_base)
    }))
}

/// A bar per length token under each prefix, as long as its value.
fn lengths(look: &Look, prefixes: &[&str]) -> El {
    let k = &look.kit;
    let bars = tokens::all().filter_map(|t| match t.get(look) {
        Value::Length(x) if prefixes.iter().any(|p| t.name().starts_with(p)) => Some((t.name(), x)),
        _ => None,
    });
    col().gap(look.space.xxs).children(bars.map(|(name, x)| {
        row()
            .gap(look.space.xs)
            .items_center()
            .child(note(look, format!("{name} {x}")).w(LENGTH_LABEL))
            .child(
                el().size(x, look.space.xs)
                    .fill(k.c.icon_accent)
                    .radius(k.radius.xs),
            )
    }))
}

fn radii(look: &Look) -> El {
    let k = &look.kit;
    let r = k.radius;
    let side = look.frame.control;
    row().gap(look.space.lg).children(
        [
            ("xs", r.xs),
            ("sm", r.sm),
            ("md", r.md),
            ("lg", r.lg),
            ("xl", r.xl),
        ]
        .map(|(name, radius)| {
            col()
                .gap(look.space.xxs)
                .items_center()
                .child(
                    el().size(side, side)
                        .radius(radius)
                        .fill(k.c.layer_02)
                        .border(1.0, k.c.border_strong),
                )
                .child(note(look, format!("{name} {radius}")))
        }),
    )
}

/// The tile ladder with its default, and the thumbnail size classes.
fn ladders(look: &Look) -> El {
    let list = |steps: &[f32]| {
        steps
            .iter()
            .map(f32::to_string)
            .collect::<Vec<_>>()
            .join(" ")
    };
    col()
        .gap(look.space.xxs)
        .child(note(
            look,
            format!(
                "tiles {} (default {})",
                list(look.tiles.steps()),
                look.tiles.default()
            ),
        ))
        .child(note(
            look,
            format!("thumbnails {}", list(look.thumbnails.sizes())),
        ))
}

fn glow(look: &Look) -> El {
    let g = &look.glow;
    col()
        .gap(look.space.xxs)
        .child(swatch(look, "color", g.color))
        .child(note(
            look,
            format!("on {}, radius {}, opacity {}", g.on, g.radius, g.opacity),
        ))
}

fn shape_and_motion(look: &Look) -> El {
    col()
        .gap(look.space.xxs)
        .child(note(look, format!("roundness {}", look.roundness)))
        .child(note(look, format!("animations {}", look.animations)))
}

/// The widths of the slot sheet's name and value columns.
const SLOT_NAME: f32 = 220.0;
const SLOT_VALUE: f32 = 110.0;

/// Every slot token, a section per slot, each with its name, value and kind. Scrolls when
/// the slots outgrow the window.
fn slots_sheet(look: &Look) -> El {
    let s = &look.space;
    col()
        .key("w2-slots")
        .w_full()
        .h_full()
        .min_h(0.0)
        .padding(s.xl)
        .gap(s.lg)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .children(slots::TOKENS.iter().map(|&(id, tokens)| {
            let body = if tokens.is_empty() {
                note(look, "no tokens yet".to_owned())
            } else {
                col()
                    .gap(s.xxs)
                    .children(tokens.iter().map(|t| slot_token(look, t)))
            };
            section(look, id, body)
        }))
}

fn slot_token(look: &Look, token: &tokens::Token) -> El {
    let k = &look.kit;
    let value = token.get(look);
    let shown = match value {
        Value::Color(c) => c.hex(),
        Value::Length(x) | Value::Number(x) => x.to_string(),
        Value::Flag(b) => b.to_string(),
        _ => "?".to_owned(),
    };
    let kind = match token.kind() {
        Kind::Color => "colour",
        Kind::Length => "length",
        Kind::Number => "number",
        Kind::Flag => "flag",
        _ => "?",
    };
    let mut line = row()
        .gap(look.space.xs)
        .items_center()
        .child(note(look, token.name().to_owned()).w(SLOT_NAME))
        .child(note(look, shown).w(SLOT_VALUE))
        .child(
            text(kind)
                .font(k.ty.small)
                .color(k.c.text_muted)
                .w(SLOT_VALUE),
        );
    if let Value::Color(c) = value {
        line = line.child(chip(look, c));
    }
    line
}

/// Every semantic icon, in each catalog.
fn icons_sheet(look: &Look) -> El {
    row()
        .key("w2-icons")
        .w_full()
        .h_full()
        .padding(look.space.xl)
        .gap(look.space.xxl)
        .children(icons::catalogs().into_iter().map(|set| catalog(look, set)))
}

/// A catalog's icon for each name, labelled with the name's id, in [`ICON_COLUMNS`] columns.
fn catalog(look: &Look, set: IconSet) -> El {
    let k = &look.kit;
    let cell = |&name: &IconName| {
        row()
            .gap(look.space.xs)
            .items_center()
            .child(icon((set.icon)(name), look.icon_size.md).color(k.c.icon_base))
            .child(text(name.id()).font(k.ty.small).color(k.c.text_muted))
    };
    let columns = IconName::ALL.chunks(IconName::ALL.len().div_ceil(ICON_COLUMNS));
    section(
        look,
        set.name,
        row()
            .gap(look.space.md)
            .children(columns.map(|c| col().gap(look.space.xs).children(c.iter().map(cell)))),
    )
}
