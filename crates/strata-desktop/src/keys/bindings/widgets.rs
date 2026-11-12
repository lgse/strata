// SPDX-License-Identifier: MIT

use crate::keys::{Binding, Key};

catalog! {
    listed {
        FILTER_DESCEND: Binding = Binding::plain(Key::slash);
    }
    unlisted {
        // GTK's own unselect in selectable labels; a table's cell selection follows it.
        UNSELECT_ALL: Binding = Binding::ctrl_shift(Key::a);
    }
}
