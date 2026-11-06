// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn localized_overwrite_warning_preserves_caller_labels_and_cancelled_file() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::save::localized_overwrite_warning_preserves_caller_labels_and_cancelled_file",
        || {
            fn find<T: IsA<gtk::Widget> + glib::object::IsClass>(
                root: &gtk::Widget,
                predicate: &impl Fn(&T) -> bool,
            ) -> Option<T> {
                if let Some(widget) = root.downcast_ref::<T>()
                    && predicate(widget)
                {
                    return Some(widget.clone());
                }
                let mut child = root.first_child();
                while let Some(widget) = child {
                    child = widget.next_sibling();
                    if let Some(found) = find(&widget, predicate) {
                        return Some(found);
                    }
                }
                None
            }
            crate::ui::prepare_portal_ui();
            let preferences = PreferenceManager::shared();
            preferences.set_browser_mode(BrowserMode::List);
            rust_i18n::set_locale("fr");
            let root = tempfile::tempdir().expect("fixture");
            let path = root.path().join("Language");
            std::fs::write(&path, "original contents").expect("existing destination");
            let mut req = request(root.path().to_path_buf());
            req.title = "Language".into();
            req.accept_label = "Open".into();
            req.kind = ChooserKind::SaveFile {
                current_name: Some("Language".into()),
            };
            let (state, result) = open_chooser(req);
            assert_eq!(state.window.title().as_deref(), Some("Language"));
            assert_eq!(state.accept_button.label().as_deref(), Some("Open"));
            state.accept_button.emit_clicked();
            let warning = "Le fichier de destination existe déjà. Continuer peut l’écraser.";
            wait_until(|| {
                find(state.window.upcast_ref(), &|label: &gtk::Label| {
                    label
                        .text()
                        .split_whitespace()
                        .eq(warning.split_whitespace())
                })
                .is_some()
            });
            let dialog = find(state.window.upcast_ref(), &|widget: &gtk::Box| {
                widget.has_css_class("action-dialog")
            })
            .expect("overwrite dialog");
            assert!(
                find(dialog.upcast_ref(), &|label: &gtk::Label| label.text()
                    == "Language")
                .is_some()
            );
            find(dialog.upcast_ref(), &|button: &gtk::Button| {
                button.label().as_deref() == Some("Annuler") && button.is_visible()
            })
            .expect("localized cancellation")
            .emit_clicked();
            assert!(
                result.borrow().is_none(),
                "cancelling overwrite must not accept the request"
            );
            assert_eq!(
                std::fs::read_to_string(&path).expect("preserved file"),
                "original contents"
            );
            state.cancel();
        },
    );
}

#[test]
fn save_file_ignores_load_cursor() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::save::save_file_ignores_load_cursor",
        || {
            crate::ui::prepare_portal_ui();
            PreferenceManager::shared().set_browser_mode(BrowserMode::List);
            let root = tempfile::tempdir().expect("fixture");
            std::fs::create_dir(root.path().join("child")).expect("child folder");
            let mut save_request = request(root.path().to_path_buf());
            save_request.kind = ChooserKind::SaveFile {
                current_name: Some("output.txt".into()),
            };
            let (state, result) = open_chooser(save_request);
            let browser = state.view.browser();
            wait_column(&browser, 0, 1);
            assert!(browser.selection_is_load_cursor());
            let selected = accept(&state, &result);
            assert_single_uri(&selected, root.path().join("output.txt"), "");
            state.window.close();
        },
    );
}

struct SelectedFolderSave {
    state: Rc<ChooserState>,
    result: Completion,
    target_folder: PathBuf,
    _root: tempfile::TempDir,
}

