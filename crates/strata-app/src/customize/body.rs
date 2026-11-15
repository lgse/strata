// SPDX-License-Identifier: MIT

//! The Customize dialog's body: the preview, the COLOR bar and the ICON choices, as GTK's
//! `append_preview`, `build_folder_color_bar` (`customization/color_bar.rs`) and
//! `build_icon_buttons` (`customization/icon_picker.rs`) build them. Every choice is a named
//! button; the one the item wears shows active.

use rune::{Color, El, Tx, col, icon, row, text};
use rune_kit::{Button, ButtonVariant};
use strata_components::{
    contract::{Entry, EntryId, EntryKind},
    thumbnails::entry_glyph,
};
use strata_core::{
    i18n::tr,
    model::{FolderColor, FolderColorValue},
};
use strata_desktop::settings::ICON_CHOICES;
use strata_kit::{icons::IconName, look::Look};

use super::{Item, choose_color, choose_icon, custom, emoji};
use crate::{
    App,
    slots::{SavedGlyph, color_of},
};

/// What the body shows: the item and what it wears now.
pub(super) struct Shown<'a> {
    pub(super) item: &'a Item,
    pub(super) color: Option<&'a FolderColorValue>,
    pub(super) icon: Option<&'a str>,
}

/// The key of the custom colour's button, where the keyboard goes back from its dialog.
pub(super) const CUSTOM: &str = "customize-color-custom";

const DOT: f32 = 20.0;
const CHOICE_ICON: f32 = 20.0;
const PREVIEW: f32 = 56.0;

pub(super) fn children(app: &App, shown: &Shown<'_>, over: &mut Vec<El>) -> Vec<El> {
    let look = &app.look;
    vec![
        preview(look, shown),
        section(look, "COLOR", color_bar(look, shown)),
        section(look, "ICON", icon_section(app, shown, over)),
    ]
}

fn icon_section(app: &App, shown: &Shown<'_>, over: &mut Vec<El>) -> El {
    let look = &app.look;
    col()
        .w_full()
        .gap(look.space.sm)
        .child(icon_choices(look, shown))
        .child(emoji::button(app, shown.icon, over))
}

fn section(look: &Look, heading: &str, content: El) -> El {
    col()
        .w_full()
        .gap(look.space.sm)
        .child(
            text(tr(heading))
                .font(look.ty.heading)
                .color(look.kit.c.text_muted),
        )
        .child(content)
}

/// The item's icon or emoji in its colour — inside the coloured folder when the item is a
/// folder — with its name under it, as GTK's 56 px preview (`show_customized_icon_image`).
fn preview(look: &Look, shown: &Shown<'_>) -> El {
    let k = &look.kit;
    let kind = if shown.item.folder {
        EntryKind::Folder
    } else {
        EntryKind::File
    };
    let saved = shown.icon.and_then(SavedGlyph::parse);
    let entry = Entry::new(EntryId::new(0), &shown.item.name, kind, "")
        .maybe_color(shown.color.and_then(color_of))
        .maybe_glyph(saved.as_ref().map(SavedGlyph::as_entry));
    let glyph = entry_glyph(look, entry, PREVIEW, k.c.icon_accent);
    col()
        .key("customize-preview")
        .w_full()
        .gap(look.space.xs)
        .items_center()
        .child(glyph)
        .child(
            text(shown.item.name.clone())
                .font(k.ty.ui_medium)
                .color(k.c.text_base),
        )
}

struct Dot {
    key: String,
    name: String,
    active: bool,
    art: El,
}

/// A round button of the bar, named `dot.name`, drawing `dot.art`.
fn dot_button(look: &Look, dot: Dot, press: impl Fn(&mut App, &mut Tx) + 'static) -> El {
    let k = &look.kit;
    let ring = if dot.active {
        k.c.border_focus
    } else {
        Color::TRANSPARENT
    };
    Button::new(dot.name)
        .variant(ButtonVariant::Ghost)
        .label_with(move |_, _| dot.art)
        .on_press_tx(press)
        .build(k)
        .key(dot.key)
        .px(look.space.xxs)
        .radius(DOT)
        .border(2.0, ring)
}

