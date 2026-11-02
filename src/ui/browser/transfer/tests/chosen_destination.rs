// SPDX-License-Identifier: MIT

use super::*;
use std::sync::{Arc, Mutex, mpsc::Sender};

struct FocusedEnter {
    view: crate::ui::browser::BrowserView,
    window: gtk::Window,
    events: Events,
    source_dir: PathBuf,
    destination: PathBuf,
    source: PathBuf,
    _fixture: tempfile::TempDir,
}

fn focused_enter(name: &str) -> FocusedEnter {
    let (fixture, source_dir, destination) = focused_enter_destination_fixture(name);
    let source = source_dir.join("photo.txt");
    let (view, _overlay, window, events) = open_transfer_browser(&source_dir);
    FocusedEnter {
        view,
        window,
        events,
        source_dir,
        destination,
        source,
        _fixture: fixture,
    }
}

impl FocusedEnter {
    fn transfer_dialog_then_focused_enter(&self, moving: bool) {
        self.view
            .state
            .show_transfer_dialog(vec![transfer_entry(&self.source)], moving);
        let (chooser_window, chooser) = destination_chooser(&self.window);
        navigate_destination(&chooser, &self.destination);
        activate_focused_accept(&chooser_window);
    }

    fn wait_landed(&self, what: &str) {
        wait_until(
            || self.destination.join("photo.txt").exists() && finished(&self.events),
            what,
        );
    }

    fn close(self) {
        self.view.browser().clear_observer();
        self.window.destroy();
    }
}

#[test]
fn copy_to_focused_enter_confirms_destination() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::chosen_destination::copy_to_focused_enter_confirms_destination",
        || {
            let enter = focused_enter("copy focused-enter fixture");
            enter.transfer_dialog_then_focused_enter(false);
            enter.wait_landed("focused Enter copies to the destination");
            assert!(enter.source.exists(), "Copy leaves its source");
            assert_eq!(
                enter.view.browser().active_location(),
                Some(Location::local(&enter.destination)),
                "Copy navigates to its destination"
            );
            enter.close();
        },
    );
}

#[test]
fn move_to_focused_enter_confirms_destination() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::chosen_destination::move_to_focused_enter_confirms_destination",
        || {
            let enter = focused_enter("move focused-enter fixture");
            enter.transfer_dialog_then_focused_enter(true);
            enter.wait_landed("focused Enter moves to the destination");
            assert!(!enter.source.exists(), "Move removes its source");
            enter.close();
        },
    );
}

#[test]
fn send_to_focused_enter_copies_to_device_root() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::chosen_destination::send_to_focused_enter_copies_to_device_root",
        || {
            let enter = focused_enter("send-to focused-enter fixture");
            let device_id = "volume:focused-enter-device";
            let root = enter.destination.clone();
            enter.view.state.show_send_to_folder_dialog_with_resolver(
                device_id.to_owned(),
                vec![Location::local(&enter.source)],
                Rc::new(move |id| (id == device_id).then(|| root.clone())),
            );
            let (chooser_window, _chooser) = destination_chooser(&enter.window);
            activate_focused_accept(&chooser_window);
            enter.wait_landed("focused Enter copies the empty path to the device root");
            assert!(enter.source.exists(), "sources remain in place");
            assert_eq!(
                enter.view.browser().active_location(),
                Some(Location::local(&enter.source_dir)),
                "Send to does not navigate to its destination"
            );
            enter.close();
        },
    );
}

#[test]
fn extract_to_focused_enter_extracts_destination() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::chosen_destination::extract_to_focused_enter_extracts_destination",
        || {
            let fixture = tempfile::tempdir().expect("extract focused-enter fixture");
            let work = fixture.path().join("work");
            let destination = fixture.path().join("destination");
            std::fs::create_dir_all(&work).expect("work directory");
            std::fs::create_dir_all(&destination).expect("destination directory");
            std::fs::write(work.join("notes.txt"), b"notes").expect("archived file");
            let archive = work.join("bundle.tar");
            crate::adapters::write_compression_fixture(
                &archive,
                &[work.join("notes.txt")],
                crate::services::ArchiveFormat::Tar,
                None,
            )
            .expect("fixture archive");
            let (view, overlay, window, _events) = open_transfer_browser(&work);
            view.state.show_extract_to_dialog(transfer_entry(&archive));
            let (chooser_window, chooser) = destination_chooser(&window);
            navigate_destination(&chooser, &destination);
            activate_focused_accept(&chooser_window);
            wait_until(
                || {
                    std::fs::read_to_string(destination.join("notes.txt"))
                        .is_ok_and(|contents| contents == "notes" && !modal_layer_shown(&overlay))
                },
                "focused Enter extracts the archived contents and closes its dialog",
            );
            view.browser().clear_observer();
            window.destroy();
        },
    );
}

