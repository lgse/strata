// SPDX-License-Identifier: MIT

use super::*;

use crate::ui::browser::{BrowserView, PeekBehavior};
use crate::ui::browser_modes::BrowserMode;

fn wait_until(message: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "{message}");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn find_grid(widget: &gtk::Widget) -> Option<gtk::GridView> {
    if let Some(grid) = widget.downcast_ref::<gtk::GridView>() {
        return Some(grid.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(grid) = find_grid(&widget) {
            return Some(grid);
        }
        child = widget.next_sibling();
    }
    None
}

#[test]
fn deferred_work_resumes_after_the_last_scroll_allocation() {
    crate::test_support::gtk_test(
        "ui::thumbnail::viewport::scroll_tests::deferred_work_resumes_after_the_last_scroll_allocation",
        || {
            let directory = tempfile::tempdir().expect("fixture");
            let paths = (0..8)
                .map(|index| directory.path().join(format!("photo-{index}.png")))
                .collect::<Vec<_>>();
            for path in &paths {
                std::fs::write(path, b"fixture").expect("file");
            }
            let browser = Browser::new(Rc::new(crate::adapters::LocalFileSource));
            browser.navigate(Location::local(directory.path()));
            wait_until("directory did not load", || {
                browser.entry_at(0, 7).is_some()
            });
            clear_thumbnail_runtime();
            hold_thumbnail_workers();
            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let images = paths
                .iter()
                .map(|_| ThumbnailSlot::new(96))
                .collect::<Vec<_>>();
            for image in &images {
                content.append(image);
            }
            let scroll = gtk::ScrolledWindow::builder().child(&content).build();
            let window = gtk::Window::builder()
                .child(&scroll)
                .default_width(300)
                .default_height(300)
                .build();
            window.present();
            for (position, (image, path)) in images.iter().zip(&paths).enumerate() {
                set_thumbnail_or_icon_for_path(image, path, crate::assets::icons::PICTURES, 96, 96);
                request_metadata(
                    image,
                    image,
                    &browser,
                    0,
                    position,
                    Location::local(path),
                    false,
                );
            }
            wait_until("initial thumbnail was not admitted", || {
                has_pending_thumbnail(&paths[0])
            });
            glib::MainContext::default().block_on(glib::timeout_future(Duration::from_millis(100)));
            assert!(!has_pending_thumbnail(&paths[7]));
            assert_eq!(
                browser.entry_at(0, 7).expect("last entry").size,
                MetadataValue::Unknown
            );

            // Scroll just after a frame so the admission idle sees the old allocation.
            // These mapped targets are not rebound: no map/bind or worker completion can rescue them.
            let clock = scroll.frame_clock().expect("frame clock");
            let handler = Rc::new(RefCell::new(None));
            let handler_for_paint = handler.clone();
            let adjustment = scroll.vadjustment();
            handler.replace(Some(clock.connect_after_paint(move |clock| {
                clock.disconnect(
                    handler_for_paint
                        .borrow_mut()
                        .take()
                        .expect("one-shot handler"),
                );
                adjustment.set_value(adjustment.upper() - adjustment.page_size());
            })));
            clock.request_phase(gdk::FrameClockPhase::AFTER_PAINT);
            wait_until("visible thumbnail stayed deferred after allocation", || {
                has_pending_thumbnail(&paths[7])
            });
            crate::ui::thumbnail::tests::complete_pending_thumbnail(&paths[7]);
            wait_until("visible metadata stayed deferred after allocation", || {
                browser
                    .entry_at(0, 7)
                    .is_some_and(|entry| entry.size == MetadataValue::Known(7))
            });
            cancel_thumbnails_in(content.upcast_ref());
            window.destroy();
            clear_thumbnail_runtime();
        },
    );
}

#[test]
fn fast_scroll_admits_dimensions_for_every_visible_cached_thumbnail() {
    crate::test_support::gtk_test(
        "ui::thumbnail::viewport::scroll_tests::fast_scroll_admits_dimensions_for_every_visible_cached_thumbnail",
        || {
            let directory = tempfile::tempdir().expect("fixture");
            let pixbuf =
                gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, 32, 24)
                    .expect("image");
            pixbuf.fill(0x6699ccff);
            let png = pixbuf.save_to_bufferv("png", &[]).expect("PNG");
            let texture = gdk::Texture::for_pixbuf(&pixbuf);
            clear_thumbnail_runtime();
            for index in 0..5000 {
                let path = directory.path().join(format!("photo-{index:04}.png"));
                std::fs::write(&path, &png).expect("file");
                if index >= 4900 {
                    THUMBNAIL_CACHE.with(|cache| {
                        cache.borrow_mut().insert(
                            ThumbnailKey {
                                path,
                                modified: None,
                                file_size: None,
                                thumbnail_size: crate::ui::thumbnail_cache::CANONICAL_MAX_EDGE,
                            },
                            texture.clone(),
                        )
                    });
                }
            }
            hold_thumbnail_workers();
            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                PeekBehavior::default(),
            );
            view.set_view_mode(BrowserMode::Icons);
            let browser = view.browser();
            let window = gtk::Window::builder()
                .child(&view.widget())
                .default_width(1400)
                .default_height(1100)
                .build();
            window.present();
            browser.navigate(Location::local(directory.path()));
            wait_until("directory did not load", || {
                browser.column_snapshot(0).is_some_and(|s| !s.loading)
                    && has_pending_thumbnail(&directory.path().join("photo-0000.png"))
            });
            let grid = find_grid(&view.widget()).expect("icons grid");
            let scroll = viewport_of(&grid).expect("listing viewport");
            let adjustment = scroll.vadjustment();
            for step in 1..=100 {
                adjustment.set_value(
                    (adjustment.upper() - adjustment.page_size()) * f64::from(step) / 100.0,
                );
                let frame = Rc::new(Cell::new(false));
                let done = frame.clone();
                grid.add_tick_callback(move |_, _| {
                    done.set(true);
                    glib::ControlFlow::Break
                });
                wait_until("scroll frame did not arrive", || frame.get());
            }
            let displayed = || {
                TRACKED_THUMBNAILS.with(|tracked| {
                    tracked
                        .borrow()
                        .iter()
                        .filter(|target| {
                            target
                                .image
                                .upgrade()
                                .is_some_and(|image| visibility(&image).0 == 0)
                        })
                        .map(|target| target.path.clone())
                        .collect::<Vec<_>>()
                })
            };
            wait_until("warm thumbnails did not reach the final viewport", || {
                displayed().len() > 16
            });
            let visible = displayed();
            wait_until(
                "dimensions stopped before the end of the visible thumbnail set",
                || {
                    visible.iter().all(|path| {
                        let index: usize = path
                            .file_stem()
                            .expect("fixture stem")
                            .to_str()
                            .expect("ASCII fixture name")
                            .strip_prefix("photo-")
                            .expect("fixture prefix")
                            .parse()
                            .expect("fixture index");
                        browser.entry_at(0, index).is_some_and(|entry| {
                            entry.image_dimensions == MetadataValue::Known((32, 24))
                        })
                    })
                },
            );
            cancel_thumbnails_in(&view.widget());
            browser.clear_observer();
            window.destroy();
            clear_thumbnail_runtime();
        },
    );
}

