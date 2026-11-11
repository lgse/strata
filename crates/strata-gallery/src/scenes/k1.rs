//! The gallery page for K1 Theme mapping: its scenes, demo state, view and key and poll hooks.
//! Scene names start with `k1-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-1-leaves/K1-theme-mapping.md`.
//!
//! The page shows one theme's look: its fourteen colours down the left, and on the right a
//! specimen drawn only from the look's tokens: a window's header, tabs, sidebar and file
//! rows (rest, hover, selected, drop target), a menu, a dialog with the element glow,
//! rune-kit controls, the danger and info roles, and highlighted code. Every look is the
//! theme's at Strata's default shape, density and type.
//!
//! Under the colours, a searchable list of every bundled theme, with Neutral (rune-kit's
//! fallback kit) first, picks the theme the whole gallery wears ([`Gallery::theme`]): the
//! index and every page draw in it until another is picked. Up and Down step to the previous
//! or next bundled theme (or, on an Omarchy scene, the previous or next variant), animating
//! the switch. The `k1-theme-*` review scenes open on `STRATA_K1_THEME=<id>`, for a `--snap`
//! of any theme by name:
//!
//! ```text
//! STRATA_K1_THEME=dracula mise run gallery -- --snap out.png --scene k1-theme-dark
//! ```

use rune::{
    Color, Cursor, Cx, El, Key, KeyEvent, ScrollSpec, TextEdit, Tx, col, el, icon, rich, row, text,
};
use rune_kit::{Button, ButtonVariant, Checkbox, State, StateColors, Switch, TextInput};
use strata_kit::icons::IconName;
use strata_kit::look::Look;
use strata_kit::shape::LookParams;
use strata_kit::slots;
use strata_kit::theme::omarchy::{self, Variant};
use strata_kit::theme::{self, Key as Role, Theme, bundled, bundled_theme};

use crate::{Gallery, Scene};

/// Which theme the page shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shown {
    /// A bundled theme, by id.
    Bundled(&'static str),
    /// An Omarchy theme from one of the page's Quattro fixtures, in a variant.
    Omarchy { light: bool, variant: Variant },
}

/// K1's demo state, held in `Gallery::k1`. The theme it shows is the gallery's,
/// [`Gallery::theme`].
#[derive(Default)]
pub struct Demo {
    /// The theme list's search.
    pub query: TextEdit,
    /// The theme the list last scrolled to, so it scrolls once per change.
    pub revealed: Option<Option<Shown>>,
}

/// A dark Quattro `colors.toml`, invented for the page.
const EMBER: &str = r##"mode = "dark"
background = "#14161b"
dark_background = "#0f1115"
lighter_background = "#1d2027"
foreground = "#d4d7de"
accent = "#e0a458"
selection = "#3a3226"
muted = "#4a4f5a"
color1 = "#e06c75"
magenta = "#c678dd"
green = "#98c379"
orange = "#d19a66"
cyan = "#56b6c2"
yellow = "#e5c07b"
blue = "#61afef"
"##;

/// A light Quattro `colors.toml`, invented for the page.
const PAPER: &str = r##"mode = "light"
background = "#f5f3ee"
dark_background = "#e9e5dc"
lighter_background = "#eeebe4"
foreground = "#2e2b26"
accent = "#2f6fb0"
selection = "#cfdcea"
muted = "#b9b2a5"
color1 = "#b3261e"
magenta = "#8e3fa1"
green = "#3d7a2a"
orange = "#b0581c"
cyan = "#1d7a80"
yellow = "#8a6d00"
blue = "#2f6fb0"
"##;

/// The theme `shown` names.
pub fn theme_of(shown: Shown) -> Theme {
    match shown {
        Shown::Bundled(id) => bundled_theme(id)
            .unwrap_or_else(|| panic!("{id} is bundled"))
            .clone(),
        Shown::Omarchy { light, variant } => {
            let (name, colors) = if light {
                ("paper", PAPER)
            } else {
                ("ember", EMBER)
            };
            omarchy::theme(name, colors, variant).unwrap_or_else(|| panic!("{name} maps"))
        }
    }
}