/// A save chooser of `kind` in `mode` whose only entry, `target_folder`, is selected.
fn save_with_selected_folder(mode: BrowserMode, kind: ChooserKind) -> SelectedFolderSave {
    crate::ui::prepare_portal_ui();
    PreferenceManager::shared().set_browser_mode(mode);
    let root = tempfile::tempdir().expect("fixture");
    let target_folder = root.path().join("target_folder");
    std::fs::create_dir(&target_folder).expect("folder");
    let mut save_request = request(root.path().to_path_buf());
    save_request.kind = kind;
    let (state, result) = open_chooser(save_request);
    let browser = state.view.browser();
    wait_column(&browser, 0, 1);

    browser.select(0, 0);
    wait_until(|| {
        browser
            .selected_entries()
            .first()
            .is_some_and(|entry| entry.location == Location::local(&target_folder))
    });
    SelectedFolderSave {
        state,
        result,
        target_folder,
        _root: root,
    }
}

fn save_output_txt() -> ChooserKind {
    ChooserKind::SaveFile {
        current_name: Some("output.txt".into()),
    }
}

fn saves_output_into_the_selected_folder(mode: BrowserMode) {
    let save = save_with_selected_folder(mode, save_output_txt());
    let selected = accept(&save.state, &save.result);
    assert_single_uri(&selected, save.target_folder.join("output.txt"), "");
}

#[test]
fn save_file_accepts_selected_folder_without_navigating_into_it() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::save::save_file_accepts_selected_folder_without_navigating_into_it",
        || saves_output_into_the_selected_folder(BrowserMode::List),
    );
}

#[test]
fn save_files_accepts_selected_folder_without_navigating_into_it() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::save::save_files_accepts_selected_folder_without_navigating_into_it",
        || {
            let save = save_with_selected_folder(
                BrowserMode::List,
                ChooserKind::SaveFiles {
                    names: vec!["one.txt".into(), "two.txt".into()],
                },
            );
            let mut uris = accept(&save.state, &save.result)
                .uris()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            uris.sort();
            let target_folder = &save.target_folder;
            let mut expected = [target_folder.join("one.txt"), target_folder.join("two.txt")]
                .map(|path| gio::File::for_path(path).uri().to_string())
                .to_vec();
            expected.sort();
            assert_eq!(uris, expected);
        },
    );
}

#[test]
fn save_files_asks_to_replace_only_the_names_that_exist() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::save::save_files_asks_to_replace_only_the_names_that_exist",
        || {
            crate::ui::prepare_portal_ui();
            PreferenceManager::shared().set_browser_mode(BrowserMode::List);
            let root = tempfile::tempdir().expect("fixture");
            std::fs::write(root.path().join("notes.txt"), "kept").expect("existing file");
            let mut save_request = request(root.path().to_path_buf());
            save_request.kind = ChooserKind::SaveFiles {
                names: vec!["notes.txt".into(), "new-file.txt".into()],
            };
            let (state, _result) = open_chooser(save_request);
            let labels = || {
                descendants(state.window.upcast_ref())
                    .into_iter()
                    .filter(|widget| widget.has_css_class("action-dialog"))
                    .flat_map(|dialog| descendants(&dialog))
                    .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
                    .filter(|label| label.is_mapped())
                    .map(|label| label.text().to_string())
                    .collect::<Vec<_>>()
            };
            state.accept_button.emit_clicked();
            wait_until(|| labels().iter().any(|text| text == "Replace existing file?"));

            let labels = labels();
            assert!(labels.iter().any(|text| text == "notes.txt"), "{labels:?}");
            assert!(
                !labels.iter().any(|text| text.contains("new-file.txt")),
                "{labels:?}"
            );
            state.cancel();
        },
    );
}

#[test]
fn save_file_in_icons_mode_accepts_selected_folder_without_navigating_into_it() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::save::save_file_in_icons_mode_accepts_selected_folder_without_navigating_into_it",
        || saves_output_into_the_selected_folder(BrowserMode::Icons),
    );
}

