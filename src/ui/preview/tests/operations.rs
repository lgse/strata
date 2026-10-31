// SPDX-License-Identifier: MIT

use super::*;
use crate::adapters::ActiveOperation;

fn released(root: std::path::PathBuf) -> ActiveOperation {
    let operation = ActiveOperation::begin([]);
    glib::MainContext::default().block_on(operation.release(vec![root], &gio::Cancellable::new()));
    operation
}

#[test]
fn a_release_stops_the_preview_of_what_it_changes_until_the_operation_ends() {
    crate::test_support::gtk_test(
        "ui::preview::tests::operations::a_release_stops_the_preview_of_what_it_changes_until_the_operation_ends",
        || {
            let (directory, view) = super::video::sorted_listing(&["a.wav", "b.wav"]);
            let browser = view.browser();
            let provider = Rc::new(Provider::default());
            let drawer = PreviewDrawer::new(provider.clone(), false);
            drawer.observe_browser(&browser);
            let preferences = crate::ui::preferences::PreferenceManager::shared();
            preferences.set_preview_autoplay(false);
            preferences.set_single_click_previews(false);
            browser.select(0, 0);
            let shown = || {
                drawer
                    .state
                    .current
                    .borrow()
                    .as_ref()
                    .map(|entry| entry.display_name.clone())
            };
            let a = directory.path().join("a.wav");
            let links = tempfile::tempdir().expect("link folder");
            let link = links.path().join("music");
            std::os::unix::fs::symlink(directory.path(), &link).expect("linked folder");
            let wait_for_request = |row: &str, requested: usize| {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                while provider.0.borrow().len() == requested {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "{row}: never shown again"
                    );
                    glib::MainContext::default().iteration(false);
                }
            };
            let settle = || {
                let settled = std::time::Instant::now() + FOCUS_PREVIEW_DELAY * 4;
                while std::time::Instant::now() < settled {
                    glib::MainContext::default().iteration(false);
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            };
            let refuse_new = |requested: usize| {
                for index in requested..provider.0.borrow().len() {
                    super::video::failed(
                        &provider,
                        index,
                        crate::services::file_activity::HELD_FOR_OPERATION,
                    );
                }
            };
            for (row, root, stops, refills, shows) in [
                (
                    "a sibling",
                    directory.path().join("b.wav"),
                    false,
                    false,
                    "a.wav",
                ),
                ("the item", a.clone(), true, true, "a.wav"),
                (
                    "the item through a linked folder",
                    link.join("a.wav"),
                    true,
                    true,
                    "a.wav",
                ),
                (
                    "the item, refused through a linked folder while the operation runs",
                    a.clone(),
                    true,
                    true,
                    "a.wav",
                ),
                (
                    "its folder",
                    directory.path().to_path_buf(),
                    true,
                    true,
                    "a.wav",
                ),
                (
                    "the item, while another operation ends first",
                    a.clone(),
                    true,
                    true,
                    "a.wav",
                ),
                (
                    "the item, while another preview opens",
                    a.clone(),
                    true,
                    false,
                    "b.wav",
                ),
                (
                    "its folder, while a preview inside it is refused",
                    directory.path().to_path_buf(),
                    true,
                    true,
                    "b.wav",
                ),
                (
                    "its folder, while a preview inside it is refused after the operation ends",
                    directory.path().to_path_buf(),
                    true,
                    true,
                    "a.wav",
                ),
            ] {
                drawer.show(browser.entry_at(0, 0).expect("first track"), Some(0));
                super::video::ready(&provider, provider.0.borrow().len() - 1, "audio/x-wav");
                assert!(drawer.state.media.borrow().is_some(), "{row}");

                let operation = released(root);

                assert_eq!(shown().is_none(), stops, "{row}");
                assert_eq!(drawer.state.media.borrow().is_none(), stops, "{row}");
                let refused_after_end = match row {
                    "the item, refused through a linked folder while the operation runs" => {
                        let requested = provider.0.borrow().len();
                        drawer.show(
                            FileEntry {
                                location: Location::local(link.join("a.wav")),
                                ..browser.entry_at(0, 0).expect("first track")
                            },
                            Some(0),
                        );
                        refuse_new(requested);
                        settle();
                        assert_eq!(
                            provider.0.borrow().len(),
                            requested + 1,
                            "{row}: waits for the operation"
                        );
                        None
                    }
                    "the item, while another operation ends first" => {
                        let requested = provider.0.borrow().len();
                        drop(released(directory.path().join("elsewhere.wav")));
                        settle();
                        refuse_new(requested);
                        assert_eq!(shown(), None, "{row}: waits for the item's own operation");
                        None
                    }
                    "the item, while another preview opens" => {
                        drawer.show(browser.entry_at(0, 1).expect("second track"), Some(0));
                        super::video::ready(
                            &provider,
                            provider.0.borrow().len() - 1,
                            "audio/x-wav",
                        );
                        None
                    }
                    "its folder, while a preview inside it is refused" => {
                        let requested = provider.0.borrow().len();
                        browser.select(0, 1);
                        wait_for_request(row, requested);
                        refuse_new(requested);
                        assert_eq!(shown().as_deref(), Some("b.wav"), "{row}");
                        None
                    }
                    "its folder, while a preview inside it is refused after the operation ends" => {
                        let requested = provider.0.borrow().len();
                        browser.select(0, 0);
                        wait_for_request(row, requested);
                        Some(requested)
                    }
                    _ => None,
                };
                let requested = provider.0.borrow().len();
                drop(operation);
                if let Some(refused) = refused_after_end {
                    refuse_new(refused);
                }
                if refills {
                    wait_for_request(row, requested);
                } else {
                    settle();
                }
                let last = provider
                    .0
                    .borrow()
                    .last()
                    .map(|pending| pending.request.entry.location.clone());
                assert_eq!(
                    last,
                    Some(Location::local(directory.path().join(shows))),
                    "{row}"
                );
                assert_eq!(shown().as_deref(), Some(shows), "{row}");
            }
            drawer.close();
        },
    );
}
