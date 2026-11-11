//! The gallery page for K2 Shape, density and type: its scenes, demo state, view and key and poll
//! hooks. Scene names start with `k2-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-1-leaves/K2-shape-density-type.md`.
//!
//! The page is the "look" scene: a stepper for every [`LookParams`] field down the left, and
//! a specimen drawn from the resulting look on the right (a header and tab strip, sidebar and
//! list rows, a search result, rune-kit controls, the radii, the type ramp, and a readout of
//! the tokens). Every control rebuilds the look from the gallery's theme (K1's pick, the
//! fallback until one is picked) with [`Look::with_params`]. The golden scenes are Strata's defaults and the extremes.
//!
//! The page also plays the host for the interface font: its `poll` hands `look.font` to
//! `Tx::set_font_families` when it changes, and Escape puts the app's families back before
//! the gallery returns to the index. The installed families are a fake list, since the host
//! passes them in as data. The app's own families are the bundled JetBrains Mono
//! (`strata_kit::fonts`, K6), the only face registered, so a chosen family draws in it unless
//! Rune finds the family on the machine, and no golden scene picks one.

use rune::{Cx, El, Key, KeyEvent, ScrollSpec, TextEdit, TextStyle, Tx, col, el, icon, row, text};
use rune_kit::{Button, Checkbox, Switch, TextInput};
use strata_kit::density::{self, Preset, Scale};
use strata_kit::icons::IconName;
use strata_kit::look::{Families, Look};
use strata_kit::shape::{LookParams, ROUNDNESS};
use strata_kit::typography::{self, TEXT_SIZE};
use strata_kit::{slots, typography::filter_fonts};

use crate::{Gallery, Scene};

/// The gallery's own families, as `strata_gallery::spec` passes them to `AppSpec::families`:
/// the bundled JetBrains Mono, which `LookParams::default` takes as the app's.
pub const APP_FAMILIES: (&str, &str) = strata_kit::fonts::FAMILIES;

/// The fake installed families: duplicates by case, a quoted name, a name Rune's family
/// parser can't take, and a CJK family.
const INSTALLED: &[&str] = &[
    "Inter",
    "DejaVu Sans",
    "noto sans",
    "Noto Sans",
    "Cantarell",
    "Other \"Quoted\" Sans",
    "Both \"and' Sans",
    "inter",
    "Noto Sans CJK JP",
];

/// The font picker's row for the app's own families.
const DEFAULT_LABEL: &str = "Default";

/// The desktop text scale's range and step, standing in for P5's factor.
const SCALE_STEP: f32 = 0.25;
const SCALE_RANGE: (f32, f32) = (0.5, 2.0);

/// K2's demo state, held in `Gallery::k2`.
pub struct Demo {
    pub params: LookParams,
    /// The font picker is open.
    pub picker: bool,
    pub query: TextEdit,
    /// The families `poll` last handed to Rune, or `None` before the first.
    pub sent: Option<Option<Families>>,
    /// rune-kit controls in the specimen.
    pub field: TextEdit,
    pub checked: bool,
    pub switched: bool,
}

impl Default for Demo {
    fn default() -> Demo {
        Demo {
            params: LookParams::default(),
            picker: false,
            query: TextEdit::default(),
            sent: None,
            field: TextEdit::default(),
            checked: true,
            switched: true,
        }
    }
}

/// The parameters the steppers move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Param {
    Roundness,
    Frame,
    Content,
    TextSize,
    TextScale,
}

