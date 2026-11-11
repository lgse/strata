//! The gallery page for C20a Slider and NumberField: its scenes, demo state, view and key and poll
//! hooks. Scene names start with `c20a-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-1-leaves/C20a-slider-number-field.md`.
//!
//! The page lays the controls out as Strata's settings do, each in a rune-kit `Field`: the
//! Appearance menu's Frame and Content density sliders (in a scroll view, as the menu is),
//! the Icons thumbnail-size slider, and the Thumbnail workers, Text size and Corner roundness
//! spin buttons, with C21's text menu on the fields. What each change reports shows in the
//! log line (`c20a-log`), so scripts can tell a live drag from a commit.

use rune::{Cx, El, KeyEvent, ScrollSpec, Tx, WheelEvent, col, row, text};
use rune_kit::Field as KitField;
use strata_components::context_menu::{FieldMenu, FieldMenuState};
use strata_components::controls::number_field::{NumberField, NumberFieldState};
use strata_components::controls::slider::{Range, Slider, SliderChange, SliderState, Snap};
use strata_kit::look::Look;

use crate::{Gallery, Scene};

/// Strata's Frame density: 0–12 px, Compact 0, Airy 4, paging 4 px.
pub const FRAME: (f64, f64, f64) = (12.0, 0.0, 4.0);
/// Strata's Content density: 0–16 px, Compact 4, Airy 8.
pub const CONTENT: (f64, f64, f64) = (16.0, 4.0, 8.0);

/// The page's text fields, for C21's field menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Workers,
    TextSize,
    Roundness,
}

/// C20a's demo state, held in `Gallery::c20a`.
pub struct Demo {
    pub frame: SliderState,
    pub content: SliderState,
    pub thumbs: SliderState,
    pub workers: NumberFieldState,
    pub text_size: NumberFieldState,
    pub roundness: NumberFieldState,
    /// Every control disabled.
    pub disabled: bool,
    pub fields: FieldMenuState<Field>,
    /// The last change any control reported: "Frame Live 3", "Workers 5".
    pub log: String,
    /// Every change reported, oldest first (the page's tests read it).
    pub heard: Vec<String>,
}

fn density(max: f64) -> Range {
    Range::new(0.0, max, 1.0).page(4.0)
}

impl Default for Demo {
    fn default() -> Demo {
        Demo {
            frame: SliderState::new(FRAME.1, density(FRAME.0)),
            content: SliderState::new(CONTENT.1, density(CONTENT.0)),
            thumbs: SliderState::new(64.0, Range::new(32.0, 256.0, 16.0)),
            workers: NumberFieldState::new(4.0, Range::new(1.0, 16.0, 1.0)),
            text_size: NumberFieldState::new(13.0, Range::new(8.0, 48.0, 1.0)).suffix(" px"),
            roundness: NumberFieldState::new(60.0, Range::new(0.0, 200.0, 10.0)).suffix("%"),
            disabled: false,
            fields: FieldMenuState::new(),
            log: String::new(),
            heard: Vec::new(),
        }
    }
}

impl Demo {
    /// Every control at its minimum (`false`) or its maximum.
    fn at_limits(&mut self, max: bool) {
        let end = |r: Range| if max { r.max() } else { r.min() };
        for s in [&mut self.frame, &mut self.content, &mut self.thumbs] {
            s.set_value(end(s.range()));
        }
        for f in [&mut self.workers, &mut self.text_size, &mut self.roundness] {
            f.set_value(end(f.range()));
        }
    }

    pub fn field(&self, f: Field) -> &NumberFieldState {
        match f {
            Field::Workers => &self.workers,
            Field::TextSize => &self.text_size,
            Field::Roundness => &self.roundness,
        }
    }

