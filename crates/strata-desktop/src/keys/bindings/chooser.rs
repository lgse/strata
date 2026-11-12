// SPDX-License-Identifier: MIT

use crate::keys::{Binding, Command, Key};

catalog! {
    tenxer_only {
        /// Folder requests in the file chooser accept with it in either map.
        CHOOSE_FOLDER: Command "choose_folder" = Binding::ctrl(Key::Return), live in both;
    }
}
