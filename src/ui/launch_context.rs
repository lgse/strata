// SPDX-License-Identifier: MIT

use gtk::{gdk, prelude::*};

/// A launch context for programs started from `display`, with Strata's own `GDK_DEBUG`
/// override removed.
pub(crate) fn for_display(display: &gdk::Display) -> gdk::AppLaunchContext {
    let context = display.app_launch_context();
    crate::child_environment::restore_launch_context(&context);
    context
}
