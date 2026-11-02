// SPDX-License-Identifier: MIT

use super::*;

const CONFINED_DEVICE_ID: &str = "volume:current-device";

struct ConfinedDevice {
    _fixture: tempfile::TempDir,
    source_dir: PathBuf,
    device: PathBuf,
    outside: PathBuf,
    first: PathBuf,
    second: PathBuf,
}

fn confined_device() -> ConfinedDevice {
    let fixture = tempfile::tempdir().expect("choose-folder fixture");
    let source_dir = fixture.path().join("source");
    let device = fixture.path().join("device");
    let outside = fixture.path().join("outside");
    std::fs::create_dir_all(&source_dir).expect("source directory");
    std::fs::create_dir_all(device.join("subdir")).expect("device subdirectory");
    std::fs::create_dir_all(outside.join("outside-folder")).expect("outside directory");
    let first = source_dir.join("first.txt");
    let second = source_dir.join("second.txt");
    for path in [&first, &second] {
        let name = path
            .file_name()
            .expect("source file name")
            .to_string_lossy();
        std::fs::write(path, name.as_bytes()).expect("source file");
    }
    std::fs::write(device.join("regular-file.txt"), b"file").expect("device file");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, device.join("external-link"))
            .expect("external symlink");
        std::os::unix::fs::symlink(device.join("subdir"), device.join("internal-link"))
            .expect("internal symlink");
    }
    ConfinedDevice {
        _fixture: fixture,
        source_dir,
        device,
        outside,
        first,
        second,
    }
}

struct ConfinedSendTo {
    view: crate::ui::browser::BrowserView,
    window: gtk::Window,
    events: Events,
    chooser_window: gtk::Window,
    chooser: crate::ui::browser::BrowserView,
    preferences: Rc<crate::ui::preferences::PreferenceManager>,
    original_recents: Vec<PathBuf>,
    device: ConfinedDevice,
}

fn open_confined_send_to() -> ConfinedSendTo {
    let device = confined_device();
    let preferences = crate::ui::preferences::PreferenceManager::shared();
    preferences.remember_send_to_destination(
        CONFINED_DEVICE_ID,
        Path::new("Previously used"),
        None,
    );
    let original_recents = preferences.send_to_recent_destinations(CONFINED_DEVICE_ID);
    let (view, _overlay, window, events) = open_transfer_browser(&device.source_dir);

    let root = device.device.clone();
    view.state.show_send_to_folder_dialog_with_resolver(
        CONFINED_DEVICE_ID.to_owned(),
        vec![
            Location::local(&device.first),
            Location::local(&device.second),
        ],
        Rc::new(move |id| (id == CONFINED_DEVICE_ID).then(|| root.clone())),
    );
    let (chooser_window, chooser) = destination_chooser(&window);
    ConfinedSendTo {
        view,
        window,
        events,
        chooser_window,
        chooser,
        preferences,
        original_recents,
        device,
    }
}

impl ConfinedSendTo {
    fn listing_loaded(&self) -> bool {
        self.chooser
            .browser()
            .column_snapshot(0)
            .is_some_and(|snapshot| !snapshot.loading)
    }

    fn assert_listed_inside_device(&self) {
        let mut position = 0;
        while let Some(entry) = self.chooser.browser().entry_at(0, position) {
            assert!(entry.is_directory());
            assert!(
                entry
                    .location
                    .native_path()
                    .is_some_and(|path| path.starts_with(&self.device.device))
            );
            assert_ne!(
                entry.display_name, "outside-folder",
                "nothing outside the device is listed"
            );
            position += 1;
        }
    }

    fn assert_outside_paths_stay_confined(&self) {
        let device = &self.device.device;
        for path in [self.device.outside.clone(), device.join("../outside")] {
            self.chooser.navigate_location(Location::local(&path));
            drain_context();
            assert_eq!(
                self.chooser.browser().active_location(),
                Some(Location::local(device)),
                "a path outside the device stays confined"
            );
        }
    }

    fn invalid_folder_failed(&self, path: &Path) -> bool {
        self.chooser
            .browser()
            .column_snapshot(0)
            .is_some_and(|snapshot| {
                snapshot.location == Location::local(path)
                    && !snapshot.loading
                    && snapshot.error.is_some()
            })
    }