#[derive(Clone, Copy)]
enum ChosenFor {
    Copy,
    Move,
    Extract,
    SendTo,
}

#[derive(Clone, Copy)]
enum ChosenFolder {
    SlowToAnswer,
    Failing(rustix::io::Errno, &'static str),
    WriteOnly,
}

fn chosen_cases() -> [(&'static str, ChosenFor, ChosenFolder); 7] {
    [
        (
            "copy to a folder slow to answer",
            ChosenFor::Copy,
            ChosenFolder::SlowToAnswer,
        ),
        (
            "extract to a folder slow to answer",
            ChosenFor::Extract,
            ChosenFolder::SlowToAnswer,
        ),
        (
            "send to a device slow to answer",
            ChosenFor::SendTo,
            ChosenFolder::SlowToAnswer,
        ),
        (
            "copy to a folder that is gone",
            ChosenFor::Copy,
            ChosenFolder::Failing(rustix::io::Errno::NOENT, "Choose an existing folder."),
        ),
        (
            "extract to a folder that fails",
            ChosenFor::Extract,
            ChosenFolder::Failing(rustix::io::Errno::IO, "Unable to open that location: "),
        ),
        (
            "move to a write-only folder",
            ChosenFor::Move,
            ChosenFolder::WriteOnly,
        ),
        (
            "extract to a write-only folder",
            ChosenFor::Extract,
            ChosenFolder::WriteOnly,
        ),
    ]
}

type Started = Option<crate::services::OperationRequestId>;

struct ChosenFolderCheck {
    case: &'static str,
    chosen_for: ChosenFor,
    chosen: ChosenFolder,
    _chooser: crate::ui::browser::BrowserView,
    chooser_window: gtk::Window,
    events: Events,
    window: gtk::Window,
    view: crate::ui::browser::BrowserView,
    destination: PathBuf,
    _fixture: tempfile::TempDir,
}

fn show_chooser_for(
    view: &crate::ui::browser::BrowserView,
    chosen_for: ChosenFor,
    work: &Path,
    destination: &Path,
) {
    let source = work.join("photo.txt");
    match chosen_for {
        ChosenFor::Copy | ChosenFor::Move => view.state.show_transfer_dialog(
            vec![transfer_entry(&source)],
            matches!(chosen_for, ChosenFor::Move),
        ),
        ChosenFor::Extract => view
            .state
            .show_extract_to_dialog(transfer_entry(&work.join("bundle.tar"))),
        ChosenFor::SendTo => {
            let root = destination.to_owned();
            view.state.show_send_to_folder_dialog_with_resolver(
                "volume:chosen-device".to_owned(),
                vec![Location::local(&source)],
                Rc::new(move |_| Some(root.clone())),
            );
        }
    }
}

fn open_chooser_for(
    case: &'static str,
    chosen_for: ChosenFor,
    chosen: ChosenFolder,
) -> ChosenFolderCheck {
    let (fixture, work, destination) = focused_enter_destination_fixture("chosen folder fixture");
    crate::adapters::write_compression_fixture(
        &work.join("bundle.tar"),
        &[work.join("photo.txt")],
        crate::services::ArchiveFormat::Tar,
        None,
    )
    .expect("fixture archive");
    let (view, _overlay, window, events) = open_transfer_browser(&work);
    show_chooser_for(&view, chosen_for, &work, &destination);
    let (chooser_window, chooser) = destination_chooser(&window);
    navigate_destination(&chooser, &destination);
    wait_until(
        || {
            chooser
                .browser()
                .column_snapshot(0)
                .is_some_and(|snapshot| !snapshot.loading)
                && chooser.browser().column_has_monitor(0)
        },
        &format!("{case}: the chosen folder's listing and monitor"),
    );
    ChosenFolderCheck {
        case,
        chosen_for,
        chosen,
        _chooser: chooser,
        chooser_window,
        events,
        window,
        view,
        destination,
        _fixture: fixture,
    }
}

impl ChosenFolderCheck {
    fn set_write_only_destination_mode(&self, mode: u32, what: &str) {
        use std::os::unix::fs::PermissionsExt;
        if matches!(self.chosen, ChosenFolder::WriteOnly) {
            std::fs::set_permissions(&self.destination, std::fs::Permissions::from_mode(mode))
                .expect(what);
        }
    }

    /// Records the thread that stamps the chosen folder, holds a slow folder
    /// until released and fails a failing one.
    fn hold_the_folder_check(
        &self,
    ) -> (
        Sender<()>,
        Arc<Mutex<Option<bool>>>,
        crate::adapters::FsQuirks,
    ) {
        let (release, held) = std::sync::mpsc::channel::<()>();
        let checked = Arc::new(Mutex::new(None));
        let seen = checked.clone();
        let slow = matches!(self.chosen, ChosenFolder::SlowToAnswer);
        let scope = crate::adapters::FsQuirks::install(
            self.destination.parent().expect("fixture root"),
            crate::adapters::Quirks {
                before: vec![(
                    crate::adapters::FsOp::FolderStamp,
                    "destination",
                    0,
                    Box::new(move || {
                        *seen.lock().expect("check thread") =
                            Some(crate::util::ui_thread::is_current());
                        if slow {
                            let _released = held.recv();
                        }
                    }),
                )],
                fail: match self.chosen {
                    ChosenFolder::Failing(error, _) => {
                        vec![(crate::adapters::FsOp::FolderStamp, "destination", 0, error)]
                    }
                    ChosenFolder::SlowToAnswer | ChosenFolder::WriteOnly => Vec::new(),
                },
                ..crate::adapters::Quirks::ext4()
            },
        );
        (release, checked, scope)
    }

    fn last_started(&self) -> Started {
        self.view.browser().last_started_operation()
    }

    fn expect_waits_then_lands(&self, started: Started, release: &Sender<()>) {
        let case = self.case;
        iterate_context(20);
        let accept_sensitive = destination_accept(&self.chooser_window).is_sensitive();
        assert!(
            self.chooser_window.is_visible() && !accept_sensitive,
            "{case}: the chooser waits for the folder, open and without a second accept"
        );
        assert_eq!(
            self.last_started(),
            started,
            "{case}: nothing starts before the folder answers"
        );
        release.send(()).expect("held check");
        wait_until(
            || self.destination.join("photo.txt").exists(),
            &format!("{case}: the item to land in the chosen folder"),
        );
    }

    fn expect_stays_open_saying(&self, started: Started, message: &str) {
        let case = self.case;
        let chooser_overlay = chooser_overlay(&self.chooser_window);
        wait_until(
            || {
                visible_error_message(&chooser_overlay)
                    .is_some_and(|shown| shown.starts_with(message))
            },
            &format!("{case}: the chooser to say what is wrong"),
        );
        let accept_sensitive = destination_accept(&self.chooser_window).is_sensitive();
        assert!(
            self.chooser_window.is_visible() && accept_sensitive,
            "{case}: the chooser stays open for another folder"
        );
        assert_eq!(self.last_started(), started);
        assert!(!self.destination.join("photo.txt").exists());
    }

    fn extraction_failed_to_open(&self) -> bool {
        self.events.borrow().iter().any(|event| {
            matches!(
                event,
                crate::app::BrowserEvent::OperationFailed { message, .. }
                    if message.starts_with("Could not open extraction destination")
            )
        })
    }

    fn expect_accepts_the_write_only_folder(&self, started: Started) {
        let case = self.case;
        wait_until(
            || !self.chooser_window.is_visible() && self.last_started() != started,
            &format!("{case}: the chooser to accept the write-only folder"),
        );
        if matches!(self.chosen_for, ChosenFor::Extract) {
            wait_until(
                || self.extraction_failed_to_open(),
                &format!("{case}: extraction, which lists its destination, to say so"),
            );
        } else {
            wait_until(
                || self.destination.join("photo.txt").exists(),
                &format!("{case}: the item to land in the write-only folder"),
            );
        }
    }
}

#[test]
fn a_chosen_folder_is_checked_off_the_ui_thread_while_the_chooser_stays_open() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::chosen_destination::a_chosen_folder_is_checked_off_the_ui_thread_while_the_chooser_stays_open",
        || {
            for (case, chosen_for, chosen) in chosen_cases() {
                let check = open_chooser_for(case, chosen_for, chosen);
                check.set_write_only_destination_mode(0o300, "write-only destination");
                let (release, checked, _scope) = check.hold_the_folder_check();
                let started = check.last_started();

                confirm_destination(&check.chooser_window);

                wait_until(
                    || checked.lock().expect("check thread").is_some(),
                    &format!("{case}: the chosen folder to be checked"),
                );
                assert_eq!(
                    *checked.lock().expect("check thread"),
                    Some(false),
                    "{case}: the folder is checked off the UI thread"
                );
                match chosen {
                    ChosenFolder::SlowToAnswer => check.expect_waits_then_lands(started, &release),
                    ChosenFolder::Failing(_, message) => {
                        check.expect_stays_open_saying(started, message)
                    }
                    ChosenFolder::WriteOnly => check.expect_accepts_the_write_only_folder(started),
                }
                check.set_write_only_destination_mode(0o700, "readable destination");
                check.view.browser().clear_observer();
                check.window.destroy();
            }
        },
    );
}
