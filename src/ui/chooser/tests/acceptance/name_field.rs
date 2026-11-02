// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn arrow_scope_keeps_left_in_the_chooser_file_view() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::name_field::arrow_scope_keeps_left_in_the_chooser_file_view",
        || {
            crate::ui::prepare_portal_ui();
            PreferenceManager::shared().set_arrow_navigation_scoped(true);
            let root = tempfile::tempdir().expect("fixture");
            for name in ["a.txt", "b.txt", "c.txt"] {
                std::fs::write(root.path().join(name), "text").expect("fixture file");
            }
            for mode in [BrowserMode::List, BrowserMode::Icons, BrowserMode::Columns] {
                PreferenceManager::shared().set_browser_mode(mode);
                let state = build_chooser(
                    request(root.path().to_path_buf()),
                    Arc::new(AtomicBool::new(false)),
                    |_| {},
                )
                .expect("chooser");
                state.view.set_view_mode(mode);
                let browser = state.view.browser();
                wait_column(&browser, 0, 3);
                for scoped in [true, false, true] {
                    PreferenceManager::shared().set_arrow_navigation_scoped(scoped);
                    for key in [gtk::gdk::Key::Left, gtk::gdk::Key::Up] {
                        browser.select(0, 0);
                        browser.focus_active();
                        wait_until(|| {
                            state.view.item_view_has_focus() && state.view.item_at_sidebar_edge()
                        });
                        press(&state.window, key);
                        assert_eq!(
                            state.view.item_view_has_focus(),
                            scoped,
                            "{mode:?}: {key:?} with arrow scope {scoped}"
                        );
                    }
                }
                state.window.close();
            }
        },
    );
}

#[test]
fn item_menu_offers_compress_for_native_folder() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::name_field::item_menu_offers_compress_for_native_folder",
        || {
            use gtk::prelude::*;
            crate::ui::prepare_portal_ui();
            let root = tempfile::tempdir().expect("fixture");
            std::fs::create_dir(root.path().join("folder")).expect("folder");
            std::fs::write(root.path().join("notes.txt"), "notes").expect("file");
            let state = build_chooser(
                request(root.path().to_path_buf()),
                Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .expect("chooser");
            let browser = state.view.browser();
            wait_column(&browser, 0, 2);
            browser.select(0, 0);
            browser.focus_active();
            let visible_popovers = || {
                descendants(state.window.upcast_ref())
                    .into_iter()
                    .filter_map(|widget| widget.downcast::<gtk::Popover>().ok())
                    .filter(|popover| popover.is_visible())
                    .collect::<Vec<_>>()
            };
            let before = visible_popovers()
                .iter()
                .map(|popover| popover.as_ptr())
                .collect::<Vec<_>>();
            wait_until(|| {
                if visible_popovers()
                    .iter()
                    .any(|candidate| !before.contains(&candidate.as_ptr()))
                {
                    return true;
                }
                state.view.open_focused_context_menu()
                    && visible_popovers()
                        .iter()
                        .any(|candidate| !before.contains(&candidate.as_ptr()))
            });
            let popover = visible_popovers()
                .into_iter()
                .find(|candidate| !before.contains(&candidate.as_ptr()))
                .expect("chooser item menu");
            wait_until(|| popover.is_mapped());
            let labels = descendants(popover.upcast_ref())
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
                .filter(|label| label.is_visible() && !label.text().is_empty())
                .map(|label| label.text().to_string())
                .collect::<Vec<_>>();
            assert!(
                labels.iter().any(|label| label == "Compress…"),
                "{labels:?}"
            );
            popover.popdown();
            state.window.close();
        },
    );
}

