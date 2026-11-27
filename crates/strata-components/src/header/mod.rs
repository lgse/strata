// SPDX-License-Identifier: MIT

mod bar;
mod chrome;
mod frame;

#[cfg(test)]
mod tests;

pub use bar::{Header, action};
pub use chrome::{HeaderLabels, Shown, WindowAction, WindowButtons, WindowChrome};
pub use frame::{frame, resize_bands};
