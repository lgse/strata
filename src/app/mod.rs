// SPDX-License-Identifier: MIT

mod browser;
mod navigation;
mod peek;

pub use browser::{Browser, BrowserColumnSnapshot, BrowserEvent, CursorToggle, VisualKind};
pub(crate) use navigation::{EntryInsertion, EntrySplice, compare_display_names};