impl Param {
    fn id(self) -> &'static str {
        match self {
            Param::Roundness => "roundness",
            Param::Frame => "frame",
            Param::Content => "content",
            Param::TextSize => "text-size",
            Param::TextScale => "text-scale",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Param::Roundness => "Corner roundness",
            Param::Frame => "Frame",
            Param::Content => "Content",
            Param::TextSize => "Text size",
            Param::TextScale => "Desktop text scale",
        }
    }

    fn value(self, p: &LookParams) -> String {
        match self {
            Param::Roundness => format!("{} %", ROUNDNESS.clamp(p.roundness)),
            Param::Frame => density_label(density::FRAME, p.frame_density),
            Param::Content => density_label(density::CONTENT, p.content_density),
            Param::TextSize => format!("{} px", TEXT_SIZE.clamp(p.text_size)),
            Param::TextScale => format!("{:.2}×", p.text_scale),
        }
    }

    fn step(self, p: &mut LookParams, delta: i32) {
        match self {
            Param::Roundness => p.roundness = ROUNDNESS.stepped(p.roundness, delta),
            Param::Frame => p.frame_density = density::FRAME.stepped(p.frame_density, delta),
            Param::Content => {
                p.content_density = density::CONTENT.stepped(p.content_density, delta)
            }
            Param::TextSize => p.text_size = typography::step_text_size(p.text_size, delta),
            Param::TextScale => {
                let v = p.text_scale + SCALE_STEP * delta as f32;
                p.text_scale = v.clamp(SCALE_RANGE.0, SCALE_RANGE.1);
            }
        }
    }
}

/// A density's value as Strata labels it: "Compact · 4 px", "Custom · 9 px".
fn density_label(scale: Scale, px: u8) -> String {
    let px = scale.clamp(px);
    let name = match scale.preset(px) {
        Some(Preset::Compact) => "Compact",
        Some(Preset::Airy) => "Airy",
        None => "Custom",
    };
    format!("{name} · {px} px")
}

/// Rebuilds the look from the gallery's theme and the page's parameters.
fn rebuild(g: &mut Gallery) {
    g.look = g.theme_look(&g.k2.params);
}

fn change(g: &mut Gallery, f: impl FnOnce(&mut LookParams)) {
    f(&mut g.k2.params);
    rebuild(g);
}

fn open(g: &mut Gallery, dark: bool, params: LookParams) {
    g.open("k2", dark);
    g.k2 = Demo {
        params,
        ..Demo::default()
    };
    rebuild(g);
}

fn defaults() -> LookParams {
    LookParams::default()
}

fn dense_small() -> LookParams {
    LookParams {
        text_size: TEXT_SIZE.min,
        frame_density: 0,
        content_density: 0,
        ..LookParams::default()
    }
}

fn airy_large() -> LookParams {
    LookParams {
        text_size: TEXT_SIZE.max,
        frame_density: density::FRAME.max,
        content_density: density::CONTENT.max,
        ..LookParams::default()
    }
}

