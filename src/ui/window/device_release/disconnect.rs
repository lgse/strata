// SPDX-License-Identifier: MIT

//! Disconnecting a network share follows a release's rule: every window and tab
//! leaves the share first, and returns if the share stays mounted.

use std::{cell::Cell, rc::Rc};

use gtk::prelude::*;

use crate::{app::Browser, ui::browser::show_error_dialog};

use super::run::{leave_device_at, release_error_is_quiet};

pub(super) async fn leave_share_and_unmount<F, Fut>(
    root: &gio::File,
    browser: Option<&Rc<Browser>>,
    unmount: F,
    still_mounted: impl FnOnce() -> bool,
) -> Result<(), glib::Error>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), glib::Error>>,
{
    let way_back = leave_device_at(root, browser);
    let result = unmount().await;
    if result.is_err() && still_mounted() {
        way_back.restore();
    }
    result
}

/// Read right after a failed unmount, GIO's mount list may not yet show a share that
/// went away anyway; the browsers then return to a share that is gone and report it
/// as any unavailable folder does.
fn mounted_at(root: &gio::File) -> bool {
    gio::VolumeMonitor::get()
        .mounts()
        .iter()
        .any(|mount| mount.root().equal(root))
}

/// `in_flight` is the share row's own: a second Disconnect is ignored while the first runs.
pub(in crate::ui::window) fn disconnect_share(
    parent: &gtk::Widget,
    browser: Option<Rc<Browser>>,
    mount: gio::Mount,
    in_flight: Rc<Cell<bool>>,
) {
    if in_flight.replace(true) {
        return;
    }
    let window = parent.root().and_downcast::<gtk::Window>();
    let operation = gtk::MountOperation::new(window.as_ref());
    let error_parent = parent.clone();
    glib::MainContext::default().spawn_local(async move {
        let root = mount.root();
        let unmount =
            || mount.unmount_with_operation_future(gio::MountUnmountFlags::NONE, Some(&operation));
        let result =
            leave_share_and_unmount(&root, browser.as_ref(), unmount, || mounted_at(&root)).await;
        in_flight.set(false);
        // Cancelling GVfs's own "busy" dialog reports FailedHandled, not Cancelled.
        if let Err(error) = result
            && !release_error_is_quiet(&error)
        {
            show_error_dialog(
                &error_parent,
                &crate::i18n::tr("Unable to disconnect"),
                &error.to_string(),
            );
        }
    });
}
