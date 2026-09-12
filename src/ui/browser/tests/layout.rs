// SPDX-License-Identifier: MIT

use super::*;
use std::time::{Duration, Instant};

fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut widgets = vec![widget.clone()];
    let mut child = widget.first_child();
    while let Some(next) = child {
        widgets.extend(descendants(&next));
        child = next.next_sibling();
    }
    widgets
}

fn wait_until(ready: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "thumbnail size must reach the visible grid"
        );
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn thumbnail_size_control_updates_the_grid_and_survives_view_rebuild() {
    crate::test_support::gtk_test(
        "ui::browser::tests::layout::thumbnail_size_control_updates_the_grid_and_survives_view_rebuild",
        || {
            crate::ui::prepare_portal_ui();
            let fixture = tempfile::tempdir().expect("fixture");
            std::fs::create_dir(fixture.path().join("folder")).expect("folder");
            std::fs::write(fixture.path().join("report.txt"), b"report").expect("file");
            let view = BrowserView::new(
                Rc::new(crate::adapters::LocalFileSource),
                PeekBehavior::default(),
            );
            let browser = view.browser();
            let root = view.widget();
            let window = gtk::Window::builder()
                .child(&root)
                .default_width(1000)
                .default_height(380)
                .build();
            window.present();
            browser.navigate(Location::local(fixture.path()));
            wait_until(|| {
                browser
                    .column_snapshot(0)
                    .is_some_and(|snapshot| !snapshot.loading)
            });
            view.set_view_mode(BrowserMode::Icons);
            wait_until(|| {
                descendants(&root).iter().any(|widget| {
                    widget.has_css_class("icons-thumbnail-menu") && widget.is_mapped()
                })
            });
            let menu = descendants(&root)
                .into_iter()
                .find(|widget| widget.has_css_class("icons-thumbnail-menu") && widget.is_mapped())
                .expect("thumbnail menu")
                .downcast::<gtk::MenuButton>()
                .expect("menu button");
            let scale = descendants(menu.popover().expect("thumbnail popover").upcast_ref())
                .into_iter()
                .find_map(|widget| widget.downcast::<gtk::Scale>().ok())
                .expect("thumbnail scale");
            scale.set_value(32.0);
            let visible_slots_have_requested_size = || {
                let slots = descendants(&root)
                    .into_iter()
                    .filter_map(|widget| {
                        widget
                            .downcast::<crate::ui::thumbnail::ThumbnailSlot>()
                            .ok()
                    })
                    .filter(|slot| slot.is_mapped())
                    .collect::<Vec<_>>();
                !slots.is_empty() && slots.iter().all(|slot| slot.slot_size() == 32)
            };
            wait_until(visible_slots_have_requested_size);
            view.set_view_mode(BrowserMode::List);
            view.set_view_mode(BrowserMode::Icons);
            wait_until(visible_slots_have_requested_size);
            window.destroy();
            browser.clear_observer();
        },
    );
}
