// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(
            std::time::Instant::now() < deadline,
            "animation fixture timed out"
        );
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn survivors_remain_at_original_positions_until_dissolve_cleanup() {
    crate::test_support::gtk_test(
        "ui::browser::dissolve_delete::tests::survivors_remain_at_original_positions_until_dissolve_cleanup",
        || {
            gtk::Settings::default()
                .expect("GTK settings")
                .set_gtk_enable_animations(true);
            for completion in ["animated", "cancelled", "reduced-motion"] {
                crate::ui::motion::set_reduce_motion(false);
                let source = gtk::Box::new(gtk::Orientation::Vertical, 0);
                source.set_valign(gtk::Align::Start);
                let deleted = gtk::Label::new(Some("deleted.txt"));
                deleted.add_css_class("file-row");
                let survivor = gtk::Label::new(Some("survivor.txt"));
                source.append(&deleted);
                source.append(&survivor);
                let overlay = gtk::Overlay::new();
                overlay.set_child(Some(&source));
                let window = gtk::Window::builder().child(&overlay).build();
                window.present();
                let entry = FileEntry {
                    location: crate::model::Location::local("/deleted.txt"),
                    native_name: "deleted.txt".into(),
                    thumbnail_path: None,
                    display_name: "deleted.txt".into(),
                    kind: crate::model::EntryKind::File,
                    size: crate::model::MetadataValue::Unknown,
                    modified_unix_seconds: crate::model::MetadataValue::Unknown,
                    is_hidden: false,
                    mode: crate::model::MetadataValue::Unknown,
                };
                let prepared = RefCell::new(None);
                wait_until(|| {
                    if prepared.borrow().is_none() {
                        prepared.replace(prepare_dissolve(
                            source.upcast_ref(),
                            std::slice::from_ref(&entry),
                        ));
                    }
                    prepared.borrow().is_some()
                });
                let prepared = prepared.into_inner().expect("prepared dissolve");
                let original = bounds_in_overlay(survivor.upcast_ref(), &overlay)
                    .expect("original survivor bounds");
                let backdrop = prepared
                    .canvas
                    .imp()
                    .backdrop
                    .borrow()
                    .clone()
                    .expect("frozen survivor snapshot");
                assert!(backdrop.bounds().y() >= original.y());
                assert!(backdrop.bounds().y() < original.y() + original.height());
                source.remove(&deleted);
                wait_until(|| {
                    bounds_in_overlay(survivor.upcast_ref(), &overlay)
                        .expect("updated survivor bounds")
                        .y()
                        < original.y()
                });
                // The live layout has collapsed, but only the original-position snapshot is visible.
                assert_eq!(source.opacity(), 0.0);
                assert!(prepared.canvas.is_mapped());
                assert!(backdrop.bounds().y() >= original.y());
                let canvas = prepared.canvas.clone();
                assert_eq!(
                    overlay.pick(
                        f64::from(original.x() + original.width() / 2.0),
                        f64::from(original.y() + original.height() / 2.0),
                        gtk::PickFlags::DEFAULT,
                    ),
                    Some(canvas.clone().upcast()),
                    "the frozen presentation must shield rebound rows from pointer input"
                );
                if completion == "cancelled" {
                    drop(prepared);
                } else {
                    crate::ui::motion::set_reduce_motion(completion == "reduced-motion");
                    prepared.play(|| {});
                    wait_until(|| canvas.parent().is_none());
                }
                assert_eq!(source.opacity(), 1.0);
                assert!(canvas.parent().is_none());
                window.destroy();
            }
            crate::ui::motion::set_reduce_motion(false);
        },
    );
}

#[test]
fn fragment_budget_is_bounded_for_large_batches() {
    assert_eq!(fragment_budget(1), MIN_FRAGMENT_BUDGET);
    assert_eq!(fragment_budget(4), MAX_FRAGMENT_BUDGET);
    assert_eq!(fragment_budget(1_000), MAX_FRAGMENT_BUDGET);
}

#[test]
fn one_row_stays_below_one_hundred_animated_fragments() {
    let width = 300.0;
    let height = 28.0;
    let tile = tile_size_for_budget(f64::from(width * height), fragment_budget(1));
    let mut random = Random::new(5);

    let fragments = fragments_for_size(width, height, tile, 0, &mut random);

    assert!(fragments.len() <= 100);
}

#[test]
fn viewport_batch_keeps_a_small_fixed_rendering_budget() {
    let rows = 20;
    let width = 300.0;
    let height = 28.0;
    let tile = tile_size_for_budget(
        f64::from(width * height) * rows as f64,
        fragment_budget(rows),
    );
    let mut random = Random::new(9);
    let fragments: usize = (0..rows)
        .map(|row| fragments_for_size(width, height, tile, row, &mut random).len())
        .sum();

    assert!(fragments <= 240);
}

#[test]
fn only_bounds_intersecting_the_viewport_animate() {
    assert!(bounds_intersect_viewport(
        gtk::graphene::Rect::new(10.0, 10.0, 300.0, 28.0),
        800,
        600,
    ));
    assert!(!bounds_intersect_viewport(
        gtk::graphene::Rect::new(10.0, 620.0, 300.0, 28.0),
        800,
        600,
    ));
}

#[test]
fn fragments_cover_the_complete_row() {
    let mut random = Random::new(7);
    let fragments = fragments_for_size(303.0, 29.0, 10.0, 0, &mut random);

    let right = fragments
        .iter()
        .map(|fragment| fragment.source.x() + fragment.source.width())
        .fold(0.0, f32::max);
    let bottom = fragments
        .iter()
        .map(|fragment| fragment.source.y() + fragment.source.height())
        .fold(0.0, f32::max);

    assert_eq!(right, 303.0);
    assert_eq!(bottom, 29.0);
}
