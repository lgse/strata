// SPDX-License-Identifier: MIT

use super::*;

struct SendToSources {
    _fixture: tempfile::TempDir,
    source_dir: PathBuf,
    destination: PathBuf,
    previous_mount_root: PathBuf,
    single: PathBuf,
    first: PathBuf,
    second: PathBuf,
}

fn send_to_sources() -> SendToSources {
    let fixture = tempfile::tempdir().expect("send-to fixture");
    let source_dir = fixture.path().join("source");
    let destination = fixture.path().join("device-root");
    let previous_mount_root = fixture.path().join("previous-mount-root");
    std::fs::create_dir(&source_dir).expect("source directory");
    std::fs::create_dir(&destination).expect("device root");
    std::fs::create_dir(&previous_mount_root).expect("previous mount root");
    let single = source_dir.join("single.txt");
    let first = source_dir.join("first.txt");
    let second = source_dir.join("second.txt");
    for (path, contents) in [(&single, "single"), (&first, "first"), (&second, "second")] {
        std::fs::write(path, contents).expect("source file");
    }
    SendToSources {
        _fixture: fixture,
        source_dir,
        destination,
        previous_mount_root,
        single,
        first,
        second,
    }
}

fn send_single_then_pair(
    view: &crate::ui::browser::BrowserView,
    sources: &SendToSources,
    events: &Events,
) {
    let destination = sources.destination.clone();
    view.state.send_to_removable_device_with_resolver(
        "volume:current-device",
        vec![Location::local(&sources.single)],
        |id| {
            assert_eq!(id, "volume:current-device");
            Some(destination.clone())
        },
    );
    wait_until(|| finished(events), "single-item send-to completion");
    view.state.send_to(
        Location::local(&sources.destination),
        vec![
            Location::local(&sources.first),
            Location::local(&sources.second),
        ],
        SendToTransferContext {
            device_name: "test-device".to_owned(),
        },
    );
    wait_until(
        || finished_count(events) == 2,
        "multi-selection send-to completion",
    );
}

fn assert_every_source_copied_in_place(sources: &SendToSources) {
    for (path, contents) in [
        (sources.destination.join("single.txt"), "single"),
        (sources.destination.join("first.txt"), "first"),
        (sources.destination.join("second.txt"), "second"),
    ] {
        assert_eq!(
            std::fs::read_to_string(path).expect("copied item"),
            contents
        );
    }
    assert!(
        [&sources.single, &sources.first, &sources.second]
            .iter()
            .all(|path| path.exists())
    );
    assert!(
        std::fs::read_dir(&sources.previous_mount_root)
            .expect("previous mount root")
            .next()
            .is_none(),
        "the menu-time mount root is not used"
    );
}

#[test]
fn send_to_copies_every_source_without_reveal_or_navigation() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::send_to::send_to_copies_every_source_without_reveal_or_navigation",
        || {
            let sources = send_to_sources();
            let view = operating_view();
            view.navigate_location(Location::local(&sources.source_dir));
            let browser = view.browser();
            let events = observe_events(&view);

            send_single_then_pair(&view, &sources, &events);

            assert_every_source_copied_in_place(&sources);
            assert_eq!(
                browser.active_location(),
                Some(Location::local(&sources.source_dir))
            );
            assert_eq!(started_totals(&events), [(1, false), (2, false)]);
            assert!(!revealed(&events));
            browser.clear_observer();
        },
    );
}

struct RemountedDevice {
    _fixture: tempfile::TempDir,
    source_dir: PathBuf,
    source: PathBuf,
    stale_root: PathBuf,
    current_root: PathBuf,
}

fn remounted_device() -> RemountedDevice {
    let fixture = tempfile::tempdir().expect("remount fixture");
    let source_dir = fixture.path().join("source");
    let stale_root = fixture.path().join("mount-a");
    let current_root = fixture.path().join("mount-b");
    std::fs::create_dir(&source_dir).expect("source directory");
    std::fs::create_dir(&stale_root).expect("stale mount root");
    std::fs::create_dir(&current_root).expect("current mount root");
    let source = source_dir.join("selected.txt");
    std::fs::write(&source, "selected contents").expect("source file");
    RemountedDevice {
        _fixture: fixture,
        source_dir,
        source,
        stale_root,
        current_root,
    }
}

fn assert_copied_to_the_current_root(device: &RemountedDevice) {
    assert_eq!(
        std::fs::read_to_string(device.current_root.join("selected.txt"))
            .expect("copy on remounted device"),
        "selected contents"
    );
    assert!(
        device.source.exists(),
        "Send to preserves the selected source"
    );
    assert!(
        std::fs::read_dir(&device.stale_root)
            .expect("stale mount root")
            .next()
            .is_none(),
        "the stale root is not trusted after remount"
    );
}

