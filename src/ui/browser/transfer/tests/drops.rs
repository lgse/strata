// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::browser_modes::BrowserMode;

#[test]
fn drop_commit_kind_describes_the_pending_cursor_action() {
    assert_eq!(DropCommit::Copy.transfer_kind(), TransferKind::Copy);
    assert_eq!(DropCommit::Move.transfer_kind(), TransferKind::Move);
    assert_eq!(
        DropCommit::Ask {
            default: TransferKind::Copy,
            volume: VolumeRelation::Different,
        }
        .transfer_kind(),
        TransferKind::Copy
    );
}

#[test]
fn cross_volume_prompt_only_claims_another_device_when_the_lookup_resolved() {
    assert_eq!(
        cross_volume_drop_description(VolumeRelation::Different),
        "The destination is on a different device."
    );
    assert_eq!(
        cross_volume_drop_description(VolumeRelation::Unknown),
        "Strata could not determine whether the destination is on the same device."
    );
}

struct DropFixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    destination: PathBuf,
    source: PathBuf,
}

fn drop_fixture(name: &str, contents: &[u8]) -> DropFixture {
    let dir = tempfile::tempdir().expect(name);
    let root = dir.path().to_path_buf();
    let destination = root.join("destination");
    std::fs::create_dir(&destination).expect("drop destination");
    let source = root.join("file.txt");
    std::fs::write(&source, contents).expect("drop source");
    DropFixture {
        _dir: dir,
        root,
        destination,
        source,
    }
}

impl DropFixture {
    fn folder_after_drop(&self, opens_destination: bool) -> Location {
        if opens_destination {
            Location::local(&self.destination)
        } else {
            Location::local(&self.root)
        }
    }
}

struct DropCase<'a> {
    view: &'a crate::ui::browser::BrowserView,
    mode: BrowserMode,
    enabled: bool,
    moving: bool,
}

impl DropCase<'_> {
    fn drop_a_file(&self, fixture: &DropFixture) {
        let view = self.view;
        let events = observe_events(view);
        let commit = if self.moving {
            DropCommit::Move
        } else {
            DropCommit::Copy
        };
        view.state.commit_file_drop(
            Location::local(&fixture.destination),
            vec![Location::local(&fixture.source)],
            commit,
        );
        wait_until(|| finished(&events), "drop completion");
        assert_eq!(
            std::fs::read(fixture.destination.join("file.txt")).expect("dropped file"),
            b"dropped"
        );
        assert_eq!(fixture.source.exists(), !self.moving);
        assert_eq!(revealed(&events), self.enabled);
        let expected = fixture.folder_after_drop(self.enabled);
        wait_until(
            || view.browser().active_location() == Some(expected.clone()),
            "the revealed folder",
        );
        if self.enabled && self.mode == BrowserMode::Columns {
            assert_eq!(
                view.browser().location_at(0),
                Some(Location::local(&fixture.root))
            );
            assert_eq!(
                view.browser().location_at(1),
                Some(Location::local(&fixture.destination))
            );
        }
    }

    fn save_dropped_content(&self, fixture: &DropFixture) {
        let view = self.view;
        let expected = fixture.folder_after_drop(self.enabled);
        view.navigate_location(Location::local(&fixture.root));
        wait_until(
            || view.browser().active_location() == Some(Location::local(&fixture.root)),
            "the fixture folder before saving content",
        );
        view.commit_drop(
            Location::local(&fixture.destination),
            crate::ui::browser::DropRequest::Content(vec![crate::services::DroppedFile::text(
                "dropped",
                "text".to_owned(),
            )]),
        );
        let saved = Location::local(fixture.destination.join("dropped.txt"));
        let saved_path = saved.native_path().expect("local content").to_owned();
        wait_until(|| saved_path.exists(), "the saved content");
        wait_until(
            || view.browser().active_location() == Some(expected.clone()),
            "the folder after saving content",
        );
        if self.enabled {
            wait_until(
                || {
                    view.browser()
                        .selected_entries()
                        .iter()
                        .any(|entry| entry.location == saved)
                },
                "the saved content to be selected",
            );
        }
        assert_eq!(std::fs::read(&saved_path).expect("saved content"), b"text");
    }
}