#[test]
fn save_file_with_search_results_accepts_selected_folder_without_navigating_into_it() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::save::save_file_with_search_results_accepts_selected_folder_without_navigating_into_it",
        || {
            crate::ui::prepare_portal_ui();
            PreferenceManager::shared().set_browser_mode(BrowserMode::List);
            let root = tempfile::tempdir().expect("fixture");
            let target_folder = root.path().join("sub/target_folder");
            std::fs::create_dir_all(&target_folder).expect("folders");
            let mut save_request = request(root.path().to_path_buf());
            save_request.kind = ChooserKind::SaveFile {
                current_name: Some("output.txt".into()),
            };
            let (state, result) = open_chooser(save_request);
            let browser = state.view.browser();
            wait_listed(&browser, 0);

            state.view.show_filter_with_query("target");
            wait_until(|| {
                search_results(&state.view.widget()).is_some_and(|results| results.n_items() > 0)
            });
            let results = search_results(&state.view.widget()).expect("search results");
            results.select(0, true);
            wait_until(|| {
                state.view.selected_search_results().is_some_and(|entries| {
                    entries
                        .first()
                        .is_some_and(|e| e.location == Location::local(&target_folder))
                })
            });

            let selected = accept(&state, &result);
            assert_single_uri(&selected, target_folder.join("output.txt"), "");
        },
    );
}

fn shows_text(widget: &gtk::Widget, text: &str) -> bool {
    let label = widget
        .downcast_ref::<gtk::Label>()
        .is_some_and(|label| label.text() == text);
    label
        || widget
            .downcast_ref::<gtk::Inscription>()
            .is_some_and(|label| label.text().as_deref() == Some(text))
}

fn shown_label(root: &gtk::Widget, text: &str) -> Option<gtk::Widget> {
    descendants(root)
        .into_iter()
        .find(|widget| widget.is_mapped() && shows_text(widget, text))
}

fn is_primary_capture_click(click: &gtk::GestureClick) -> bool {
    matches!(click.button(), 0 | 1) && click.propagation_phase() == gtk::PropagationPhase::Capture
}

/// Presses `target` through `surface`'s capture-phase click gestures, once `surface`
/// picks it at that point.
fn press_through_capture(surface: &gtk::Widget, target: &gtk::Widget) {
    let target_point = || {
        target
            .compute_point(surface, &gtk::graphene::Point::new(2.0, 2.0))
            .expect("file position")
    };
    wait_until(|| {
        let point = target_point();
        surface.pick(
            f64::from(point.x()),
            f64::from(point.y()),
            gtk::PickFlags::DEFAULT,
        ) == Some(target.clone())
    });
    let point = target_point();
    let controllers = surface.observe_controllers();
    let clicks = (0..controllers.n_items())
        .filter_map(|index| controllers.item(index).and_downcast::<gtk::GestureClick>())
        .filter(is_primary_capture_click);
    for click in clicks {
        click.emit_by_name::<()>(
            "pressed",
            &[&1i32, &f64::from(point.x()), &f64::from(point.y())],
        );
    }
}

#[test]
fn save_file_clicking_the_selected_row_refills_the_name_in_every_view() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::save::save_file_clicking_the_selected_row_refills_the_name_in_every_view",
        || {
            crate::ui::prepare_portal_ui();
            for mode in [BrowserMode::List, BrowserMode::Columns, BrowserMode::Icons] {
                PreferenceManager::shared().set_browser_mode(mode);
                let root = tempfile::tempdir().expect("fixture");
                std::fs::write(root.path().join("existing.txt"), "existing").expect("file");
                let mut req = request(root.path().to_path_buf());
                req.kind = ChooserKind::SaveFile {
                    current_name: Some("suggested.txt".into()),
                };
                let state =
                    build_chooser(req, Arc::new(AtomicBool::new(false)), |_| {}).expect("chooser");
                let browser = state.view.browser();
                let file_label = || shown_label(&state.view.widget(), "existing.txt");
                wait_until(|| {
                    browser
                        .column_snapshot(0)
                        .is_some_and(|column| !column.loading && column.count == 1)
                        && file_label().is_some()
                });
                let filename = state.filename.as_ref().expect("filename");
                assert_eq!(filename.text(), "suggested.txt", "{mode:?}");
                assert!(browser.selection_is_load_cursor(), "{mode:?}");

                let click_file = || {
                    press_through_capture(&state.view.widget(), &file_label().expect("file label"))
                };
                click_file();
                wait_until(|| filename.text() == "existing.txt");
                assert!(!browser.selection_is_load_cursor(), "{mode:?}");
                filename.set_text("edited.txt");
                browser.set_selection(0, &[0], Some(0));
                while glib::MainContext::default().pending() {
                    glib::MainContext::default().iteration(false);
                }
                assert_eq!(
                    filename.text(),
                    "edited.txt",
                    "focus echoes must not refill: {mode:?}"
                );
                click_file();
                wait_until(|| filename.text() == "existing.txt");
                assert!(state.completion.borrow().is_some(), "{mode:?}");
                assert!(visible_modal_layer(&state.window).is_none(), "{mode:?}");
                state.window.close();
            }
        },
    );
}