#[test]
fn send_to_drive_root_uses_current_root_after_same_id_remount() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::send_to::send_to_drive_root_uses_current_root_after_same_id_remount",
        || {
            let device = remounted_device();
            let device_id = "volume:remounted-device";
            let view = operating_view();
            view.navigate_location(Location::local(&device.source_dir));
            let browser = view.browser();
            let events = observe_events(&view);

            let current = device.current_root.clone();
            view.state.send_to_removable_device_with_resolver(
                device_id,
                vec![Location::local(&device.source)],
                |resolved_id| {
                    assert_eq!(resolved_id, device_id);
                    Some(current.clone())
                },
            );
            wait_until(
                || device.current_root.join("selected.txt").exists() && finished(&events),
                "drive-root copy to the current mount root",
            );

            assert_copied_to_the_current_root(&device);
            assert_eq!(
                browser.active_location(),
                Some(Location::local(&device.source_dir)),
                "Send to leaves the browser location unchanged"
            );
            assert_eq!(started_totals(&events), [(1, false)]);
            assert!(!revealed(&events), "Send to never reveals its destination");
            browser.clear_observer();
        },
    );
}

fn first_label_text(root: &gtk::Widget) -> Option<String> {
    walk(vec![root.clone()], |_| true)
        .iter()
        .find_map(|widget| widget.downcast_ref::<gtk::Label>())
        .map(|label| label.label().to_string())
}

fn send_to_toast_labels(overlay: &gtk::Overlay) -> Vec<String> {
    descendants(overlay)
        .iter()
        .filter(|widget| widget.has_css_class("overlay-notice"))
        .filter_map(first_label_text)
        .collect()
}

struct SendToToastFixture {
    _tempdir: tempfile::TempDir,
    view: crate::ui::browser::BrowserView,
    overlay: gtk::Overlay,
    window: gtk::Window,
    events: Events,
    source_dir: PathBuf,
    device: PathBuf,
}

fn open_send_to_toast_browser(fixture_name: &str, files: &[&str]) -> SendToToastFixture {
    // The toast is for sends that finish before delayed progress appears. Under
    // full-suite I/O load a two-file copy and device flush can outlast the real
    // 350 ms delay, which correctly suppresses the toast and fails these checks.
    // Visible-progress coverage uses an immediate-progress item count instead.
    crate::ui::browser::progress::set_file_progress_delay_for_test(std::time::Duration::from_secs(
        60,
    ));
    let tempdir = tempfile::tempdir().expect(fixture_name);
    let source_dir = tempdir.path().join("source");
    let device = tempdir.path().join("VANIA");
    std::fs::create_dir_all(&source_dir).expect("source directory");
    std::fs::create_dir_all(&device).expect("device directory");
    for name in files {
        std::fs::write(source_dir.join(name), name.as_bytes()).expect("source file");
    }
    let (view, overlay, window, events) = open_transfer_browser(&source_dir);
    SendToToastFixture {
        _tempdir: tempdir,
        view,
        overlay,
        window,
        events,
        source_dir,
        device,
    }
}

impl SendToToastFixture {
    fn sources(&self, names: &[&str]) -> Vec<Location> {
        names
            .iter()
            .map(|name| Location::local(self.source_dir.join(name)))
            .collect()
    }

    fn send_to_device(&self, device_id: &'static str, names: &[&str]) {
        let device_root = self.device.clone();
        self.view.state.send_to_removable_device_with_resolver(
            device_id,
            self.sources(names),
            move |id| (id == device_id).then(|| device_root.clone()),
        );
    }

    fn landed(&self, names: &[&str]) -> bool {
        names.iter().all(|name| self.device.join(name).exists())
    }

    fn close(self) {
        self.view.browser().clear_observer();
        self.window.destroy();
    }
}

#[test]
fn send_to_fast_copy_shows_transient_success() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::send_to::send_to_fast_copy_shows_transient_success",
        || {
            let fixture = open_send_to_toast_browser("send-to toast fixture", &["a.txt"]);
            fixture.send_to_device("volume:toast-device", &["a.txt"]);
            wait_until(
                || fixture.landed(&["a.txt"]) && finished(&fixture.events),
                "the fast send-to copy",
            );
            assert_eq!(
                send_to_toast_labels(&fixture.overlay),
                ["Copied to VANIA"],
                "a fast success shows the transient notice"
            );
            fixture.close();
        },
    );
}