fn swatch(look: &Look, color: Color, active: bool) -> El {
    let mut dot = row().size(DOT, DOT).radius(DOT / 2.0).fill(color).center();
    if active {
        dot = dot.child(icon(look.icon(IconName::Check), DOT / 2.0).color(look.kit.c.white));
    }
    dot
}

/// The theme's colour, GTK's seven presets, and a custom colour.
fn color_bar(look: &Look, shown: &Shown<'_>) -> El {
    let k = &look.kit;
    let mut bar = row().w_full().flex_wrap().gap(look.space.xs).items_center();
    let theme = look
        .icon_named("strata-palette")
        .unwrap_or_else(|| look.icon(IconName::Settings));
    bar = bar.child(dot_button(
        look,
        Dot {
            key: "customize-color-default".to_owned(),
            name: tr("Default (Theme color)"),
            active: shown.color.is_none(),
            art: icon(theme, DOT).color(k.c.icon_accent),
        },
        |app, _| choose_color(app, None),
    ));
    for preset in FolderColor::ALL {
        let value = FolderColorValue::Preset(preset);
        let active = shown.color == Some(&value);
        let color = color_of(&value).unwrap_or(k.c.icon_accent);
        bar = bar.child(dot_button(
            look,
            Dot {
                key: format!("customize-color-{}", preset.name().to_ascii_lowercase()),
                name: tr(preset.name()),
                active,
                art: swatch(look, color, active),
            },
            // A second press on the active preset puts the theme's colour back.
            move |app, _| choose_color(app, (!active).then_some(FolderColorValue::Preset(preset))),
        ));
    }
    bar.child(custom_button(look, shown))
}

/// "Custom color…", or the custom colour the item wears: "Custom (#34d399)".
fn custom_button(look: &Look, shown: &Shown<'_>) -> El {
    let k = &look.kit;
    let custom = shown
        .color
        .filter(|value| matches!(value, FolderColorValue::Custom(_)));
    let (name, art) = match custom {
        Some(value) => (
            rust_i18n::t!("Custom (%{hex})", hex = value.hex()).into_owned(),
            swatch(look, color_of(value).unwrap_or(k.c.icon_accent), true),
        ),
        None => (
            tr("Custom color…"),
            row()
                .size(DOT, DOT)
                .center()
                .child(icon(look.icon(IconName::Plus), DOT / 2.0).color(k.c.icon_accent)),
        ),
    };
    let initial = shown.color.map(|value| value.hex().to_owned());
    dot_button(
        look,
        Dot {
            key: CUSTOM.to_owned(),
            name,
            active: custom.is_some(),
            art,
        },
        move |app, _| custom::open(app, initial.as_deref()),
    )
}

/// GTK's sixteen bundled icons, each a button named by its label.
fn icon_choices(look: &Look, shown: &Shown<'_>) -> El {
    let k = &look.kit;
    let mut grid = row().w_full().flex_wrap().gap(look.space.sm);
    for (index, (name, label)) in ICON_CHOICES.iter().enumerate() {
        let Some(art) = look.icon_named(name) else {
            continue;
        };
        let active = shown.icon == Some(*name);
        let ring = if active {
            k.c.border_focus
        } else {
            Color::TRANSPARENT
        };
        let ink = k.c.icon_accent;
        grid = grid.child(
            Button::new(tr(label))
                .variant(ButtonVariant::Neutral)
                .label_with(move |_, _| icon(art, CHOICE_ICON).color(ink))
                .on_press_tx(move |app: &mut App, _: &mut Tx| choose_icon(app, name))
                .build(k)
                .key(format!("customize-icon-{index}"))
                .border(1.0, ring),
        );
    }
    grid
}