#[test]
fn pasted_url_in_open_name_field_downloads_and_returns_temp_path() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::name_field::pasted_url_in_open_name_field_downloads_and_returns_temp_path",
        || {
            crate::ui::prepare_portal_ui();
            PreferenceManager::shared().set_browser_mode(BrowserMode::List);
            let base = crate::test_support::serve_http_once(
                b"HTTP/1.1 200 OK\r\ncontent-length: 8\r\n\r\ncontents".to_vec(),
            );
            let root = tempfile::tempdir().expect("fixture");
            let (state, result) = open_chooser(request(root.path().to_path_buf()));
            let browser = state.view.browser();
            wait_listed(&browser, 0);

            let filename = state.filename.as_ref().expect("open name entry");
            let abandoned = crate::test_support::serve_http_once(
                b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n".to_vec(),
            );
            filename.set_text(&format!("{abandoned}/cancelled"));
            state.accept_button.emit_clicked();
            filename.set_text("local.txt");
            filename.emit_activate();
            state.activate_file(&Location::local(root.path().join("local.txt")));
            assert!(result.borrow().is_none());
            assert!(state.cancel_download());
            filename.set_text(&format!("{base}/pasted.zip"));
            let selected = accept(&state, &result);
            assert_eq!(selected.uris().len(), 1);
            let path = gio::File::for_uri(&selected.uris()[0].to_string())
                .path()
                .expect("local temp path");
            assert!(
                path.parent()
                    .and_then(|parent| parent.file_name())
                    .is_some_and(|name| name.to_string_lossy().starts_with("strata-download-"))
            );
            assert_eq!(
                path.file_name().expect("file name").to_string_lossy(),
                "pasted.zip"
            );
            assert_eq!(
                std::fs::read_to_string(&path).expect("downloaded body"),
                "contents"
            );
            let _cleanup = std::fs::remove_dir_all(path.parent().expect("download directory"));
            state.window.close();
        },
    );
}

#[test]
fn name_field_failures_leave_request_open_for_retry() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::name_field::name_field_failures_leave_request_open_for_retry",
        || {
            crate::ui::prepare_portal_ui();
            let root = tempfile::tempdir().expect("fixture");
            std::fs::write(root.path().join("notes.txt"), "notes").expect("fixture file");
            std::fs::create_dir(root.path().join("folder")).expect("fixture folder");
            let mut chooser_request = request(root.path().to_path_buf());
            chooser_request.filters = vec![FileFilter::new("Text").glob("*.txt")];
            std::fs::write(root.path().join("other.png"), "not text").expect("filtered file");
            let (state, result) = open_chooser(chooser_request);
            wait_until(|| {
                state
                    .view
                    .browser()
                    .column_snapshot(0)
                    .is_some_and(|column| !column.loading)
            });
            state.view.browser().select(0, 0);
            let filename = state.filename.as_ref().expect("open name field");
            for invalid in [
                "missing.txt",
                "other.png",
                "../notes.txt",
                "https://user:pass@example.com/file",
            ] {
                filename.set_text(invalid);
                filename.emit_activate();
                wait_until(|| !state.destination_check.get());
                assert!(state.error.is_visible(), "{invalid}");
                assert!(result.borrow().is_none(), "{invalid}");
            }
            let base = crate::test_support::serve_http_once(
                b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n".to_vec(),
            );
            filename.set_text(&format!("{base}/missing.txt"));
            filename.emit_activate();
            wait_until(|| !state.download_in_progress());
            assert!(state.error.is_visible());
            assert!(state.error.text().contains("404"));
            assert!(visible_modal_layer(&state.window).is_none());
            assert!(result.borrow().is_none());
            filename.set_text("folder");
            filename.emit_activate();
            wait_until(|| {
                state.view.browser().active_location()
                    == Some(Location::local(root.path().join("folder")))
            });
            assert!(result.borrow().is_none());
            state.view.browser().navigate(Location::local(root.path()));
            wait_until(|| {
                state
                    .view
                    .browser()
                    .column_snapshot(0)
                    .is_some_and(|column| !column.loading)
            });
            filename.set_text("notes.txt");
            filename.emit_activate();
            wait_until(|| result.borrow().is_some());
            let selected = result
                .borrow_mut()
                .take()
                .expect("result")
                .expect("accepted");
            let path = gio::File::for_uri(&selected.uris()[0].to_string())
                .path()
                .expect("local file");
            assert_eq!(path, root.path().join("notes.txt"));
            state.window.close();
        },
    );
}