fn open(g: &mut Gallery, dark: bool, shown: Shown) {
    g.open("k1", dark);
    g.k1 = Demo::default();
    g.theme = Some(shown);
    g.look = g.theme_look(&LookParams::default());
    g.dark = g.look.kit.dark;
}

/// Makes `shown` the gallery's theme (`None`: Neutral), animating the colours. Neutral keeps
/// the current appearance, light or dark.
fn show(g: &mut Gallery, shown: Option<Shown>, tx: &mut Tx) {
    g.theme = shown;
    let next = g.theme_look(&LookParams::default());
    theme::switch(&mut g.look, next, tx);
    g.dark = g.look.kit.dark;
}

/// The review scenes' theme: `STRATA_K1_THEME`, else `fallback`.
fn named(fallback: &'static str) -> Shown {
    let wanted = std::env::var("STRATA_K1_THEME").ok();
    let id = wanted
        .and_then(|id| bundled().iter().find(|t| t.id == id))
        .map_or(fallback, |t| t.id.as_str());
    Shown::Bundled(id)
}

fn bundled_scene(g: &mut Gallery, dark: bool, id: &'static str) {
    open(g, dark, Shown::Bundled(id));
}

fn omarchy_scene(g: &mut Gallery, dark: bool, variant: Variant) {
    let shown = Shown::Omarchy {
        light: !dark,
        variant,
    };
    open(g, dark, shown);
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("k1", dark)`, then fills in `g.k1`. Three representative pairs (owner ruling
/// Q12): Catppuccin, whose Mocha surface sits below its background; Gruvbox, a warm Base16
/// palette; and Strata's own Omarchy Light and Azure Glow.
pub const SCENES: &[Scene] = &[
    ("k1-catppuccin-light", |g| {
        bundled_scene(g, false, "catppuccin-latte")
    }),
    ("k1-catppuccin-dark", |g| {
        bundled_scene(g, true, "catppuccin-mocha")
    }),
    ("k1-gruvbox-light", |g| {
        bundled_scene(g, false, "gruvbox-light-hard")
    }),
    ("k1-gruvbox-dark", |g| {
        bundled_scene(g, true, "gruvbox-dark-hard")
    }),
    ("k1-curated-light", |g| {
        bundled_scene(g, false, "omarchy-light")
    }),
    ("k1-curated-dark", |g| bundled_scene(g, true, "azure-glow")),
];

/// Scenes for review with `--scene` only, kept out of the goldens: any bundled theme by
/// name, and the Omarchy variants on a light and a dark Quattro palette.
pub const REVIEW: &[Scene] = &[
    ("k1-theme-light", |g| {
        let shown = named("omarchy-light");
        open(g, false, shown)
    }),
    ("k1-theme-dark", |g| {
        let shown = named(theme::DEFAULT_ID);
        open(g, true, shown)
    }),
    ("k1-omarchy-lighter-light", |g| {
        omarchy_scene(g, false, Variant::Lighter)
    }),
    ("k1-omarchy-lighter-dark", |g| {
        omarchy_scene(g, true, Variant::Lighter)
    }),
    ("k1-omarchy-darker-light", |g| {
        omarchy_scene(g, false, Variant::Darker)
    }),
    ("k1-omarchy-darker-dark", |g| {
        omarchy_scene(g, true, Variant::Darker)
    }),
    ("k1-omarchy-contrast-light", |g| {
        omarchy_scene(g, false, Variant::Contrast)
    }),
    ("k1-omarchy-contrast-dark", |g| {
        omarchy_scene(g, true, Variant::Contrast)
    }),
];

pub fn view(g: &Gallery, _cx: &mut Cx<Gallery>) -> El {
    let look = g.look();
    let theme = g.theme.map(theme_of);
    row()
        .key("k1")
        .w_full()
        .h_full()
        .child(roles(g, look, theme.as_ref()))
        .child(
            col()
                .key("k1-specimen")
                .grow(1.0)
                .h_full()
                .min_w(0.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(specimen(look)),
        )
}

/// The theme's name and colours, then the theme list under them.
fn roles(g: &Gallery, look: &Look, theme: Option<&Theme>) -> El {
    let kit = &look.kit;
    let source = match g.theme {
        Some(Shown::Bundled(_)) => "Bundled".to_owned(),
        Some(Shown::Omarchy { variant, .. }) => format!("Omarchy, {variant:?}"),
        None => "rune-kit's fallback kit".to_owned(),
    };
    let appearance = if kit.dark { "dark" } else { "light" };
    let name = theme.map_or_else(|| NEUTRAL.to_owned(), |t| t.name.clone());
    let mut colours = col()
        .key("k1-colours")
        .w_full()
        .gap(look.space.xxs)
        .child(
            text(name)
                .key("k1-theme-name")
                .font(look.ty.title)
                .color(kit.c.text_base),
        )
        .child(
            text(format!("{source} · {appearance}"))
                .font(kit.ty.small)
                .color(kit.c.text_muted),
        )
        .child(el().h(look.space.sm));
    for key in Role::ALL {
        let value = theme.and_then(|t| t.colors.get(key));
        let label = value.map_or_else(|| "derived".to_owned(), |c| c.hex());
        colours = colours.child(
            row()
                .gap(look.space.sm)
                .center_y()
                .child(
                    el().size(look.icon_size.md, look.icon_size.md)
                        .radius(kit.radius.xs)
                        .fill(value.map_or(kit.c.layer_02, |c| c.color()))
                        .border(1.0, kit.c.border_base),
                )
                .child(
                    text(format!("{} {label}", key.name()))
                        .font(kit.ty.small)
                        .color(kit.c.text_base),
                ),
        );
    }
    col()
        .key("k1-roles")
        .w(270.0)
        .h_full()
        .min_h(0.0)
        .padding(look.space.lg)
        .gap(look.space.md)
        .fill(kit.c.bg_deep)
        .border_sides([0.0, 1.0, 0.0, 0.0], kit.c.border_base)
        .child(colours)
        .child(picker(g, look))
}

/// The theme list's first entry: no theme, rune-kit's fallback kit.
const NEUTRAL: &str = "Neutral";

/// The theme list's row for `shown`; an Omarchy theme has none.
fn pick_key(shown: Option<Shown>) -> Option<String> {
    match shown {
        None => Some("k1-pick-neutral".to_owned()),
        Some(Shown::Bundled(id)) => Some(format!("k1-pick-{id}")),
        Some(Shown::Omarchy { .. }) => None,
    }
}

/// The searchable list of bundled themes, Neutral first, filling the panel's height and
/// scrolling on its own. A press makes that theme the gallery's.
fn picker(g: &Gallery, look: &Look) -> El {
    let kit = &look.kit;
    let query = g.k1.query.text.to_lowercase();
    let entry = |key: String, name: &str, shown: Option<Shown>| {
        let current = g.theme == shown;
        let mut r = row()
            .key(key)
            .role("button")
            .label(name.to_owned())
            .checked(current)
            .focusable(true)
            .cursor(Cursor::Pointer)
            .w_full()
            .h(slots::list::row_height(look))
            .px(look.space.md)
            .gap(look.space.sm)
            .center_y()
            .radius(kit.radius.md)
            .hover(kit.c.hover)
            .press(kit.c.pressed)
            .on_press_tx(move |g: &mut Gallery, tx: &mut Tx| show(g, shown, tx))
            .child(
                text(name.to_owned())
                    .font(kit.ty.ui_compact)
                    .color(kit.c.text_base)
                    .truncate()
                    .shrink()
                    .a11y_hidden(),
            )
            .child(el().grow(1.0));
        if current {
            r = r.fill(kit.alpha(kit.c.bg_accent, 0.22)).child(
                icon(look.icon(IconName::Check), look.icon_size.md)
                    .color(kit.c.icon_accent)
                    .a11y_hidden(),
            );
        }
        r
    };
    let neutral = NEUTRAL
        .to_lowercase()
        .contains(&query)
        .then(|| entry(pick_key(None).expect("Neutral has a row"), NEUTRAL, None));
    let themes = bundled()
        .iter()
        .filter(|t| t.name.to_lowercase().contains(&query))
        .map(|t| {
            let id = t.id.as_str();
            let shown = Some(Shown::Bundled(id));
            entry(pick_key(shown).expect("a bundled row"), &t.name, shown)
        });
    col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .gap(look.space.xs)
        .child(
            TextInput::new(&g.k1.query)
                .label("Search themes")
                .placeholder("Search themes")
                .on_edit(|g: &mut Gallery| &mut g.k1.query)
                .build(kit)
                .key("k1-search")
                .w_full(),
        )
        .child(
            col()
                .key("k1-theme-list")
                .role("list")
                .label("Themes")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .gap(look.space.xxs)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .children(neutral)
                .children(themes),
        )
}

/// Up and Down step through the bundled themes, or an Omarchy scene's variants, animating
/// the switch. From Neutral, Down goes to the first theme and Up to the last.
pub fn on_key(g: &mut Gallery, key: &KeyEvent, tx: &mut Tx) -> bool {
    let delta: i64 = match key.key {
        Key::Down => 1,
        Key::Up => -1,
        _ => return false,
    };
    let themes = bundled();
    let n = themes.len() as i64;
    let next = match g.theme {
        Some(Shown::Bundled(id)) => {
            let at = themes.iter().position(|t| t.id == id).unwrap_or(0) as i64;
            Shown::Bundled(themes[(at + delta).rem_euclid(n) as usize].id.as_str())
        }
        None => {
            let at = if delta > 0 { 0 } else { n - 1 };
            Shown::Bundled(themes[at as usize].id.as_str())
        }
        Some(Shown::Omarchy { light, variant }) => {
            let all = [Variant::Lighter, Variant::Darker, Variant::Contrast];
            let at = all.iter().position(|&v| v == variant).unwrap_or(0) as i64;
            Shown::Omarchy {
                light,
                variant: all[(at + delta).rem_euclid(3) as usize],
            }
        }
    };
    show(g, Some(next), tx);
    true
}

/// Scrolls the theme list to the gallery's theme whenever it changes: a pick, a key, or the
/// page opening on a theme picked before.
pub fn poll(g: &mut Gallery, _t: f64, tx: &mut Tx) -> bool {
    if g.k1.revealed == Some(g.theme) {
        return false;
    }
    g.k1.revealed = Some(g.theme);
    if let Some(key) = pick_key(g.theme) {
        tx.scroll_into_view(key);
    }
    false
}

/// Everything drawn from the look, top to bottom.
fn specimen(look: &Look) -> El {
    col()
        .w_full()
        .padding(look.space.lg)
        .gap(look.space.lg)
        .child(window(look))
        .child(
            row()
                .w_full()
                .gap(look.space.lg)
                .items_start()
                .child(menu(look))
                .child(dialog(look)),
        )
        .child(controls(look))
        .child(code(look))
}

/// A bar action: an icon in the accent, as Strata's header actions draw.
fn action(look: &Look, name: IconName) -> El {
    let c = look.frame.control;
    row()
        .size(c, c)
        .center()
        .child(icon(look.icon(name), look.icon_size.md).color(look.kit.c.icon_accent))
}

/// A window: header and tab strip on the surface, the sidebar beside the file rows.
fn window(look: &Look) -> El {
    let kit = &look.kit;
    let header = row()
        .w_full()
        .h(look.frame.bar)
        .px(look.space.md)
        .gap(look.space.xs)
        .center_y()
        .fill(kit.c.bg_deep)
        .border_sides([0.0, 0.0, 1.0, 0.0], kit.c.border_base)
        .child(action(look, IconName::Back))
        .child(action(look, IconName::Forward))
        .child(
            text("Projects")
                .font(kit.ty.ui_medium)
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
            .fill(if active { kit.c.bg_base } else { kit.c.bg_deep })
            .child(
                text(name.to_owned())
                    .font(kit.ty.ui_compact)
                    .color(if active {
                        kit.c.text_base
                    } else {
                        kit.c.text_muted
                    }),
            )
    };
    let tabs = row()
        .w_full()
        .h(slots::tabs::tab_height(look) - look.space.sm)
        .px(look.space.md)
        .pb(look.space.xs)
        .gap(look.space.xs)
        .fill(kit.c.bg_deep)
        .border_sides([0.0, 0.0, 1.0, 0.0], kit.c.border_base)
        .child(tab("Projects", true))
        .child(tab("Downloads", false));
    col()
        .key("k1-window")
        .w_full()
        .radius(kit.radius.lg)
        .border(1.0, kit.c.border_base)
        .clip_rounded(kit.radius.lg)
        .child(header)
        .child(tabs)
        .child(row().w_full().child(sidebar(look)).child(files(look)))
}

fn sidebar(look: &Look) -> El {
    let kit = &look.kit;
    let entry = |name: IconName, label: &str, fill: Option<Color>, tint: Color| {
        let mut r = row()
            .w_full()
            .h(slots::sidebar::row_height(look))
            .px(look.space.xs)
            .gap(look.space.sm)
            .center_y()
            .radius(kit.radius.md)
            .child(icon(look.icon(name), look.icon_size.md).color(tint))
            .child(
                text(label.to_owned())
                    .font(kit.ty.ui_compact)
                    .color(kit.c.text_base)
                    .truncate(),
            );
        if let Some(fill) = fill {
            r = r.fill(fill);
        }
        r
    };
    col()
        .key("k1-sidebar")
        .w(170.0)
        .padding(look.space.xs)
        .gap(slots::sidebar::row_gap(look))
        .fill(kit.c.bg_deep)
        .border_sides([0.0, 1.0, 0.0, 0.0], kit.c.border_base)
        .child(text("PINNED").font(look.ty.heading).color(kit.c.text_muted))
        .child(entry(
            IconName::Home,
            "Home",
            Some(slots::sidebar::selected_fill(look)),
            slots::sidebar::selected_icon(look),
        ))
        .child(entry(
            IconName::Folder,
            "Documents (hover)",
            Some(slots::sidebar::hover_fill(look)),
            kit.c.icon_muted,
        ))
        .child(entry(IconName::Trash, "Trash", None, kit.c.icon_muted))
}

fn files(look: &Look) -> El {
    let kit = &look.kit;
    let rows: [(&str, &str, Option<Color>, bool); 4] = [
        (
            "notes.md (selected)",
            "2 KB",
            Some(kit.alpha(kit.c.bg_accent, 0.22)),
            false,
        ),
        ("budget.ods (hover)", "14 KB", Some(kit.c.hover), false),
        ("photos (drop target)", "—", Some(look.fill.highlight), true),
        ("todo.txt", "1 KB", None, false),
    ];
    col()
        .key("k1-files")
        .grow(1.0)
        .min_w(0.0)
        .padding(look.space.xs)
        .gap(look.space.xxs)
        .fill(kit.c.bg_base)
        .children(rows.into_iter().map(|(name, size, fill, drop)| {
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
                        .color(if drop {
                            kit.c.text_accent
                        } else {
                            kit.c.text_base
                        })
                        .truncate()
                        .shrink(),
                )
                .child(el().grow(1.0))
                .child(
                    text(size.to_owned())
                        .font(kit.ty.ui_compact)
                        .color(kit.c.text_muted),
                );
            if let Some(fill) = fill {
                r = r.fill(fill);
            }
            if drop {
                r = r.border(1.0, kit.c.border_active);
            }
            r
        }))
}

/// A menu on the surface: items at rest and hovered, and a separator.
fn menu(look: &Look) -> El {
    let kit = &look.kit;
    let item = |label: &str, hover: bool| {
        let r = row()
            .w_full()
            .h(slots::list::row_height(look))
            .px(look.space.md)
            .center_y()
            .radius(kit.radius.sm)
            .child(
                text(label.to_owned())
                    .font(kit.ty.ui_compact)
                    .color(kit.c.text_base),
            );
        if hover { r.fill(kit.c.hover) } else { r }
    };
    let card = col()
        .key("k1-menu")
        .w(190.0)
        .padding(look.space.xs)
        .gap(look.space.xxs)
        .radius(kit.radius.lg)
        .fill(kit.c.layer_01)
        .child(item("Open", false))
        .child(item("Open With… (hover)", true))
        .child(el().w_full().h(1.0).fill(kit.c.border_muted))
        .child(item("Rename", false))
        .child(item("Move to Trash", false));
    kit.elevate(card, &kit.elevation.floating)
}

/// A dialog card with the element glow, and rune-kit buttons.
fn dialog(look: &Look) -> El {
    let kit = &look.kit;
    let glow = &look.glow;
    col()
        .key("k1-glow")
        .grow(1.0)
        .min_w(0.0)
        .padding(look.space.lg)
        .gap(look.space.md)
        .radius(kit.radius.lg)
        .fill(kit.c.layer_01)
        .border(1.0, kit.c.border_base)
        .shadow(0.0, glow.radius, glow.color.alpha(glow.opacity))
        .child(
            text("Replace “notes.md”?")
                .font(look.ty.title)
                .color(kit.c.text_base),
        )
        .child(
            text("A file with this name already exists in Projects.")
                .font(kit.ty.ui)
                .color(kit.c.text_muted),
        )
        .child(
            row()
                .gap(look.space.sm)
                .child(Button::new("Cancel").build(kit).key("k1-cancel"))
                .child(
                    Button::new("Keep both")
                        .variant(ButtonVariant::Submit)
                        .build(kit)
                        .key("k1-submit"),
                )
                .child(
                    Button::new("Replace")
                        .variant(ButtonVariant::Danger)
                        .build(kit)
                        .key("k1-danger"),
                ),
        )
}

/// rune-kit controls, and the danger and info roles.
fn controls(look: &Look) -> El {
    let kit = &look.kit;
    let callout = |key: &str, role: StateColors, label: &str| {
        row()
            .key(key.to_owned())
            .px(look.space.md)
            .py(look.space.sm)
            .radius(kit.radius.md)
            .fill(role.bg)
            .border(1.0, role.border)
            .child(
                text(label.to_owned())
                    .font(kit.ty.ui_compact)
                    .color(role.fg),
            )
    };
    let field = TextEdit::new("~/Projects");
    col()
        .w_full()
        .gap(look.space.md)
        .child(
            row()
                .w_full()
                .gap(look.space.lg)
                .center_y()
                .child(
                    TextInput::new(&field)
                        .label("Location")
                        .state(State::Focus)
                        .build(kit)
                        .key("k1-field")
                        .w(200.0),
                )
                .child(
                    Switch::new(true)
                        .label("Glow")
                        .build(kit)
                        .key("k1-switch-on"),
                )
                .child(
                    Switch::new(false)
                        .label("Thumbnails")
                        .build(kit)
                        .key("k1-switch-off"),
                )
                .child(
                    Checkbox::new(true)
                        .label("Hidden files")
                        .build(kit)
                        .key("k1-check"),
                ),
        )
        .child(
            row()
                .gap(look.space.md)
                .child(callout(
                    "k1-danger-role",
                    kit.c.danger,
                    "Couldn't move 2 items",
                ))
                .child(callout(
                    "k1-info-role",
                    kit.c.info,
                    "Showing 120 of 4,812 items",
                )),
        )
}

const CODE: &str = "// Count the entries under a folder.
#[derive(Debug)]
struct Tally { files: u64, bytes: u64 }
fn tally(dir: &Path) -> io::Result<Tally> {
    let t = Tally { files: 0, bytes: 0x400 };
    println!(\"{t:?}\"); // a string, then a comment
    Ok(t)
}";

/// Rust coloured by rune-kit's built-in scanner and the theme's syntax colours, on the
/// surface as Strata's code previews sit.
fn code(look: &Look) -> El {
    let kit = &look.kit;
    let found = rune_kit::syntax::highlight(Some("rust"), CODE);
    let spans = look.syntax.spans(&found, 0);
    col()
        .key("k1-code")
        .w_full()
        .padding(look.space.md)
        .radius(kit.radius.md)
        .fill(kit.c.layer_01)
        .border(1.0, kit.c.border_muted)
        .child(rich(CODE, spans).font(kit.ty.mono).color(kit.c.text_base))
}