/// Sends `a.txt` while the device's first access is parked, then lets it answer.
fn send_while_the_device_is_parked(
    fixture: &SendToToastFixture,
    recent: Option<&str>,
    copied: &Path,
) {
    let device_id = "volume:toast-device";
    let parked = crate::test_support::parked::Parked::default();
    let _device = crate::adapters::FsQuirks::install(
        &fixture.device,
        crate::adapters::Quirks {
            before: vec![(crate::adapters::FsOp::Access, "*", 0, parked.racer())],
            ..crate::adapters::Quirks::default()
        },
    );
    let device_root = fixture.device.clone();
    let resolve = move |id: &str| (id == device_id).then(|| device_root.clone());
    let sources = fixture.sources(&["a.txt"]);
    match recent {
        None => fixture
            .view
            .state
            .send_to_removable_device_with_resolver(device_id, sources, resolve),
        Some(recent) => fixture.view.state.send_to_recent_destination_with_resolver(
            device_id,
            Path::new(recent),
            sources,
            resolve,
        ),
    }
    wait_until(|| parked.reached(), "the device check");
    assert_eq!(parked.reached_on_ui_thread(), Some(false), "{recent:?}");
    assert!(
        !started(&fixture.events),
        "nothing is copied before the device answers: {recent:?}"
    );
    parked.release();
    wait_until(
        || copied.exists() && finished(&fixture.events),
        "the copy once the device answered",
    );
    fixture.events.borrow_mut().clear();
}

#[test]
fn send_to_checks_the_device_root_off_the_ui_thread_before_copying() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::send_to::send_to_checks_the_device_root_off_the_ui_thread_before_copying",
        || {
            let fixture = open_send_to_toast_browser("send-to check fixture", &["a.txt"]);
            std::fs::create_dir(fixture.device.join("Photos")).expect("recent folder");
            for (recent, copied) in [
                (None, fixture.device.join("a.txt")),
                (Some("Photos"), fixture.device.join("Photos/a.txt")),
            ] {
                send_while_the_device_is_parked(&fixture, recent, &copied);
            }
            assert_eq!(
                crate::ui::preferences::PreferenceManager::shared()
                    .send_to_recent_destinations("volume:toast-device")
                    .first()
                    .map(PathBuf::as_path),
                Some(Path::new("Photos"))
            );
            fixture.close();
        },
    );
}

#[test]
fn send_to_plural_copy_shows_item_count() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::send_to::send_to_plural_copy_shows_item_count",
        || {
            for (locale, expected) in [
                ("en", "3 items copied to VANIA"),
                ("ru", "Скопировано на VANIA: 3 элемента"),
            ] {
                let names = ["a.txt", "b.txt", "c.txt"];
                let fixture = open_send_to_toast_browser("send-to plural toast fixture", &names);
                rust_i18n::set_locale(locale);
                fixture.send_to_device("volume:toast-device", &names);
                wait_until(
                    || fixture.landed(&names) && finished(&fixture.events),
                    "the plural send-to copy",
                );
                assert_eq!(
                    send_to_toast_labels(&fixture.overlay),
                    [expected],
                    "a plural success counts the items in {locale}"
                );
                fixture.close();
            }
        },
    );
}

#[test]
fn send_to_with_visible_progress_shows_no_toast() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::send_to::send_to_with_visible_progress_shows_no_toast",
        || {
            let names: Vec<String> = (0..16)
                .map(|index| format!("file-{index:02}.txt"))
                .collect();
            let borrowed: Vec<&str> = names.iter().map(String::as_str).collect();
            let fixture = open_send_to_toast_browser("send-to progress fixture", &borrowed);
            let overlay = &fixture.overlay;
            fixture.send_to_device("volume:toast-device", &borrowed);
            wait_until(
                || find_widget_with_class(overlay, "file-operation-card").is_some(),
                "the progress card",
            );
            wait_until(
                || fixture.landed(&borrowed) && finished(&fixture.events),
                "the progress-covered send-to copy",
            );
            wait_until(
                || {
                    find_widget_with_class(overlay, "progress-complete")
                        .is_some_and(|widget| widget.is_visible())
                },
                "the completion card",
            );
            find_widget_with_class(overlay, "progress-complete")
                .and_downcast::<gtk::Button>()
                .expect("completion action")
                .emit_clicked();
            wait_until(
                || find_widget_with_class(overlay, "file-operation-card").is_none(),
                "the progress card closes",
            );
            assert!(
                send_to_toast_labels(overlay).is_empty(),
                "no transient notice follows visible progress"
            );
            fixture.close();
        },
    );
}