#[test]
fn large_icon_scroll_fills_the_final_viewport_without_another_input() {
    crate::test_support::gtk_test(
        "ui::thumbnail::viewport::scroll_tests::large_icon_scroll_fills_the_final_viewport_without_another_input",
        || {
            let directory = tempfile::tempdir().expect("fixture");
            for index in 0..5000 {
                std::fs::write(
                    directory.path().join(format!("photo-{index:04}.png")),
                    b"fixture",
                )
                .expect("file");
            }
            clear_thumbnail_runtime();
            hold_thumbnail_workers();
            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                PeekBehavior::default(),
            );
            view.set_view_mode(BrowserMode::Icons);
            let browser = view.browser();
            let window = gtk::Window::builder()
                .child(&view.widget())
                .default_width(800)
                .default_height(500)
                .build();
            window.present();
            browser.navigate(Location::local(directory.path()));
            wait_until("directory did not load", || {
                browser
                    .column_snapshot(0)
                    .is_some_and(|snapshot| !snapshot.loading)
                    && has_pending_thumbnail(&directory.path().join("photo-0000.png"))
            });
            let grid = find_grid(&view.widget()).expect("icons grid");
            let scroll = viewport_of(&grid).expect("listing viewport");
            let adjustment = scroll.vadjustment();
            let last = directory.path().join("photo-4999.png");
            assert!(
                !has_pending_thumbnail(&last),
                "distant entries stay deferred"
            );
            for fraction in [0.4, 0.8, 1.0] {
                adjustment.set_value((adjustment.upper() - adjustment.page_size()) * fraction);
                // Let admission run before the next frame has necessarily allocated recycled cells.
                wait_until("viewport admission did not yield", || {
                    !REFRESH_PENDING.with(Cell::get)
                });
            }
            wait_until(
                "final viewport thumbnail stayed deferred after the flick",
                || has_pending_thumbnail(&last),
            );
            crate::ui::thumbnail::tests::complete_pending_thumbnail(&last);
            wait_until("final viewport metadata did not fill", || {
                browser
                    .entry_at(0, 4999)
                    .is_some_and(|entry| entry.size == MetadataValue::Known(7))
            });
            cancel_thumbnails_in(&view.widget());
            browser.clear_observer();
            window.destroy();
            clear_thumbnail_runtime();
        },
    );
}
