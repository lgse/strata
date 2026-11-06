// SPDX-License-Identifier: MIT

use gtk::gdk::Key;

use crate::ui::keys::Binding;

catalog! {
    listed {
        FILTER_DESCEND: Binding = Binding::plain(Key::slash);
    }
    unlisted {
        // GTK's own unselect in selectable labels; a table's cell selection follows it.
        UNSELECT_ALL: Binding = Binding::ctrl_shift(Key::a);
    }
}
