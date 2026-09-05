// SPDX-License-Identifier: GPL-3.0-or-later

use crate::model::Location;
use gtk::gio;

pub(crate) fn gio_file_for_location(location: &Location) -> gio::File {
    location
        .native_path()
        .map(gio::File::for_path)
        .unwrap_or_else(|| gio::File::for_uri(location.uri_value().unwrap_or_default()))
}
