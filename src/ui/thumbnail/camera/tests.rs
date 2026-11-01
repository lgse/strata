// SPDX-License-Identifier: MIT

use super::*;
use crate::adapters::gio_close::tests::closed_on_the_ui_thread_once_answered_and_let_go;

#[test]
fn a_camera_icon_stream_answered_as_the_thumbnail_lets_go_closes_on_a_worker() {
    let closed_on_the_ui_thread =
        closed_on_the_ui_thread_once_answered_and_let_go(|file| async move {
            let icon = gio::FileIcon::new(&file);
            let _ = load_icon_stream(icon.upcast_ref()).await;
        });
    assert!(!closed_on_the_ui_thread);
}
