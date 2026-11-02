// SPDX-License-Identifier: MIT

mod apps;
mod dialog;
mod launch;
mod navigation;

use std::sync::OnceLock;

pub(super) use apps::{Choices, choices_for_type, requires_uri_handlers, resolve};
pub(super) use dialog::{OpenWithContext, show};
#[cfg(test)]
use launch::command_line_at;
#[cfg(test)]
use launch::launch_in_folder;
pub(super) use launch::{launch, launch_at, launch_with_portal};

/// Inside a Flatpak sandbox GIO only sees the runtime's desktop entries, so
/// files are handed to the host through the OpenURI portal instead.
pub(in crate::ui) fn uses_portal() -> bool {
    static FLATPAK: OnceLock<bool> = OnceLock::new();
    *FLATPAK.get_or_init(|| {
        #[cfg(not(test))]
        crate::util::ui_thread::assert_off();
        std::path::Path::new("/.flatpak-info").exists()
    })
}

#[cfg(test)]
mod tests;