#[test]
fn non_file_open_requests_reject_remote_downloads() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::name_field::non_file_open_requests_reject_remote_downloads",
        || {
            crate::ui::prepare_portal_ui();
            let root = tempfile::tempdir().expect("fixture");
            for kind in [
                ChooserKind::Open {
                    directory: true,
                    multiple: false,
                },
                ChooserKind::SaveFile { current_name: None },
                ChooserKind::SaveFiles {
                    names: vec!["one.txt".into()],
                },
            ] {
                let mut req = request(root.path().to_path_buf());
                req.kind = kind;
                let (state, result) = open_chooser(req);
                state.open_remote("https://example.com/file");
                assert!(state.error.is_visible());
                assert!(!state.download_in_progress());
                assert!(result.borrow().is_none());
                if matches!(state.request.kind, ChooserKind::SaveFile { .. }) {
                    state
                        .filename
                        .as_ref()
                        .expect("save name")
                        .set_text("https://example.com/file");
                    state.accept();
                    assert!(state.error.is_visible());
                    assert!(!state.download_in_progress());
                }
                state.window.close();
            }
        },
    );
}

#[test]
fn typed_name_survives_automatic_load_selection() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::name_field::typed_name_survives_automatic_load_selection",
        || {
            crate::ui::prepare_portal_ui();
            PreferenceManager::shared().set_browser_mode(BrowserMode::List);
            for first_is_directory in [false, true] {
                let root = tempfile::tempdir().expect("fixture");
                let first = root.path().join("aaa");
                if first_is_directory {
                    std::fs::create_dir(&first).expect("first folder");
                } else {
                    std::fs::write(&first, "first").expect("first file");
                }
                let target = root.path().join("typed.txt");
                std::fs::write(&target, "typed").expect("typed file");
                let (state, result) = open_chooser(request(root.path().to_path_buf()));
                let browser = state.view.browser();
                assert!(
                    browser
                        .column_snapshot(0)
                        .is_some_and(|column| column.loading)
                );
                let filename = state.filename.as_ref().expect("open name field");
                filename.set_text("typed.txt");
                wait_listed(&browser, 0);
                assert_eq!(filename.text(), "typed.txt");
                let selected = accept(&state, &result);
                let actual = gio::File::for_uri(&selected.uris()[0].to_string())
                    .path()
                    .expect("local path");
                assert_eq!(actual, target);
                state.window.close();
            }
        },
    );
}

#[test]
fn downloaded_files_respect_the_selected_filter() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::name_field::downloaded_files_respect_the_selected_filter",
        || {
            crate::ui::prepare_portal_ui();
            let base = crate::test_support::serve_http_once(
                b"HTTP/1.1 200 OK\r\ncontent-length: 8\r\n\r\ncontents".to_vec(),
            );
            let root = tempfile::tempdir().expect("fixture");
            let mut chooser_request = request(root.path().to_path_buf());
            chooser_request.filters = vec![FileFilter::new("Text").glob("*.txt")];
            let (state, result) = open_chooser(chooser_request);
            wait_until(|| {
                state
                    .view
                    .browser()
                    .column_snapshot(0)
                    .is_some_and(|column| !column.loading)
            });
            let filename = state.filename.as_ref().expect("open name field");
            filename.set_text(&format!("{base}/pasted.pdf"));
            state.accept_button.emit_clicked();
            wait_until(|| state.error.is_visible());
            assert!(result.borrow().is_none());
            assert_eq!(
                state.error.text(),
                "The file does not match the selected filter"
            );
            state.window.close();
        },
    );
}
