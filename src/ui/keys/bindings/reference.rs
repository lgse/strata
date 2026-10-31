// SPDX-License-Identifier: MIT

use gtk::gdk::Key;

use crate::ui::keys::{Binding, Command};

catalog! {
    listed {}
    unlisted {
        // The panel's own note lists these; they are not reference rows.
        REFERENCE_CATEGORIES: Command "reference_categories" = Binding::ctrl(Key::b),
            live in both;
        REFERENCE_SEARCH: Command "reference_search" = Binding::ctrl(Key::f), live in both;
        REFERENCE_LIST: Command "reference_list" = Binding::ctrl(Key::l), live in both;
    }
}
