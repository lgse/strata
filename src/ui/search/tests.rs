// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn only_rows_intersecting_the_viewport_are_visible() {
    assert!(intersects_viewport(100.0, 32.0, 90.0, 100.0));
    assert!(!intersects_viewport(58.0, 32.0, 90.0, 100.0));
    assert!(!intersects_viewport(190.0, 32.0, 90.0, 100.0));
}

#[test]
fn global_search_combines_home_and_drives_and_refreshes_mounts() {
    crate::test_support::gtk_test(
        "ui::search::tests::global_search_combines_home_and_drives_and_refreshes_mounts",
        || {
            let fixture = tempfile::tempdir().expect("search fixture");
            let home = fixture.path().join("Home");
            let usb = fixture.path().join("USB");
            for root in [&home, &usb] {
                std::fs::create_dir(root).expect("search root");
                std::fs::write(root.join("needle.txt"), b"fixture").expect("search file");
            }
            let activated = Rc::new(RefCell::new(None));
            let observed = activated.clone();
            let dialog = SearchDialog::new(
                Rc::new(move |item| {
                    observed.replace(Some(item));
                }),
                Rc::new(|| {}),
            );
            let window = gtk::Window::builder().child(&dialog.widget()).build();
            window.present();
            dialog.show(vec![home.clone(), usb.clone()], false);
            assert_eq!(
                dialog
                    .state
                    .field
                    .parent()
                    .expect("search bar")
                    .next_sibling(),
                Some(dialog.state.results.clone().upcast())
            );
            assert_eq!(
                dialog.state.status.text(),
                "Type to search Home and mounted local drives"
            );
            let tooltip = dialog.state.field.tooltip_text().expect("scope locations");
            assert!(tooltip.contains("Remote shares are not included."));
            assert!(tooltip.contains(&home.display().to_string()));
            assert!(tooltip.contains(&usb.display().to_string()));
            dialog.state.field.set_text("needle");
            wait_until(|| dialog.state.visible_results.borrow().len() == 2);
            for (position, item) in dialog.state.visible_results.borrow().iter().enumerate() {
                let row = dialog
                    .state
                    .list
                    .row_at_index(position as i32)
                    .expect("result row");
                assert_eq!(
                    row.tooltip_text().as_deref(),
                    Some(item.path.to_string_lossy().as_ref())
                );
            }

            dialog.show(vec![home.clone()], false);
            assert!(dialog.state.visible_results.borrow().is_empty());
            let tooltip = dialog
                .state
                .field
                .tooltip_text()
                .expect("updated scope locations");
            assert!(tooltip.contains(&home.display().to_string()));
            assert!(!tooltip.contains(&usb.display().to_string()));
            dialog.state.field.set_text("needle");
            wait_until(|| {
                !dialog.state.indexing_spinner.is_visible()
                    && dialog.state.visible_results.borrow().len() == 1
            });
            assert_eq!(
                dialog.state.visible_results.borrow()[0].path,
                home.join("needle.txt")
            );

            let coverage = SearchCoverage {
                unreadable: true,
                time_limit: true,
                ..Default::default()
            };
            render_results(&dialog.state, Vec::new(), false, coverage);
            assert!(dialog.state.truncated_hint.is_visible());
            assert_eq!(dialog.state.truncated_hint.text(), coverage.message());

            dialog.show(vec![home, usb.clone()], false);
            dialog.state.field.set_text("needle");
            wait_until(|| dialog.state.visible_results.borrow().len() == 2);
            let usb_position = dialog
                .state
                .visible_results
                .borrow()
                .iter()
                .position(|item| item.path.starts_with(&usb))
                .expect("USB result");
            activate_position(&dialog.state, usb_position as i32);
            assert_eq!(
                activated.borrow().as_ref().expect("activation").path,
                usb.join("needle.txt")
            );
            assert!(dialog.state.search.borrow().is_none());
            window.destroy();
        },
    );
}

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(
            std::time::Instant::now() < deadline,
            "search results timed out"
        );
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}