#[test]
fn normal_copy_fast_shows_no_send_to_toast() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::send_to::normal_copy_fast_shows_no_send_to_toast",
        || {
            let fixture = tempfile::tempdir().expect("normal copy toast fixture");
            let source_dir = fixture.path().join("source");
            let destination = fixture.path().join("destination");
            std::fs::create_dir_all(&source_dir).expect("source directory");
            std::fs::create_dir_all(&destination).expect("destination directory");
            let source = source_dir.join("photo.txt");
            std::fs::write(&source, b"photo").expect("source file");
            let (view, overlay, window, events) = open_transfer_browser(&source_dir);
            view.state
                .show_transfer_dialog(vec![transfer_entry(&source)], false);
            let (chooser_window, chooser) = destination_chooser(&window);
            navigate_destination(&chooser, &destination);
            confirm_destination(&chooser_window);
            wait_until(
                || destination.join("photo.txt").exists() && finished(&events),
                "the normal fast copy",
            );
            assert!(
                send_to_toast_labels(&overlay).is_empty(),
                "a normal copy never shows send-to feedback"
            );
            view.browser().clear_observer();
            window.destroy();
        },
    );
}

#[cfg(unix)]
#[test]
fn send_to_failure_shows_no_toast() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::send_to::send_to_failure_shows_no_toast",
        || {
            let fixture = open_send_to_toast_browser("send-to failure fixture", &["a.txt"]);
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                fixture.source_dir.join("a.txt"),
                std::fs::Permissions::from_mode(0o000),
            )
            .expect("unreadable source");
            fixture.send_to_device("volume:toast-device", &["a.txt"]);
            wait_until(
                || has_visible_button(&fixture.overlay, "Unable to complete operation"),
                "the failure dialog",
            );
            assert!(
                send_to_toast_labels(&fixture.overlay).is_empty(),
                "a failed send-to shows no success notice"
            );
            let events = fixture.events.borrow();
            assert!(
                events
                    .iter()
                    .any(|event| matches!(event, crate::app::BrowserEvent::OperationFailed { .. })),
                "the failure was reported"
            );
            assert!(
                events
                    .iter()
                    .all(|event| !matches!(event, crate::app::BrowserEvent::TransferCompleted)),
                "no successful completion was reported"
            );
            drop(events);
            fixture.close();
        },
    );
}

#[test]
fn send_to_repeated_completions_replace_toast() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::send_to::send_to_repeated_completions_replace_toast",
        || {
            let fixture = open_send_to_toast_browser(
                "send-to repeated toast fixture",
                &["a.txt", "b.txt", "c.txt"],
            );
            for (id, names) in [
                ("volume:toast-first", vec!["a.txt"]),
                ("volume:toast-second", vec!["b.txt", "c.txt"]),
            ] {
                fixture.send_to_device(id, &names);
                wait_until(
                    || fixture.landed(&names) && finished(&fixture.events),
                    "each repeated send-to copy",
                );
                fixture.events.borrow_mut().clear();
            }
            assert_eq!(
                send_to_toast_labels(&fixture.overlay),
                ["2 items copied to VANIA"],
                "the second success replaces the first notice"
            );
            fixture.close();
        },
    );
}

#[test]
fn overlapping_send_to_preserves_its_destination_and_feedback() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::send_to::overlapping_send_to_preserves_its_destination_and_feedback",
        || {
            let fixture =
                open_send_to_toast_browser("superseded send-to fixture", &["a.txt", "b.txt"]);
            let (source_dir, device) = (&fixture.source_dir, &fixture.device);
            std::fs::create_dir_all(device.join("other")).expect("plain destination");
            fixture.send_to_device("volume:toast-device", &["a.txt"]);
            fixture.view.state.start_transfer(
                Location::local(device.join("other")),
                fixture.sources(&["b.txt"]),
                false,
            );
            wait_until(
                || {
                    device.join("other/b.txt").exists()
                        && device.join("a.txt").exists()
                        && send_to_toast_labels(&fixture.overlay) == ["Copied to VANIA"]
                },
                "both independent copies complete with the Send-to destination's feedback",
            );
            assert_eq!(
                (
                    source_dir.join("a.txt").exists(),
                    source_dir.join("b.txt").exists(),
                    device.join("other/a.txt").exists(),
                    finished(&fixture.events),
                ),
                (true, true, false, true),
                "(a kept, b kept, a never sent to the plain destination, a transfer finished)"
            );
            fixture.close();
        },
    );
}
