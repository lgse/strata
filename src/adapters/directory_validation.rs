// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests;

use gtk::{gio, glib, prelude::*};

use crate::model::Location;

/// Queries metadata only, follows directory symlinks, and never probes on the GTK thread.
/// One query at a time bounds speculative work even for very large foreign selections.
/// Returns `None` unless every location is a directory.
pub(crate) async fn folder_names(locations: Vec<Location>) -> Option<Vec<(Location, String)>> {
    if locations.is_empty() {
        return None;
    }
    let mut folders = Vec::with_capacity(locations.len());
    for location in locations {
        let info = super::gio_file_for_location(&location)
            .query_info_future(
                "standard::type,standard::display-name",
                gio::FileQueryInfoFlags::NONE,
                glib::Priority::DEFAULT,
            )
            .await
            .ok()
            .filter(|info| info.file_type() == gio::FileType::Directory)?;
        let name = info.display_name().to_string();
        folders.push((location, name));
    }
    Some(folders)
}
