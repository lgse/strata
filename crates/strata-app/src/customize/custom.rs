// SPDX-License-Identifier: MIT

//! Custom Folder Color and Custom File Color: GTK's `show_custom_color_modal`
//! (`customization/custom_color.rs`), stacked over the Customize dialog. C20b's picker takes
//! the place of GTK's colour chooser: its plane, hue strip and hex field are what GTK's editor
//! page shows, so there is no palette page to go back from.
//!
//! It opens on the item's colour, or GTK's `#34d399` when it has none. Apply saves the picked
//! colour as a custom one and closes; Cancel, Escape and a click outside close it unsaved.
//! Either way the keyboard goes back to the Customize dialog's custom colour button.

use std::cell::RefCell;

use rune::Tx;
use strata_components::{
    controls::color_picker::{ColorPicker, ColorPickerState, PickerWords, Srgba},
    dialog::{Focus, StrataDialog},
};
use strata_core::{i18n::tr, model::FolderColorValue};

use super::{Item, body::CUSTOM, choose_color};
use crate::{App, slots::Frame};

/// GTK's starting colour when the item has none, `#34d399`.
const START: Srgba = Srgba::rgb(0x34, 0xd3, 0x99);

const PICKER: &str = "customize-picker";

pub(super) struct Custom {
    open: bool,
    opened: u64,
    picker: ColorPickerState,
}

impl Custom {
    pub(super) fn new() -> Self {
        Self {
            open: false,
            opened: 0,
            picker: ColorPickerState::new(START),
        }
    }

    pub(super) fn is_open(&self) -> bool {
        self.open
    }

    pub(super) fn field_focused(&self) -> bool {
        self.open && self.picker.edit().focused
    }

    pub(super) fn close(&mut self) {
        self.open = false;
    }
}

fn state(app: &App) -> &ColorPickerState {
    &app.customize.custom.picker
}

fn state_mut(app: &mut App) -> &mut ColorPickerState {
    &mut app.customize.custom.picker
}

/// "Custom color…": the dialog opens on `initial`, the item's colour.
pub(super) fn open(app: &mut App, initial: Option<&str>) {
    app.customize.emoji.close();
    let value = initial.and_then(Srgba::parse_hex).unwrap_or(START);
    let custom = &mut app.customize.custom;
    custom.picker.set_value(value);
    custom.opened += 1;
    custom.open = true;
}

fn cancel(app: &mut App) {
    app.customize.custom.open = false;
}

fn apply(app: &mut App) {
    let custom = &mut app.customize.custom;
    if !custom.open {
        return;
    }
    custom.open = false;
    if let Some(parsed) = Srgba::parse_hex(&custom.picker.edit().text) {
        custom.picker.set_value(parsed);
    }
    let hex = custom.picker.value().hex_rgb();
    choose_color(app, Some(FolderColorValue::Custom(hex)));
}

fn picker() -> ColorPicker<App> {
    ColorPicker::new(PICKER, state, state_mut).words(words())
}

/// The picker's own timers, while its dialog is open.
pub(super) fn poll(app: &mut App, tx: &mut Tx) -> bool {
    app.customize.custom.open && picker().poll(app, tx)
}

pub(super) fn build(app: &App, item: &Item, frame: &mut Frame<'_, App>) {
    let custom = &app.customize.custom;
    if custom.opened == 0 {
        return;
    }
    let (title, subtitle) = if item.folder {
        ("Custom Folder Color", "Choose a color for this folder")
    } else {
        ("Custom File Color", "Choose a color for this file")
    };
    StrataDialog::new(
        format!("customize-custom-{}", custom.opened),
        tr(title),
        cancel,
    )
    .open(custom.open)
    .stacked(true)
    .subtitle(tr(subtitle))
    .child(picker().build(app, &app.look))
    .secondary(tr("Cancel"), cancel)
    .primary(tr("Apply"), apply)
    .initial_focus(Focus::First)
    .close_label(tr("Close dialog"))
    .closable(false)
    .return_focus(CUSTOM)
    .build(&app.look, frame.layer);
}

thread_local! {
    /// The words in the language they were made for: [`PickerWords`] holds them as
    /// `&'static str`, so each language's are made once.
    static WORDS: RefCell<Vec<(String, PickerWords)>> = const { RefCell::new(Vec::new()) };
}

/// The picker's words in the language now.
fn words() -> PickerWords {
    let locale = rust_i18n::locale().to_string();
    WORDS.with_borrow_mut(|made| {
        if let Some((_, words)) = made.iter().find(|(made, _)| *made == locale) {
            return *words;
        }
        let leak = |text: String| -> &'static str { Box::leak(text.into_boxed_str()) };
        let words = PickerWords {
            plane: leak(tr("Saturation and value")),
            hue: leak(tr("Hue")),
            hex: leak(tr("Hex color")),
            plane_value: |saturation, value| {
                rust_i18n::t!(
                    "Saturation %{saturation}%, value %{value}%",
                    saturation = saturation,
                    value = value
                )
                .into_owned()
            },
            ..PickerWords::default()
        };
        made.push((locale, words));
        words
    })
}