#[test]
fn save_file_with_selected_file_saves_to_active_folder() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::save::save_file_with_selected_file_saves_to_active_folder",
        || {
            crate::ui::prepare_portal_ui();
            PreferenceManager::shared().set_browser_mode(BrowserMode::List);
            let root = tempfile::tempdir().expect("fixture");
            let existing_file = root.path().join("existing.txt");
            std::fs::write(&existing_file, "existing").expect("file");
            let mut save_request = request(root.path().to_path_buf());
            save_request.kind = ChooserKind::SaveFile {
                current_name: Some("new_file.txt".into()),
            };
            let (state, result) = open_chooser(save_request);
            let browser = state.view.browser();
            wait_column(&browser, 0, 1);

            let filename = state.filename.as_ref().expect("filename");
            assert_eq!(filename.text(), "new_file.txt");
            assert!(browser.selection_is_load_cursor());
            assert_eq!(browser.selected_entries().len(), 1);
            browser.commit_selection();
            browser.set_selection(0, &[0], Some(0));
            assert!(!browser.selection_is_load_cursor());
            wait_until(|| filename.text() == "existing.txt");
            for (invalid, message) in [("", "Enter a name"), ("bad/name", "Names cannot contain /")]
            {
                filename.set_text(invalid);
                state.accept_button.emit_clicked();
                assert!(result.borrow().is_none());
                assert!(filename.has_css_class("error"));
                assert!(filename.tooltip_text().is_none());
                assert!(state.error.is_visible());
                assert_eq!(state.error.text(), message);
                assert!(!root.path().join("bad").exists());
            }
            filename.set_text("new_file.txt");
            let selected = accept(&state, &result);
            assert_single_uri(&selected, root.path().join("new_file.txt"), "");
        },
    );
}

struct NonUtf8SaveFixture {
    state: Rc<ChooserState>,
    result: Rc<RefCell<Option<ashpd::backend::Result<SelectedFiles>>>>,
    e9: PathBuf,
    e8: PathBuf,
}

impl NonUtf8SaveFixture {
    fn open(root: &Path, current_name: &OsStr, e9: &Path, e8: &Path) -> Self {
        let mut save_request = request(root.to_path_buf());
        save_request.kind = ChooserKind::SaveFile {
            current_name: Some(current_name.to_owned()),
        };
        let (state, result) = open_chooser(save_request);
        let browser = state.view.browser();
        wait_column(&browser, 0, 2);
        Self {
            state,
            result,
            e9: e9.to_path_buf(),
            e8: e8.to_path_buf(),
        }
    }

    fn filename(&self) -> &gtk::Entry {
        self.state.filename.as_ref().expect("filename")
    }

    // The two names render identically, so their sort order is unspecified.
    fn select(&self, path: &Path) {
        let browser = self.state.view.browser();
        let position = (0..2)
            .find(|&position| {
                browser
                    .entry_at(0, position)
                    .is_some_and(|entry| entry.location.native_path() == Some(path))
            })
            .expect("fixture row");
        browser.commit_selection();
        browser.set_selection(0, &[position], Some(position));
        wait_until(|| *self.state.filename_selection.borrow() == [Location::local(path)]);
        assert_eq!(self.filename().text(), "caf\u{FFFD}.txt");
    }

    fn modal_button(&self, class: &str) -> gtk::Button {
        let layer = visible_modal_layer(&self.state.window).expect("confirmation");
        widget_with_class(&layer, class)
            .and_downcast::<gtk::Button>()
            .expect("confirmation button")
    }