    pub fn field_mut(&mut self, f: Field) -> &mut NumberFieldState {
        match f {
            Field::Workers => &mut self.workers,
            Field::TextSize => &mut self.text_size,
            Field::Roundness => &mut self.roundness,
        }
    }
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("c20a", dark)`, then fills in `g.c20a`.
pub const SCENES: &[Scene] = &[
    ("c20a-controls-light", |g| g.open("c20a", false)),
    ("c20a-controls-dark", |g| g.open("c20a", true)),
    ("c20a-min-light", |g| limits(g, false, false)),
    ("c20a-min-dark", |g| limits(g, true, false)),
    ("c20a-max-light", |g| limits(g, false, true)),
    ("c20a-max-dark", |g| limits(g, true, true)),
    ("c20a-disabled-light", |g| disabled(g, false)),
    ("c20a-disabled-dark", |g| disabled(g, true)),
    ("c20a-invalid-light", |g| invalid(g, false)),
    ("c20a-invalid-dark", |g| invalid(g, true)),
    ("c20a-engaged-light", |g| engaged(g, false)),
    ("c20a-engaged-dark", |g| engaged(g, true)),
];

/// Scenes for review with `--scene` only, kept out of the goldens.
pub const REVIEW: &[Scene] = &[];

fn limits(g: &mut Gallery, dark: bool, max: bool) {
    g.open("c20a", dark);
    g.c20a.at_limits(max);
}

fn disabled(g: &mut Gallery, dark: bool) {
    g.open("c20a", dark);
    g.c20a.disabled = true;
}

/// The Content slider held mid-drag, the others at rest beside it.
fn engaged(g: &mut Gallery, dark: bool) {
    g.open("c20a", dark);
    g.c20a.content.hold();
}

/// Corner roundness holding typed text it won't take: 250 is past 200.
fn invalid(g: &mut Gallery, dark: bool) {
    g.open("c20a", dark);
    let f = &mut g.c20a.roundness;
    f.edit_mut().replace_text("250");
    f.edited();
}

/// Strata's density value label (`density_value_text`): the preset's name at a preset.
fn density_text(px: f64, (_, compact, airy): (f64, f64, f64)) -> String {
    let name = if px == compact {
        "Compact"
    } else if px == airy {
        "Airy"
    } else {
        "Custom"
    };
    format!("{name} \u{b7} {px} px")
}

fn log_slider(g: &mut Gallery, name: &str, v: f64, change: SliderChange) {
    heard(g, format!("{name} {change:?} {v}"));
}

fn heard(g: &mut Gallery, line: String) {
    g.c20a.heard.push(line.clone());
    g.c20a.log = line;
}

fn density_slider(
    g: &Gallery,
    look: &Look,
    key: &str,
    name: &'static str,
    spec: (f64, f64, f64),
    get: fn(&Gallery) -> &SliderState,
    get_mut: fn(&mut Gallery) -> &mut SliderState,
) -> El {
    let value = get(g).value();
    let words = density_text(value, spec);
    let slider = Slider::new(key, get, get_mut)
        .snaps(vec![
            Snap::labelled(spec.1, "Compact"),
            Snap::labelled(spec.2, "Airy"),
        ])
        .label(name)
        .value_text(words.clone())
        .disabled(g.c20a.disabled)
        .on_change(move |g: &mut Gallery, v, change, _| log_slider(g, name, v, change))
        .build(g, look);
    KitField::new()
        .label(name)
        .prefix(words)
        .control(slider)
        .build(&look.kit)
        .key(format!("{key}-field"))
}

fn number<F>(g: &Gallery, look: &Look, field: Field, name: &str, title: &str, menu: F) -> El
where
    F: Fn(&mut Gallery, strata_components::contract::Point, &mut Tx) -> bool + 'static,
{
    let nf = number_field(field);
    let key = format!("{}-field", nf.key());
    let control = nf
        .label(name)
        .disabled(g.c20a.disabled)
        .on_menu(menu)
        .build(g, look);
    KitField::new()
        .label(title)
        .control(control)
        .build(&look.kit)
        .key(key)
}

type FieldGet = fn(&Gallery) -> &NumberFieldState;
type FieldGetMut = fn(&mut Gallery) -> &mut NumberFieldState;

/// The NumberField for `f`, as the view and the poll both build it.
fn number_field(f: Field) -> NumberField<Gallery> {
    let (key, get, get_mut): (&str, FieldGet, FieldGetMut) = match f {
        Field::Workers => ("c20a-workers", |g| &g.c20a.workers, |g| &mut g.c20a.workers),
        Field::TextSize => (
            "c20a-text-size",
            |g| &g.c20a.text_size,
            |g| &mut g.c20a.text_size,
        ),
        Field::Roundness => (
            "c20a-roundness",
            |g| &g.c20a.roundness,
            |g| &mut g.c20a.roundness,
        ),
    };
    NumberField::new(key, get, get_mut).on_change(move |g: &mut Gallery, v, _| {
        heard(g, format!("{f:?} {v}"));
    })
}

const FIELDS: [Field; 3] = [Field::Workers, Field::TextSize, Field::Roundness];

fn field_edit_mut(g: &mut Gallery, f: Field) -> Option<&mut rune::TextEdit> {
    (!g.c20a.disabled).then(|| g.c20a.field_mut(f).edit_mut())
}

fn field_edit(g: &Gallery, f: Field) -> Option<&rune::TextEdit> {
    (!g.c20a.disabled).then(|| g.c20a.field(f).edit())
}

/// C21's text menu on the page's number fields. An edit it makes is checked again; closing it
/// lets a blur commit the field again.
fn field_menu() -> FieldMenu<Gallery, Field> {
    FieldMenu::new(
        "c20a-field-menu",
        |g: &Gallery| &g.c20a.fields,
        |g: &mut Gallery| &mut g.c20a.fields,
        field_edit_mut,
        field_edit,
    )
    .on_change(|g: &mut Gallery, f, _| g.c20a.field_mut(f).edited())
    .on_close(|g: &mut Gallery, f| {
        g.c20a.field_mut(f).hold(false);
        g.c20a.log = format!("{f:?} menu closed");
    })
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
    let mut layer = Vec::new();
    let menu = field_menu().build(g, look, &mut layer);

    // The Appearance menu's DENSITY section scrolls with its menu, so a wheel over a slider
    // there scrolls (the sliders take no wheel).
    let appearance = col()
        .key("c20a-appearance")
        .w(280.0)
        .gap(look.space.md)
        .child(heading(look, "DENSITY"))
        .child(
            col()
                .key("c20a-appearance-scroll")
                .w_full()
                .h(230.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(
                    col()
                        .w_full()
                        .gap(look.space.lg)
                        .px(look.space.md)
                        .py(look.space.md)
                        .child(density_slider(
                            g,
                            look,
                            "c20a-frame",
                            "Frame",
                            FRAME,
                            |g| &g.c20a.frame,
                            |g| &mut g.c20a.frame,
                        ))
                        .child(density_slider(
                            g,
                            look,
                            "c20a-content",
                            "Content",
                            CONTENT,
                            |g| &g.c20a.content,
                            |g| &mut g.c20a.content,
                        ))
                        .child(
                            text(
                                "Frame spaces the window's bars; Content spaces rows and \
                                 Icons cards. The menu goes on below.",
                            )
                            .font(k.ty.small)
                            .color(k.c.text_muted)
                            .w_full()
                            .wrap(),
                        ),
                )
                .radius(k.radius.md)
                .fill(k.c.bg_base),
        );

    // The Icons pane's Thumbnail size popover: a notch of the wheel moves it 1 px.
    let thumbs = g.c20a.thumbs.value();
    let slider = Slider::new(
        "c20a-thumbs",
        |g: &Gallery| &g.c20a.thumbs,
        |g: &mut Gallery| &mut g.c20a.thumbs,
    )
    .wheel(Some(1.0))
    .label("Thumbnail size")
    .value_text(format!("{thumbs} px"))
    .disabled(g.c20a.disabled)
    .on_change(|g: &mut Gallery, v, change, _| log_slider(g, "Thumbnail size", v, change))
    .build(g, look);
    let ends = row()
        .w_full()
        .child(
            text("Small")
                .font(k.ty.small)
                .color(k.c.text_muted)
                .grow(1.0),
        )
        .child(text("Large").font(k.ty.small).color(k.c.text_muted));
    let icons = col()
        .key("c20a-icons")
        .w(260.0)
        .gap(look.space.md)
        .child(heading(look, "ICONS"))
        .child(
            KitField::new()
                .label("Thumbnail size")
                .prefix(format!("{thumbs} px"))
                .control(col().w_full().child(slider).child(ends))
                .build(k)
                .key("c20a-thumbs-field"),
        );

    let open = |f: Field| {
        move |g: &mut Gallery, at, tx: &mut Tx| {
            field_menu().open(g, f, at, tx);
            g.c20a.fields.is_open()
        }
    };
    let numbers = col()
        .key("c20a-numbers")
        .w(240.0)
        .gap(look.space.lg)
        .child(heading(look, "SETTINGS"))
        .child(number(
            g,
            look,
            Field::Workers,
            "Thumbnail workers",
            "Thumbnail workers",
            open(Field::Workers),
        ))
        .child(number(
            g,
            look,
            Field::TextSize,
            "Text size in pixels",
            "Text size",
            open(Field::TextSize),
        ))
        .child(number(
            g,
            look,
            Field::Roundness,
            "Corner roundness in percent",
            "Corner roundness",
            open(Field::Roundness),
        ));

    let log = text(if g.c20a.log.is_empty() {
        "\u{2014}".to_owned()
    } else {
        g.c20a.log.clone()
    })
    .key("c20a-log")
    .font(k.ty.small)
    .color(k.c.text_muted);

    // The page hears what a control passes on (a sideways wheel, Ctrl+wheel for the text
    // size, Ctrl+Shift+wheel for Strata's thumbnail size) and takes none of it.
    rune_kit::root(
        col()
            .key("c20a")
            .w_full()
            .h_full()
            .on_wheel(|g: &mut Gallery, _: WheelEvent, _: &mut Tx| {
                g.c20a.log = "Page wheel".to_owned();
                false
            })
            .px(look.space.xl)
            .py(look.space.xl)
            .gap(look.space.xl)
            .fill(k.c.layer_01)
            .child(
                text("Settings controls")
                    .font(look.ty.title)
                    .color(k.c.text_base),
            )
            .child(
                row()
                    .w_full()
                    .gap(look.space.xxl)
                    .items_start()
                    .child(appearance)
                    .child(icons)
                    .child(numbers),
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
/// Commits a number field that lost the keyboard, and closes the field menu if its field
/// went. True when state changed.
pub fn poll(g: &mut Gallery, _t: f64, tx: &mut Tx) -> bool {
    let mut changed = false;
    for f in FIELDS {
        changed |= number_field(f).poll(g, tx);
    }
    changed |= field_menu().check(g);
    changed
}
