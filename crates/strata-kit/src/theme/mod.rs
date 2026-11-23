// SPDX-License-Identifier: MIT

mod catalog;
mod color;
mod colors;
mod file;
mod map;
pub mod omarchy;
mod switch;
pub mod syntax;

pub use catalog::{DEFAULT_ID, UNKNOWN_ID_FALLBACK, bundled, bundled_theme};
pub use color::Rgba8;
pub use colors::{
    Colors, DEFAULT_DANGER, ErrorKind, Key, Location, Source, SourceColors, Theme, ThemeError,
};
pub use file::{Library, parse_custom};
pub use switch::{MOTION, switch};
