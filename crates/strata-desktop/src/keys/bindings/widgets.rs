// SPDX-License-Identifier: MIT

use crate::keys::{Binding, Key};

catalog! {
    listed {
        FILTER_DESCEND: Binding = Binding::plain(Key::slash);
    }
    unlisted {
        UNSELECT_ALL: Binding = Binding::ctrl_shift(Key::a);
    }
}
