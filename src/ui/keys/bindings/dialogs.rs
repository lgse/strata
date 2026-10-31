// SPDX-License-Identifier: MIT

use gtk::gdk::Key;

use crate::ui::keys::{Binding, Command};

catalog! {
    listed {
        OPEN_CONTAINING_FOLDER: Command "open_containing_folder" = Binding::alt(Key::Return);
    }
    tenxer_only {
        /// The second **d** of **d d**: confirms the 10xer Move to Trash dialog.
        TENXER_DELETE_CONFIRM: Command "confirm_trash" = Binding::plain(Key::d),
            after super::MOVE_TO_TRASH;
    }
}