    fn assert_invalid_folders_fail_to_load(&self) {
        let device = &self.device.device;
        let mut invalid = vec![device.join("missing"), device.join("regular-file.txt")];
        #[cfg(unix)]
        invalid.push(device.join("external-link"));
        for path in invalid {
            self.chooser.navigate_location(Location::local(&path));
            wait_until(
                || self.invalid_folder_failed(&path),
                "an invalid folder fails to load",
            );
            self.assert_listed_inside_device();
            assert!(!started(&self.events));
            assert_eq!(
                self.preferences
                    .send_to_recent_destinations(CONFINED_DEVICE_ID),
                self.original_recents
            );
        }
    }

    #[cfg(unix)]
    fn assert_search_finds_nothing_outside(&self) {
        use crate::ui::browser_modes::BrowserMode;
        let chooser = &self.chooser;
        let original = chooser.view_mode();
        assert!(chooser.show_filter_with_query("outside"));
        for mode in [BrowserMode::Columns, BrowserMode::List] {
            chooser.set_view_mode(mode);
            wait_until(
                || {
                    chooser
                        .filter_status()
                        .flatten()
                        .is_some_and(|status| status.displayed)
                },
                "the search there settles",
            );
            settle(std::time::Duration::from_millis(500));
            assert_eq!(
                chooser
                    .filter_status()
                    .flatten()
                    .map(|status| status.files + status.folders),
                Some(0),
                "{mode:?}: a search there finds nothing outside the device"
            );
        }
        chooser.dismiss_focused_filter();
        chooser.set_view_mode(original);
    }

    fn copy_into_the_subdirectory(&self) {
        let device = &self.device.device;
        #[cfg(unix)]
        let selected = device.join("internal-link");
        #[cfg(not(unix))]
        let selected = device.join("subdir");
        navigate_destination(&self.chooser, &selected);
        confirm_destination(&self.chooser_window);
        wait_until(
            || {
                ["first.txt", "second.txt"]
                    .iter()
                    .all(|name| device.join("subdir").join(name).exists())
                    && finished(&self.events)
            },
            "the confined multi-source copy",
        );
    }

    fn assert_send_to_left_the_browser_alone(&self) {
        assert!(
            self.device.first.exists() && self.device.second.exists(),
            "sources remain in place"
        );
        assert_eq!(
            self.view.browser().active_location(),
            Some(Location::local(&self.device.source_dir)),
            "Send to does not navigate to its destination"
        );
        assert!(
            !revealed(&self.events),
            "Send to never reveals its destination"
        );
        let state = &self.view.state;
        assert!(state.pending_navigate.borrow().is_none());
        assert!(state.pending_select.borrow().is_empty());
    }
}

#[test]
fn choose_folder_rejects_invalid_destinations_and_copies_into_a_confined_directory() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::choose_folder::choose_folder_rejects_invalid_destinations_and_copies_into_a_confined_directory",
        || {
            let send_to = open_confined_send_to();
            assert_eq!(
                send_to.chooser.browser().active_location(),
                Some(Location::local(&send_to.device.device))
            );
            wait_until(|| send_to.listing_loaded(), "confined directory listing");
            send_to.assert_listed_inside_device();
            send_to.assert_outside_paths_stay_confined();
            send_to.assert_invalid_folders_fail_to_load();
            #[cfg(unix)]
            {
                send_to.assert_search_finds_nothing_outside();
            }
            send_to.chooser.create_new_folder();
            assert!(
                !send_to.chooser.new_entry_is_active(),
                "Send to cannot create folders"
            );
            assert!(!send_to.device.device.join("missing").exists());
            send_to.copy_into_the_subdirectory();
            send_to.assert_send_to_left_the_browser_alone();
            send_to.view.browser().clear_observer();
            send_to.window.destroy();
        },
    );
}

