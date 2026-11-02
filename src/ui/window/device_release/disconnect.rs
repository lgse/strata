// SPDX-License-Identifier: MIT

//! Disconnecting a network share follows a release's rule: every window and tab
//! leaves the share first, and returns if the share stays mounted.

use std::rc::Rc;

use gtk::prelude::*;

use crate::{app::Browser, ui::browser::show_error_dialog};

use super::run::leave_device_at;

/// Leaves `root` in every browser, runs `unmount`, and on a failure that left the
/// share mounted returns each browser the user has not moved since.
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

/// Whether GIO still lists a mount at `root`.
fn mounted_at(root: &gio::File) -> bool {
    gio::VolumeMonitor::get()
        .mounts()
        .iter()
        .any(|mount| mount.root().equal(root))
}

pub(in crate::ui::window) fn disconnect_share(
    parent: &gtk::Widget,
    browser: Option<Rc<Browser>>,
    mount: gio::Mount,
) {
    let window = parent.root().and_downcast::<gtk::Window>();
    let operation = gtk::MountOperation::new(window.as_ref());
    let error_parent = parent.clone();
    glib::MainContext::default().spawn_local(async move {
        let root = mount.root();
        let unmount =
            || mount.unmount_with_operation_future(gio::MountUnmountFlags::NONE, Some(&operation));
        let result =
            leave_share_and_unmount(&root, browser.as_ref(), unmount, || mounted_at(&root)).await;
        if let Err(error) = result
            && !error.matches(gio::IOErrorEnum::Cancelled)
        {
            show_error_dialog(
                &error_parent,
                &crate::i18n::tr("Unable to disconnect"),
                &error.to_string(),
            );
        }
    });
}