#[test]
fn drop_open_preference_applies_before_settings_and_live_across_views() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::drops::drop_open_preference_applies_before_settings_and_live_across_views",
        || {
            crate::ui::preferences::PreferenceManager::seed_saved_preferences_for_test();
            let manager = crate::ui::preferences::PreferenceManager::shared();
            assert!(manager.open_folder_after_drop());
            let views: Vec<_> = (0..2).map(|_| operating_view()).collect();
            for enabled in [true, false, true] {
                manager.set_open_folder_after_drop(enabled);
                for mode in [BrowserMode::List, BrowserMode::Columns] {
                    for (index, view) in views.iter().enumerate() {
                        view.set_view_mode(mode);
                        let fixture = drop_fixture("drop fixture", b"dropped");
                        view.navigate_location(Location::local(&fixture.root));
                        let case = DropCase {
                            view,
                            mode,
                            enabled,
                            moving: index != 0,
                        };
                        case.drop_a_file(&fixture);
                        case.save_dropped_content(&fixture);
                    }
                }
            }
        },
    );
}

struct ConfirmedDrop {
    manager: Rc<crate::ui::preferences::PreferenceManager>,
    view: crate::ui::browser::BrowserView,
    overlay: gtk::Overlay,
    window: gtk::Window,
}

fn confirmed_drop() -> ConfirmedDrop {
    let manager = crate::ui::preferences::PreferenceManager::shared();
    assert!(!manager.open_folder_after_drop());
    let view = operating_view();
    view.set_view_mode(BrowserMode::List);
    let root = crate::ui::blur::BlurBin::new(&view.widget());
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&root));
    let window = gtk::Window::builder().child(&overlay).build();
    window.present();
    ConfirmedDrop {
        manager,
        view,
        overlay,
        window,
    }
}

impl ConfirmedDrop {
    fn observe_finished(&self) -> Rc<Cell<bool>> {
        let finished = Rc::new(Cell::new(false));
        let observed = finished.clone();
        self.view.browser().observe(move |event| {
            if matches!(
                operation_event(event),
                crate::app::BrowserEvent::TransferFinished { .. }
            ) {
                observed.set(true);
            }
        });
        finished
    }

    fn ask_across_devices(&self, fixture: &DropFixture) {
        self.view.state.commit_file_drop(
            Location::local(&fixture.destination),
            vec![Location::local(&fixture.source)],
            DropCommit::Ask {
                default: TransferKind::Copy,
                volume: VolumeRelation::Different,
            },
        );
        assert!(wait_for_modal_layer(&self.overlay));
    }

    fn cancel_leaves_everything_alone(&self, fixture: &DropFixture) {
        self.manager.set_open_folder_after_drop(false);
        self.ask_across_devices(fixture);
        click_button(&self.overlay, "Cancel");
        assert!(fixture.source.exists());
        assert!(!fixture.destination.join("file.txt").exists());
        assert!(
            !self.view.state.suppress_scroll_after_drop.get(),
            "cancelling confirmation must not suppress subsequent focus and reveal"
        );
    }

    fn confirm_reads_the_live_preference(
        &self,
        fixture: &DropFixture,
        finished: &Cell<bool>,
        (button, enabled): (&str, bool),
    ) {
        self.manager.set_open_folder_after_drop(!enabled);
        self.ask_across_devices(fixture);
        self.manager.set_open_folder_after_drop(enabled);
        click_button(&self.overlay, button);
        wait_until(|| finished.get(), "confirmed drop completion");
        assert_eq!(
            std::fs::read(fixture.destination.join("file.txt")).expect("confirmed file"),
            b"confirmed"
        );
        assert_eq!(fixture.source.exists(), button == "Copy");
        let expected = fixture.folder_after_drop(enabled);
        assert_eq!(self.view.browser().active_location(), Some(expected));
    }
}

#[test]
fn cross_device_confirmation_reads_the_live_drop_open_preference() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::drops::cross_device_confirmation_reads_the_live_drop_open_preference",
        || {
            let drop = confirmed_drop();
            for (button, enabled) in [("Copy", true), ("Move", false)] {
                let fixture = drop_fixture("confirmed drop fixture", b"confirmed");
                drop.view.navigate_location(Location::local(&fixture.root));
                let finished = drop.observe_finished();
                drop.cancel_leaves_everything_alone(&fixture);
                drop.confirm_reads_the_live_preference(&fixture, &finished, (button, enabled));
            }
            drop.window.destroy();
        },
    );
}