#[test]
fn choose_folder_revalidates_device_while_the_dialog_is_open() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::choose_folder::choose_folder_revalidates_device_while_the_dialog_is_open",
        || {
            let fixture = tempfile::tempdir().expect("removed-device fixture");
            let source = fixture.path().join("source.txt");
            let device = fixture.path().join("device");
            std::fs::write(&source, b"source").expect("source file");
            std::fs::create_dir_all(device.join("folder")).expect("device folder");
            let preferences = crate::ui::preferences::PreferenceManager::shared();
            preferences.remember_send_to_destination(
                "volume:removed-device",
                Path::new("Previously used"),
                None,
            );
            let original_recents = preferences.send_to_recent_destinations("volume:removed-device");
            let (view, _overlay, window, events) = open_unnavigated_transfer_browser();
            let current = Rc::new(RefCell::new(Some(device.clone())));
            let resolved = current.clone();
            view.state.show_send_to_folder_dialog_with_resolver(
                "volume:removed-device".to_owned(),
                vec![Location::local(&source)],
                Rc::new(move |id| {
                    (id == "volume:removed-device")
                        .then(|| resolved.borrow().clone())
                        .flatten()
                }),
            );
            let (chooser_window, chooser) = destination_chooser(&window);
            navigate_destination(&chooser, &device.join("folder"));
            let chooser_overlay = chooser_overlay(&chooser_window);
            current.replace(None);
            confirm_destination(&chooser_window);
            wait_until(
                || {
                    visible_error_message(&chooser_overlay).as_deref()
                        == Some("The removable device is no longer available.")
                },
                "the unavailable device error",
            );
            assert!(chooser_window.is_visible(), "the chooser stays open");
            assert!(
                !started(&events),
                "a removed device cannot dispatch a transfer"
            );
            assert!(!device.join("folder/source.txt").exists());
            let recents = preferences.send_to_recent_destinations("volume:removed-device");
            assert_eq!(
                recents, original_recents,
                "a device removed while the chooser is open does not change its recent destinations"
            );
            view.browser().clear_observer();
            window.destroy();
        },
    );
}

struct RemountedChooser {
    view: crate::ui::browser::BrowserView,
    window: gtk::Window,
    events: Events,
    chooser_window: gtk::Window,
    current: Rc<RefCell<Option<PathBuf>>>,
    source: PathBuf,
    opened_root: PathBuf,
    current_root: PathBuf,
    _fixture: tempfile::TempDir,
}

fn open_chooser_on_a_remounted_device() -> RemountedChooser {
    let fixture = tempfile::tempdir().expect("remount fixture");
    let source = fixture.path().join("source.txt");
    let opened_root = fixture.path().join("mount-a");
    let current_root = fixture.path().join("mount-b");
    std::fs::write(&source, b"source").expect("source file");
    std::fs::create_dir_all(opened_root.join("nested")).expect("folder on initial root");
    std::fs::create_dir_all(&current_root).expect("current root");
    let (view, _overlay, window, events) = open_unnavigated_transfer_browser();
    let current = Rc::new(RefCell::new(Some(opened_root.clone())));
    let resolved = current.clone();
    view.state.show_send_to_folder_dialog_with_resolver(
        "volume:remounted-device".to_owned(),
        vec![Location::local(&source)],
        Rc::new(move |_| resolved.borrow().clone()),
    );
    let (chooser_window, chooser) = destination_chooser(&window);
    navigate_destination(&chooser, &opened_root.join("nested"));
    RemountedChooser {
        view,
        window,
        events,
        chooser_window,
        current,
        source,
        opened_root,
        current_root,
        _fixture: fixture,
    }
}

#[test]
fn choose_folder_uses_the_current_root_after_a_remount() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::choose_folder::choose_folder_uses_the_current_root_after_a_remount",
        || {
            let remount = open_chooser_on_a_remounted_device();
            let chooser_overlay = chooser_overlay(&remount.chooser_window);
            let (current_root, events) = (&remount.current_root, &remount.events);
            remount.current.replace(Some(current_root.clone()));
            std::fs::remove_dir_all(&remount.opened_root).expect("old mount removed");
            confirm_destination(&remount.chooser_window);
            wait_until(
                || {
                    visible_texts(&chooser_overlay).iter().any(|text| {
                        text == "Choose an existing folder inside this removable device."
                    }) && destination_accept(&remount.chooser_window).is_sensitive()
                },
                "the missing corresponding folder error",
            );
            assert!(!current_root.join("nested").exists());
            assert!(
                !started(events),
                "a missing relative folder on the new mount is not created"
            );

            std::fs::create_dir(current_root.join("nested"))
                .expect("corresponding folder on current mount");
            confirm_destination(&remount.chooser_window);
            wait_until(
                || current_root.join("nested/source.txt").exists() && finished(events),
                "the transfer to the current mount root",
            );
            assert!(!remount.opened_root.exists());
            assert!(remount.source.exists());
            assert!(
                !revealed(events),
                "the remounted destination is not revealed"
            );
            remount.view.browser().clear_observer();
            remount.window.destroy();
        },
    );
}