fn roundness(percent: u16) -> LookParams {
    LookParams {
        roundness: percent,
        ..LookParams::default()
    }
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("k2", dark)`, then fills in `g.k2`.
pub const SCENES: &[Scene] = &[
    ("k2-defaults-light", |g| open(g, false, defaults())),
    ("k2-defaults-dark", |g| open(g, true, defaults())),
    ("k2-dense-small-light", |g| open(g, false, dense_small())),
    ("k2-dense-small-dark", |g| open(g, true, dense_small())),
    ("k2-airy-large-light", |g| open(g, false, airy_large())),
    ("k2-airy-large-dark", |g| open(g, true, airy_large())),
    ("k2-square-light", |g| open(g, false, roundness(0))),
    ("k2-square-dark", |g| open(g, true, roundness(0))),
    ("k2-round-light", |g| open(g, false, roundness(200))),
    ("k2-round-dark", |g| open(g, true, roundness(200))),
];

/// Scenes for review with `--scene` only, kept out of the goldens: the QA probe's crossings
/// of text size and density, and a chosen interface family (machine-dependent pixels).
pub const REVIEW: &[Scene] = &[
    ("k2-cross-large-dense-light", |g| {
        open(
            g,
            false,
            LookParams {
                text_size: TEXT_SIZE.max,
                frame_density: 0,
                content_density: 0,
                ..LookParams::default()
            },
        )
    }),
    ("k2-cross-large-dense-dark", |g| {
        open(
            g,
            true,
            LookParams {
                text_size: TEXT_SIZE.max,
                frame_density: 0,
                content_density: 0,
                ..LookParams::default()
            },
        )
    }),
    ("k2-cross-small-airy-light", |g| {
        open(
            g,
            false,
            LookParams {
                text_size: TEXT_SIZE.min,
                frame_density: density::FRAME.max,
                content_density: density::CONTENT.max,
                ..LookParams::default()
            },
        )
    }),
    ("k2-cross-small-airy-dark", |g| {
        open(
            g,
            true,
            LookParams {
                text_size: TEXT_SIZE.min,
                frame_density: density::FRAME.max,
                content_density: density::CONTENT.max,
                ..LookParams::default()
            },
        )
    }),
    ("k2-font-light", |g| {
        open(
            g,
            false,
            LookParams {
                font: Some("DejaVu Sans".into()),
                ..LookParams::default()
            },
        )
    }),
    ("k2-font-dark", |g| {
        open(
            g,
            true,
            LookParams {
                font: Some("DejaVu Sans".into()),
                ..LookParams::default()
            },
        )
    }),
];

pub fn view(g: &Gallery, _cx: &mut Cx<Gallery>) -> El {
    let look = g.look();
    // The controls keep the gallery's own size, so they stay usable at every text size.
    let ui = g.theme_look(&LookParams::default());
    row()
        .key("k2")
        .w_full()
        .h_full()
        .child(controls(g, &ui))
        .child(
            col()
                .key("k2-specimen")
                .grow(1.0)
                .h_full()
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(specimen(g, look)),
        )
}

/// The steppers, snap buttons and the font picker.
fn controls(g: &Gallery, look: &Look) -> El {
    let kit = &look.kit;
    let p = &g.k2.params;
    let mut panel = col()
        .key("k2-controls")
        .w(300.0)
        .h_full()
        .min_h(0.0)
        .padding(look.space.lg)
        .gap(look.space.md)
        .fill(kit.c.layer_01)
        .border_sides([0.0, 1.0, 0.0, 0.0], kit.c.border_muted)
        .scroll(ScrollSpec {
            stick_bottom: false,
        });
    for param in [
        Param::Roundness,
        Param::Frame,
        Param::Content,
        Param::TextSize,
        Param::TextScale,
    ] {
        panel = panel.child(stepper(look, param, p));
        if let Some(scale) = match param {
            Param::Frame => Some(density::FRAME),
            Param::Content => Some(density::CONTENT),
            _ => None,
        } {
            panel = panel.child(snaps(look, param, scale));
        }
    }
    panel.child(font_picker(g, look))
}

fn stepper(look: &Look, param: Param, p: &LookParams) -> El {
    let kit = &look.kit;
    let id = param.id();
    let value = param.value(p);
    let button = |sign: &str, delta: i32, verb: &str| {
        Button::new(sign)
            .on_press(move |g: &mut Gallery| change(g, |p| param.step(p, delta)))
            .build(kit)
            .key(format!(
                "k2-{id}-{}",
                if delta < 0 { "less" } else { "more" }
            ))
            .label(format!("{verb} {}", param.name().to_lowercase()))
    };
    col()
        .gap(look.space.xxs)
        .child(
            text(param.name())
                .font(kit.ty.small_medium)
                .color(kit.c.text_muted),
        )
        .child(
            row()
                .gap(look.space.sm)
                .center_y()
                .child(button("−", -1, "Decrease"))
                .child(
                    text(value.clone())
                        .key(format!("k2-{id}-value"))
                        .label(format!("{}: {value}", param.name()))
                        .font(kit.ty.ui)
                        .color(kit.c.text_base),
                )
                .child(button("+", 1, "Increase")),
        )
}

fn snaps(look: &Look, param: Param, scale: Scale) -> El {
    let kit = &look.kit;
    let id = param.id();
    let snap = |name: &str, px: u8| {
        Button::new(name)
            .on_press(move |g: &mut Gallery| {
                change(g, |p| match param {
                    Param::Frame => p.frame_density = px,
                    _ => p.content_density = px,
                })
            })
            .build(kit)
            .key(format!("k2-{id}-{}", name.to_lowercase()))
    };
    row()
        .gap(look.space.sm)
        .child(snap("Compact", scale.compact))
        .child(snap("Airy", scale.airy))
}

fn font_picker(g: &Gallery, look: &Look) -> El {
    let kit = &look.kit;
    let d = &g.k2;
    let current = d
        .params
        .font
        .clone()
        .unwrap_or_else(|| DEFAULT_LABEL.into());
    let mut picker = col().gap(look.space.xs).child(
        text("Interface font")
            .font(kit.ty.small_medium)
            .color(kit.c.text_muted),
    );
    picker = picker.child(
        Button::new(current.clone())
            .on_press(|g: &mut Gallery| {
                g.k2.picker = !g.k2.picker;
                g.k2.query = TextEdit::default();
                g.k2.query.focused = g.k2.picker;
            })
            .build(kit)
            .key("k2-font-open")
            .label(format!("Interface font: {current}")),
    );
    if !d.picker {
        return picker;
    }
    let choices = typography::font_choices(INSTALLED.iter().map(|s| (*s).to_owned()));
    let shown = filter_fonts(&choices, &d.query.text);
    let choose = |font: Option<String>| {
        move |g: &mut Gallery| {
            g.k2.picker = false;
            let font = font.clone();
            change(g, |p| p.font = font);
        }
    };
    // The Default row is filtered by its label like any family, as Strata's picker does.
    let default_shown = typography::font_label_matches(DEFAULT_LABEL, &d.query.text);
    let mut list = col().key("k2-font-list").gap(look.space.xxs);
    if default_shown {
        list = list.child(
            Button::new(DEFAULT_LABEL)
                .on_press(choose(None))
                .build(kit)
                .key("k2-font-default"),
        );
    }
    if shown.is_empty() && !default_shown {
        list = list.child(
            text("No fonts match")
                .key("k2-font-none")
                .font(kit.ty.ui)
                .color(kit.c.text_muted),
        );
    }
    for name in shown {
        let index = choices.iter().position(|c| c == name).unwrap_or(0);
        list = list.child(
            Button::new(name)
                .on_press(choose(Some(name.to_owned())))
                .build(kit)
                .key(format!("k2-font-{index}")),
        );
    }
    picker
        .child(
            TextInput::new(&d.query)
                .label("Search fonts")
                .placeholder("Search fonts")
                .on_edit(|g: &mut Gallery| &mut g.k2.query)
                .on_key(|g: &mut Gallery, key: &KeyEvent, _tx: &mut Tx| {
                    let close = key.key == Key::Escape;
                    if close {
                        g.k2.picker = false;
                    }
                    close
                })
                .build(kit)
                .key("k2-font-search"),
        )
        .child(list)
}

/// Everything drawn from the look, top to bottom.
fn specimen(g: &Gallery, look: &Look) -> El {
    col()
        .w_full()
        .gap(look.space.xl)
        .pb(look.space.xl)
        .child(chrome(look))
        .child(kit_controls(g, look))
        .child(
            row()
                .w_full()
                .gap(look.space.lg)
                .px(look.space.lg)
                .child(sidebar(look))
                .child(listing(look)),
        )
        .child(palette_row(look))
        .child(radii(look))
        .child(ramp(look))
        .child(readout(look))
}

/// A length for display, to at most two decimals.
fn points(x: f32) -> String {
    format!("{}", (x * 100.0).round() / 100.0)
}

/// A control-sized bar action.
fn action(look: &Look, name: IconName) -> El {
    let kit = &look.kit;
    let c = look.frame.control;
    row()
        .size(c, c)
        .center()
        .radius(kit.radius.sm)
        .fill(kit.c.layer_02)
        .child(icon(look.icon(name), look.icon_size.md).color(kit.c.icon_base))
}

/// A header bar and a tab strip, each grown by the frame inset.
fn chrome(look: &Look) -> El {
    let kit = &look.kit;
    let inset = look.frame.inset;
    let header = row()
        .key("k2-header")
        .w_full()
        .h(look.frame.bar + 2.0 * inset)
        .py(inset)
        .px(look.space.md)
        .gap(look.space.xs.max(inset))
        .center_y()
        .fill(kit.c.layer_01)
        .border_sides([0.0, 0.0, 1.0, 0.0], kit.c.border_muted)
        .child(action(look, IconName::Back))
        .child(action(look, IconName::Forward))
        .child(
            text("Projects")
                .font(look.ty.title)
                .color(kit.c.text_base)
                .truncate(),
        )
        .child(el().grow(1.0))
        .child(action(look, IconName::Search));
    let tab = |name: &str, active: bool| {
        row()
            .h_full()
            .px(look.space.lg)
            .center_y()
            .radius(kit.radius.md)
            .fill(if active {
                kit.c.layer_02
            } else {
                kit.c.layer_01
            })
            .child(
                text(name.to_owned())
                    .font(kit.ty.ui_compact)
                    .color(kit.c.text_base),
            )
    };
    let tabs = row()
        .key("k2-tabs")
        .w_full()
        .h(slots::tabs::tab_height(look))
        .py(inset)
        .px(look.space.md)
        .gap(look.space.xs)
        .fill(kit.c.layer_01)
        .child(tab("Projects", true))
        .child(tab("Downloads", false));
    col().w_full().child(header).child(tabs)
}

fn sidebar(look: &Look) -> El {
    let kit = &look.kit;
    let entry = |name: IconName, label: &str, active: bool| {
        row()
            .w_full()
            .h(slots::sidebar::row_height(look))
            .py(look.frame.inset)
            .gap(look.space.md)
            .center_y()
            .radius(kit.radius.md)
            .fill(if active {
                kit.c.layer_02
            } else {
                kit.c.layer_01
            })
            .child(
                row()
                    .size(look.frame.control, look.frame.control)
                    .center()
                    .child(icon(look.icon(name), look.icon_size.md).color(kit.c.icon_muted)),
            )
            .child(
                text(label.to_owned())
                    .font(kit.ty.ui_compact)
                    .color(kit.c.text_base)
                    .truncate(),
            )
    };
    col()
        .key("k2-sidebar")
        .w(220.0)
        .padding(look.space.xs)
        .radius(kit.radius.lg)
        .fill(kit.c.layer_01)
        .child(text("PLACES").font(look.ty.heading).color(kit.c.text_muted))
        .child(entry(IconName::Home, "Home", true))
        .child(entry(IconName::Folder, "Documents", false))
        .child(entry(IconName::Trash, "Trash", false))
}

fn listing(look: &Look) -> El {
    let kit = &look.kit;
    let pad = look.content.pad;
    let names = [
        ("notes.md", "2 KB"),
        ("budget.ods", "14 KB"),
        ("photo.raw", "24 MB"),
    ];
    col()
        .key("k2-listing")
        .grow(1.0)
        .min_w(0.0)
        .children(names.iter().enumerate().map(|(i, (name, size))| {
            row()
                .w_full()
                .h(slots::list::row_height(look))
                .py(pad)
                .px(look.space.md)
                .gap(look.space.md)
                .center_y()
                .radius(kit.radius.md)
                .fill(if i == 0 {
                    kit.c.layer_02
                } else {
                    kit.c.bg_base
                })
                .child(icon(look.icon(IconName::File), look.icon_size.md).color(kit.c.icon_muted))
                .child(
                    text((*name).to_owned())
                        .font(kit.ty.ui_compact)
                        .color(kit.c.text_base)
                        .truncate()
                        .shrink(),
                )
                .child(el().grow(1.0))
                .child(
                    text((*size).to_owned())
                        .font(kit.ty.ui_compact)
                        .color(kit.c.text_muted),
                )
        }))
}

/// A search palette result: a 32 pt thumbnail box, or the name and path lines.
fn palette_row(look: &Look) -> El {
    let kit = &look.kit;
    row()
        .key("k2-palette")
        .w_full()
        .h(look.row.palette)
        .px(18.0)
        .py(density::PALETTE_PAD)
        .gap(look.space.lg)
        .center_y()
        .border_sides([0.0, 0.0, density::PALETTE_RULE, 0.0], kit.c.border_muted)
        .child(
            row()
                .size(density::PALETTE_THUMB, density::PALETTE_THUMB)
                .center()
                .radius(kit.radius.sm)
                .fill(kit.c.layer_02)
                .child(icon(look.icon(IconName::Folder), look.icon_size.md).color(kit.c.icon_base)),
        )
        .child(
            col()
                .grow(1.0)
                .gap(density::PALETTE_LINE_GAP)
                .child(
                    text("report.pdf")
                        .font(TextStyle {
                            weight: 650.0,
                            ..kit.ty.ui_compact
                        })
                        .color(kit.c.text_base)
                        .truncate(),
                )
                .child(
                    text("/home/me/projects/report.pdf")
                        .font(kit.ty.small)
                        .color(kit.c.text_muted)
                        .truncate(),
                ),
        )
}

fn kit_controls(g: &Gallery, look: &Look) -> El {
    let kit = &look.kit;
    let d = &g.k2;
    row()
        .key("k2-kit")
        .w_full()
        .px(look.space.lg)
        .gap(look.space.lg)
        .center_y()
        .flex_wrap()
        .child(Button::new("Open").build(kit))
        .child(
            TextInput::new(&d.field)
                .placeholder("Path")
                .on_edit(|g: &mut Gallery| &mut g.k2.field)
                .build(kit)
                .w(200.0),
        )
        .child(
            Checkbox::new(d.checked)
                .label("Show sizes")
                .on_toggle(|g: &mut Gallery| g.k2.checked = !g.k2.checked)
                .build(kit),
        )
        .child(
            Switch::new(d.switched)
                .label("Element glow")
                .on_toggle(|g: &mut Gallery| g.k2.switched = !g.k2.switched)
                .build(kit),
        )
}

/// Kit's five radii, then Strata's in-between radii through `Look::round`.
fn radii(look: &Look) -> El {
    let kit = &look.kit;
    let r = kit.radius;
    let swatch = |name: String, radius: f32| {
        col()
            .gap(look.space.xxs)
            .child(
                el().size(56.0, 36.0)
                    .radius(radius)
                    .fill(kit.c.layer_02)
                    .border(1.0, kit.c.border_base),
            )
            .child(
                text(format!("{name} {}", points(radius)))
                    .font(kit.ty.small)
                    .color(kit.c.text_muted),
            )
    };
    let kit_steps = [
        ("xs", r.xs),
        ("sm", r.sm),
        ("md", r.md),
        ("lg", r.lg),
        ("xl", r.xl),
    ];
    row()
        .key("k2-radii")
        .px(look.space.lg)
        .gap(look.space.md)
        .flex_wrap()
        .children(kit_steps.map(|(n, x)| swatch(n.to_owned(), x)))
        .children([4.0, 8.0, 10.0, 12.0].map(|base| swatch(format!("{base}→"), look.round(base))))
        .child(
            col()
                .gap(look.space.xxs)
                .child(
                    el().size(36.0, 36.0)
                        .radius(18.0)
                        .fill(kit.c.layer_02)
                        .border(1.0, kit.c.border_base),
                )
                .child(text("circle").font(kit.ty.small).color(kit.c.text_muted)),
        )
        .child(
            // A pill follows the roundness, as Strata's 999px radii do.
            col()
                .gap(look.space.xxs)
                .child(
                    el().size(56.0, 20.0)
                        .radius(look.round(999.0).min(10.0))
                        .fill(kit.c.layer_02)
                        .border(1.0, kit.c.border_base),
                )
                .child(text("pill").font(kit.ty.small).color(kit.c.text_muted)),
        )
}

fn ramp(look: &Look) -> El {
    let kit = &look.kit;
    let t = kit.ty;
    let styles = [
        ("title", look.ty.title),
        ("heading", look.ty.heading),
        ("ui", t.ui),
        ("ui_compact", t.ui_compact),
        ("small", t.small),
        ("body", t.body),
        ("mono", t.mono),
    ];
    col()
        .key("k2-ramp")
        .px(look.space.lg)
        .gap(look.space.xs)
        .children(styles.map(|(name, style)| {
            text(format!(
                "{name} {}/{}",
                points(style.size),
                points(style.line_height)
            ))
            .font(style)
            .color(kit.c.text_base)
        }))
}

/// The tokens K2 sets, by registry name, as text the trees and scripts read.
fn readout(look: &Look) -> El {
    let kit = &look.kit;
    let r = kit.radius;
    let font = look
        .font
        .as_ref()
        .map_or_else(|| "app".to_owned(), |f| f.ui.clone());
    let lines = [
        ("radius-md", format!("radius.md {}", points(r.md))),
        ("roundness", format!("roundness {}", points(look.roundness))),
        (
            "type-ui",
            format!("type.ui.size {}", points(kit.ty.ui.size)),
        ),
        (
            "icon-md",
            format!("icon_size.md {}", points(look.icon_size.md)),
        ),
        (
            "frame-inset",
            format!("frame.inset {}", points(look.frame.inset)),
        ),
        ("frame-bar", format!("frame.bar {}", points(look.frame.bar))),
        (
            "frame-control",
            format!("frame.control {}", points(look.frame.control)),
        ),
        (
            "content-pad",
            format!("content.pad {}", points(look.content.pad)),
        ),
        ("row-list", format!("row.list {}", points(look.row.list))),
        (
            "row-columns",
            format!("row.columns {}", points(look.row.columns)),
        ),
        (
            "row-sidebar",
            format!("row.sidebar {}", points(look.row.sidebar)),
        ),
        (
            "row-palette",
            format!("row.palette {}", points(look.row.palette)),
        ),
        (
            "tab-height",
            format!("tabs.tab.height {}", points(slots::tabs::tab_height(look))),
        ),
        ("font", format!("font {font}")),
    ];
    col()
        .key("k2-readout")
        .px(look.space.lg)
        .gap(look.space.xxs)
        .children(lines.map(|(key, line)| {
            text(line)
                .key(format!("k2-token-{key}"))
                .font(kit.ty.mono)
                .color(kit.c.text_muted)
        }))
}

/// Keys no focused element took while this page is open: shortcuts, and keys with nothing
/// focused. True consumes the key; false lets the gallery's Escape go back to the index.
///
/// Ctrl++, Ctrl+= and Ctrl+− step the text size and Ctrl+0 resets it, as Strata binds them;
/// with Alt or Super held they do nothing. Escape closes the font picker, or else puts the
/// app's families and the fallback look back and leaves the page.
pub fn on_key(g: &mut Gallery, key: &KeyEvent, tx: &mut Tx) -> bool {
    if key.key == Key::Escape {
        if g.k2.picker {
            g.k2.picker = false;
            return true;
        }
        tx.set_font_families(APP_FAMILIES.0, APP_FAMILIES.1);
        g.k2.sent = None;
        // The gallery's Escape then puts the theme's look back for the index.
        return false;
    }
    if key.mods.alt || key.mods.logo {
        return false;
    }
    let delta = match key.key {
        Key::Command('=' | '+') => 1,
        Key::Command('-') => -1,
        Key::Command('0') => {
            change(g, |p| p.text_size = TEXT_SIZE.default);
            return true;
        }
        _ => return false,
    };
    change(g, |p| {
        p.text_size = typography::step_text_size(p.text_size, delta)
    });
    true
}

/// Runs at the start of every frame while this page is open, with the clock in seconds.
/// Deliver fake async results here; a worker wakes the loop with `Gallery::waker`. True when
/// state changed.
///
/// Plays the host for the interface font: a change of `look.font` goes to Rune.
pub fn poll(g: &mut Gallery, _t: f64, tx: &mut Tx) -> bool {
    let want = g.look.font.clone();
    if g.k2.sent.as_ref() != Some(&want) {
        match &want {
            Some(f) => tx.set_font_families(f.ui.clone(), f.mono.clone()),
            None => tx.set_font_families(APP_FAMILIES.0, APP_FAMILIES.1),
        }
        g.k2.sent = Some(want);
    }
    false
}