    /// Clicks Save and confirms any Replace prompt; returns whether the prompt
    /// appeared and the URI the request completed with.
    fn save(&self) -> (bool, String) {
        self.state.accept_button.emit_clicked();
        wait_until(|| {
            visible_modal_layer(&self.state.window).is_some() || self.result.borrow().is_some()
        });
        let prompted = visible_modal_layer(&self.state.window).is_some();
        if prompted {
            self.modal_button("action-dialog-confirm").emit_clicked();
            wait_until(|| self.result.borrow().is_some());
        }
        let selected = self
            .result
            .borrow_mut()
            .take()
            .expect("result")
            .expect("accepted");
        assert_eq!(selected.uris().len(), 1);
        (prompted, selected.uris()[0].to_string())
    }
}

#[test]
fn save_file_keeps_the_exact_bytes_of_a_filled_non_utf8_name() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::acceptance::save::save_file_keeps_the_exact_bytes_of_a_filled_non_utf8_name",
        || {
            crate::ui::prepare_portal_ui();
            PreferenceManager::shared().set_browser_mode(BrowserMode::List);
            let root = tempfile::tempdir().expect("fixture");
            let uri = |path: &Path| gio::File::for_path(path).uri().to_string();
            let e9 = root.path().join(OsStr::from_bytes(b"caf\xe9.txt"));
            let e8 = root.path().join(OsStr::from_bytes(b"caf\xe8.txt"));
            std::fs::write(&e9, "e9").expect("e9 fixture");
            std::fs::write(&e8, "e8").expect("e8 fixture");
            let edited = root.path().join("caf\u{FFFD}-2.txt");

            let suggested = OsStr::new("new.txt");
            let e9_name = OsStr::from_bytes(b"caf\xe9.txt");
            type SaveCase<'a> = (&'a str, &'a OsStr, fn(&NonUtf8SaveFixture), (bool, String));
            let cases: [SaveCase; 6] = [
                ("suggested name", e9_name, |_| {}, (true, uri(&e9))),
                (
                    "select then save",
                    suggested,
                    |fixture| fixture.select(&fixture.e9),
                    (true, uri(&e9)),
                ),
                (
                    "activate, cancel Replace, then save",
                    suggested,
                    |fixture| {
                        fixture.state.activate_file(&Location::local(&fixture.e9));
                        wait_until(|| visible_modal_layer(&fixture.state.window).is_some());
                        fixture.modal_button("action-dialog-cancel").emit_clicked();
                        wait_until(|| visible_modal_layer(&fixture.state.window).is_none());
                        assert!(
                            fixture.result.borrow().is_none(),
                            "Cancel ended the request"
                        );
                        assert_eq!(fixture.filename().text(), "caf\u{FFFD}.txt");
                    },
                    (true, uri(&e9)),
                ),
                (
                    "last of two colliding selections",
                    suggested,
                    |fixture| {
                        fixture.select(&fixture.e9);
                        fixture.select(&fixture.e8);
                    },
                    (true, uri(&e8)),
                ),
                (
                    "edited name is literal text",
                    suggested,
                    |fixture| {
                        fixture.select(&fixture.e9);
                        fixture.filename().set_text("caf\u{FFFD}-2.txt");
                    },
                    (false, uri(&edited)),
                ),
                (
                    "edited then restored name",
                    suggested,
                    |fixture| {
                        fixture.select(&fixture.e9);
                        fixture.filename().set_text("x");
                        fixture.filename().set_text("caf\u{FFFD}.txt");
                    },
                    (true, uri(&e9)),
                ),
            ];
            let mut outcomes = Vec::new();
            let mut expected = Vec::new();
            for (case, current_name, fill, expect) in cases {
                let fixture = NonUtf8SaveFixture::open(root.path(), current_name, &e9, &e8);
                fill(&fixture);
                outcomes.push((case, fixture.save()));
                expected.push((case, expect));
            }
            assert_eq!(outcomes, expected);
        },
    );
}