#[test]
fn choose_folder_does_not_open_when_device_resolution_fails() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::choose_folder::choose_folder_does_not_open_when_device_resolution_fails",
        || {
            let source = Location::local("/tmp/source.txt");
            let view = local_view();
            let overlay = view.overlay();
            let window = gtk::Window::builder().child(&overlay).build();
            window.present();
            view.state.show_send_to_folder_dialog_with_resolver(
                "volume:missing-device".to_owned(),
                vec![source],
                Rc::new(|_| None),
            );
            wait_until(
                || has_visible_button(&overlay, "Close"),
                "the unavailable destination dialog",
            );
            assert!(has_visible_button(&overlay, "Destination unavailable"));
            assert!(button_with_label(&overlay, "Copy here").is_none());
            click_button(&overlay, "Close");
            window.destroy();
        },
    );
}

#[test]
fn choose_folder_navigation_and_cancellation_leave_parent_unchanged() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::choose_folder::choose_folder_navigation_and_cancellation_leave_parent_unchanged",
        || {
            let (_fixture, source_dir, device) =
                focused_enter_destination_fixture("chooser navigation fixture");
            let nested = device.join("nested");
            std::fs::create_dir(&nested).expect("nested destination");
            let (view, _overlay, window, events) = open_transfer_browser(&source_dir);
            let root = device.clone();
            view.state.show_send_to_folder_dialog_with_resolver(
                "volume:navigation".into(),
                vec![Location::local(source_dir.join("photo.txt"))],
                Rc::new(move |_| Some(root.clone())),
            );
            let (chooser_window, chooser) = destination_chooser(&window);
            navigate_destination(&chooser, &nested);
            chooser.browser().parent();
            wait_until(
                || chooser.browser().active_location() == Some(Location::local(&device)),
                "parent returns to device root",
            );
            chooser.browser().parent();
            drain_context();
            assert_eq!(
                chooser.browser().active_location(),
                Some(Location::local(&device)),
                "Parent cannot escape device root"
            );
            chooser.browser().back();
            wait_until(
                || chooser.browser().active_location() == Some(Location::local(&nested)),
                "Back returns to child",
            );
            chooser.browser().forward();
            wait_until(
                || chooser.browser().active_location() == Some(Location::local(&device)),
                "Forward returns to root",
            );
            chooser_window.close();
            drain_context();
            assert!(!chooser_window.is_visible());
            assert_eq!(
                view.browser().active_location(),
                Some(Location::local(&source_dir))
            );
            assert!(!started(&events));
            window.destroy();
        },
    );
}

#[test]
fn floating_destination_is_single_flight_and_dies_with_parent() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::choose_folder::floating_destination_is_single_flight_and_dies_with_parent",
        || {
            let (_fixture, source_dir, destination) =
                focused_enter_destination_fixture("chooser lifecycle fixture");
            let source = source_dir.join("photo.txt");
            let (view, _overlay, window, events) = open_transfer_browser(&source_dir);
            let open = || {
                view.state
                    .show_transfer_dialog(vec![transfer_entry(&source)], false)
            };
            open();
            open();
            let (chooser_window, chooser) = destination_chooser(&window);
            open();
            drain_context();
            let windows = gtk::Window::list_toplevels()
                .into_iter()
                .filter_map(|widget| widget.downcast::<gtk::Window>().ok())
                .filter(|candidate| {
                    candidate.is_visible() && candidate.transient_for().as_ref() == Some(&window)
                })
                .count();
            assert_eq!(windows, 1, "pending and active requests share one chooser");
            navigate_destination(&chooser, &destination);
            chooser_window.close();
            drain_context();
            open();
            let (reopened, _) = destination_chooser(&window);
            assert_ne!(
                reopened, chooser_window,
                "cancellation releases the per-parent lease"
            );
            window.destroy();
            drain_context();
            assert!(
                !reopened.is_visible(),
                "parent destruction destroys the chooser"
            );
            assert!(!destination.join("photo.txt").exists());
            assert!(!started(&events));
        },
    );
}
