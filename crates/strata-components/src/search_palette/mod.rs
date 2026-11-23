// SPDX-License-Identifier: MIT

mod data;
mod field;
mod intent;
mod mode;
mod model;
mod state;
mod view;

#[cfg(test)]
mod tests;

pub use data::{Answer, PaletteItem, PaletteQuery};
pub use intent::{By, PaletteIntent, Via};
pub use mode::{Hint, IdleText, PaletteMode, ResultKind};
pub use model::Phase;
pub use state::{PaletteState, Polled};
pub use view::SearchPalette;

pub const PANEL_WIDTH: f32 = 760.0;

/// The search bar's height (`.search-bar`'s `min-height`).
pub const BAR_HEIGHT: f32 = 58.0;

pub const BAR_GAP: f32 = 10.0;

pub const PAD_X: f32 = 18.0;

/// The results area's height (`.search-results-scroll` and the stack's 360 px).
pub const RESULTS_HEIGHT: f32 = 360.0;

/// The footer's height (`.search-footer`'s `min-height`).
pub const FOOTER_HEIGHT: f32 = 34.0;

/// The space between footer hints (`search_footer`'s box spacing).
pub const FOOTER_GAP: f32 = 18.0;

/// The space between a hint's icon and its words (`icon_hint`'s box spacing).
pub const HINT_GAP: f32 = 5.0;

/// A hint's icon (`icon_hint`'s 13 px icon).
pub const HINT_ICON: f32 = 13.0;

/// The space between a row's thumbnail and its lines (`result_row`'s box spacing).
pub const ROW_GAP: f32 = 12.0;

/// The status's padding (`.search-status`: 28 px above and below, 24 px beside).
pub const STATUS_PAD_Y: f32 = 28.0;
pub const STATUS_PAD_X: f32 = 24.0;

/// The field's key, under the palette keyed `key`.
pub fn field_key(key: &str) -> String {
    format!("{key}-field")
}

/// The result list's key (role `listbox`), under the palette keyed `key`.
pub fn results_key(key: &str) -> String {
    format!("{key}-results")
}

pub fn option_key(key: &str, index: usize) -> String {
    format!("{key}-option-{index}")
}