fn column_loaded(browser: &Browser, depth: usize) -> bool {
    browser
        .column_snapshot(depth)
        .is_some_and(|snapshot| !snapshot.loading)
}

fn open_child(browser: &Rc<Browser>, depth: usize, name: &str, child: &Path, what: &str) {
    assert!(browser.select_entries_by_name_at(depth, &[String::from(name)]));
    browser.descend(depth, Location::local(child));
    wait_until(|| column_loaded(browser, depth + 1), what);
}

#[test]
fn background_move_without_reveal_restores_the_source_column() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::drops::background_move_without_reveal_restores_the_source_column",
        || {
            crate::ui::preferences::PreferenceManager::seed_saved_preferences_for_test();
            let manager = crate::ui::preferences::PreferenceManager::shared();
            manager.set_open_folder_after_drop(false);
            let fixture = tempfile::tempdir().expect("drop fixture");
            let source_dir = fixture.path().join("source");
            let child_dir = source_dir.join("child");
            let destination = fixture.path().to_path_buf();
            std::fs::create_dir_all(&child_dir).expect("source directories");
            let source = child_dir.join("file.txt");
            std::fs::write(&source, b"dropped").expect("drop source");
            let view = operating_view();
            view.set_view_mode(BrowserMode::Columns);
            let browser = view.browser();
            browser.navigate(Location::local(fixture.path()));
            wait_until(|| column_loaded(&browser, 0), "root load");
            open_child(&browser, 0, "source", &source_dir, "source load");
            open_child(&browser, 1, "child", &child_dir, "child load");
            assert_eq!(browser.active_depth(), Some(2));
            let completed = Rc::new(Cell::new(false));
            let observed = completed.clone();
            browser.observe(move |event| {
                if matches!(
                    operation_event(event),
                    crate::app::BrowserEvent::TransferCompleted
                ) {
                    observed.set(true);
                }
            });
            view.state.drag_source_depth.set(Some(2));
            browser.set_active_column(0);
            view.state.commit_file_drop(
                Location::local(&destination),
                vec![Location::local(&source)],
                DropCommit::Move,
            );
            wait_until(
                || view.state.drop_active_depths.get().is_some(),
                "the move to start",
            );
            assert_eq!(view.state.drop_active_depths.get(), Some((2, 0)));

            wait_until(
                || {
                    glib::MainContext::default().iteration(false);
                    completed.get() && browser.active_depth() == Some(2)
                },
                "source focus restoration",
            );
            browser.clear_observer();
        },
    );
}

#[test]
fn dropping_a_folder_onto_its_namesake_merges_and_removes_the_source() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::drops::dropping_a_folder_onto_its_namesake_merges_and_removes_the_source",
        || {
            let dir = tempfile::tempdir().expect("merge drop fixture");
            let source = dir.path().join("b/Photos");
            let merged = dir.path().join("a/Photos");
            std::fs::create_dir_all(&source).expect("source folder");
            std::fs::create_dir_all(&merged).expect("destination folder");
            std::fs::write(source.join("two.txt"), b"two").expect("source file");
            std::fs::write(merged.join("one.txt"), b"one").expect("destination file");
            let (view, overlay, window) = conflict_view();

            view.state.commit_file_drop(
                Location::local(dir.path().join("a")),
                vec![Location::local(&source)],
                DropCommit::Move,
            );

            assert!(wait_for_modal_layer(&overlay), "the conflict prompt");
            assert!(
                visible_texts(&overlay)
                    .iter()
                    .any(|text| text.contains("Merging removes the source folder")),
                "the prompt must say a merged move can't be undone"
            );
            click_button(&overlay, "Merge");
            wait_until(
                || holds(&merged.join("two.txt"), b"two") && !source.exists(),
                "the merged move to finish",
            );
            assert_eq!(
                std::fs::read(merged.join("one.txt")).expect("destination-only file"),
                b"one",
                "destination-only contents survive the merged move"
            );
            window.destroy();
        },
    );
}
