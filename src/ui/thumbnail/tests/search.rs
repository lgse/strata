// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::ui::{
    browser::{BrowserView, PeekBehavior},
    browser_modes::BrowserMode,
    thumbnail,
};
use std::rc::Rc;

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "search thumbnail did not settle");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn search_results_load_thumbnails_in_every_view() {
    gtk_test(
        "ui::thumbnail::tests::search::search_results_load_thumbnails_in_every_view",
        || {
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                let fixture = tempfile::tempdir().expect("fixture");
                let nested = fixture.path().join("nested");
                std::fs::create_dir(&nested).expect("nested directory");
                let path = nested.join("search-photo.png");
                let pixbuf = gtk::gdk_pixbuf::Pixbuf::new(
                    gtk::gdk_pixbuf::Colorspace::Rgb,
                    false,
                    8,
                    32,
                    32,
                )
                .expect("image fixture");
                pixbuf.fill(0x6699ccff);
                let png = pixbuf.save_to_bufferv("png", &[]).expect("PNG");
                std::fs::write(&path, &png).expect("image file");
                hold_thumbnail_workers();

                let view = BrowserView::new(
                    Rc::new(crate::adapters::LocalFileSource),
                    PeekBehavior::default(),
                );
                view.set_view_mode(mode);
                let browser = view.browser();
                let window = gtk::Window::builder()
                    .child(&view.widget())
                    .default_width(1000)
                    .default_height(600)
                    .build();
                window.present();
                browser.navigate(Location::local(fixture.path()));
                wait_until(|| {
                    browser
                        .column_snapshot(0)
                        .is_some_and(|snapshot| !snapshot.loading)
                });
                assert_eq!(view.view_mode(), mode);
                assert!(view.show_filter_with_query("search-photo"));
                wait_until(|| has_pending_thumbnail(&path));
                let key = ThumbnailKey {
                    path: path.clone(),
                    modified: None,
                    file_size: None,
                    thumbnail_size: 17,
                };
                let job_id = PENDING_THUMBNAILS.with(|pending| pending.borrow()[&key].id);
                let targets = take_pending_targets(&key, job_id).expect("search thumbnail job");
                assert!(!targets.is_empty());
                finish_thumbnail_targets(targets, Some(&sample_texture()), &path);
                wait_until(|| {
                    thumbnail::TRACKED_THUMBNAILS.with(|tracked| {
                        tracked.borrow().iter().any(|tracked| {
                            tracked.path == path
                                && tracked
                                    .image
                                    .upgrade()
                                    .is_some_and(|image| image.is_mapped())
                        })
                    })
                });
                thumbnail::cancel_thumbnails_in(&view.widget());
                browser.clear_observer();
                window.destroy();
                clear_thumbnail_runtime();
            }
        },
    );
}
