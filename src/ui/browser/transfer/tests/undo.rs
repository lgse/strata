// SPDX-License-Identifier: MIT

use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc::Sender};

struct MovedPair {
    _fixture: tempfile::TempDir,
    original: PathBuf,
    current: PathBuf,
}

fn moved_pair(files: &[(&str, &[u8])]) -> MovedPair {
    let fixture = tempfile::tempdir().expect("undo fixture");
    let original = fixture.path().join("original");
    let current = fixture.path().join("current");
    std::fs::create_dir_all(&original).expect("original dir");
    std::fs::create_dir_all(&current).expect("current dir");
    write_files(&original, files);
    MovedPair {
        _fixture: fixture,
        original,
        current,
    }
}

impl MovedPair {
    fn move_to_current(&self, view: &crate::ui::browser::BrowserView, names: &[&str]) {
        view.state.start_transfer(
            Location::local(&self.current),
            names
                .iter()
                .map(|name| Location::local(self.original.join(name)))
                .collect(),
            true,
        );
    }

    fn moved(&self, name: &str) -> bool {
        self.current.join(name).exists() && !self.original.join(name).exists()
    }

    fn moved_back(&self, name: &str) -> bool {
        self.original.join(name).exists() && !self.current.join(name).exists()
    }
}

fn pending_undo(browser: &Browser) -> (u64, Vec<MoveRecord>) {
    wait_until(
        || browser.pending_undo_move().is_some(),
        "the move undo to become pending",
    );
    browser.pending_undo_move().expect("pending move undo")
}

#[test]
fn undo_move_keeps_skip_visible_for_a_partial_restore() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::undo::undo_move_keeps_skip_visible_for_a_partial_restore",
        || {
            let pair = moved_pair(&[("a.txt", b"new a"), ("b.txt", b"new b")]);
            let (original, current) = (&pair.original, &pair.current);
            let (view, overlay, window) = conflict_view();

            pair.move_to_current(&view, &["a.txt", "b.txt"]);
            wait_until(|| pair.moved("b.txt"), "the move to complete");
            std::fs::write(original.join("a.txt"), b"blocker").expect("new occupant");

            let browser = view.browser();
            let (generation, records) = pending_undo(&browser);
            assert_eq!(records.len(), 2, "both moved items must be recorded");
            view.state.undo_move(generation, records);

            assert!(
                wait_for_modal_layer(&overlay),
                "undo collision dialog did not appear"
            );
            assert_skip_offered_alone(
                &overlay,
                "skip must stay visible when cancelling would discard the accepted restore",
                "apply to all has no further conflicts left to apply to",
            );
            click_button(&overlay, "Skip");

            wait_until(
                || !current.join("b.txt").exists(),
                "the accepted item to move back",
            );
            assert_eq!(
                std::fs::read(original.join("b.txt")).expect("restored file"),
                b"new b",
                "the accepted portion of the undo must still be restored"
            );
            assert!(
                current.join("a.txt").exists(),
                "skipping must leave the conflicting move in place"
            );
            assert_eq!(
                std::fs::read(original.join("a.txt")).expect("new occupant"),
                b"blocker",
                "a skipped conflict must not be overwritten"
            );
            window.destroy();
        },
    );
}

#[test]
fn redo_move_replays_the_undone_transfer() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::undo::redo_move_replays_the_undone_transfer",
        || {
            let pair = moved_pair(&[("a.txt", b"moved")]);
            let (view, _overlay, window) = conflict_view();

            pair.move_to_current(&view, &["a.txt"]);
            wait_until(|| pair.moved("a.txt"), "the move to complete");

            let browser = view.browser();
            let (generation, records) = pending_undo(&browser);
            assert!(view.state.undo_move(generation, records));
            wait_until(
                || pair.moved_back("a.txt"),
                "the undo to move the item back",
            );

            wait_until(
                || browser.pending_redo_move().is_some(),
                "the redo of the undone move to become pending",
            );
            let (generation, records) = browser.pending_redo_move().expect("pending move redo");
            assert!(view.state.redo_move(generation, records));
            wait_until(
                || pair.moved("a.txt"),
                "the redo to move the item forward again",
            );
            wait_until(
                || browser.pending_undo_move().is_some(),
                "the completed redo to regenerate the move undo",
            );
            window.destroy();
        },
    );
}

#[test]
fn keeping_both_on_a_move_puts_the_item_beside_the_other_and_undo_moves_it_back() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::undo::keeping_both_on_a_move_puts_the_item_beside_the_other_and_undo_moves_it_back",
        || {
            let pair = moved_pair(&[("todo.txt", b"moved")]);
            let (original, current) = (&pair.original, &pair.current);
            std::fs::write(current.join("todo.txt"), b"existing").expect("existing file");
            let (view, overlay, window) = conflict_view();

            pair.move_to_current(&view, &["todo.txt"]);
            assert!(
                wait_for_modal_layer(&overlay),
                "conflict dialog modal did not appear"
            );
            click_button(&overlay, "Keep Both");
            let kept = current.join("todo (1).txt");
            wait_until(
                || holds(&kept, b"moved") && !original.join("todo.txt").exists(),
                "the move beside the existing item",
            );

            let browser = view.browser();
            let (generation, records) = pending_undo(&browser);
            assert!(view.state.undo_move(generation, records));
            wait_until(
                || holds(&original.join("todo.txt"), b"moved") && !kept.exists(),
                "the undo to move the item back",
            );
            assert_eq!(
                std::fs::read(current.join("todo.txt")).expect("existing file"),
                b"existing"
            );
            window.destroy();
        },
    );
}

/// A single `notes.txt` moved from `original` to `current`, with its undo pending.
struct MovedItem {
    view: crate::ui::browser::BrowserView,
    overlay: gtk::Overlay,
    window: gtk::Window,
    original: PathBuf,
    current: PathBuf,
    fixture: tempfile::TempDir,
}

fn moved_item(make: impl FnOnce(&Path)) -> MovedItem {
    let fixture = tempfile::tempdir().expect("undo fixture");
    let (view, overlay, window) = conflict_view();
    let (original, current) = (
        fixture.path().join("original"),
        fixture.path().join("current"),
    );
    std::fs::create_dir_all(&original).expect("original dir");
    std::fs::create_dir_all(&current).expect("current dir");
    let (original, current) = (original.join("notes.txt"), current.join("notes.txt"));
    make(&original);
    view.state.start_transfer(
        Location::local(current.parent().expect("current dir")),
        vec![Location::local(&original)],
        true,
    );
    wait_until(
        || current.symlink_metadata().is_ok() && original.symlink_metadata().is_err(),
        "the move to complete",
    );
    wait_until(
        || view.browser().pending_undo_move().is_some(),
        "the move undo to become pending",
    );
    MovedItem {
        view,
        overlay,
        window,
        original,
        current,
        fixture,
    }
}

impl MovedItem {
    fn browser(&self) -> Rc<Browser> {
        self.view.browser()
    }

    fn undo(&self) -> bool {
        let (generation, records) = self.browser().pending_undo_move().expect("pending undo");
        self.view.state.undo_move(generation, records)
    }

    fn moved_back(&self) -> bool {
        self.original.exists() && !self.current.exists()
    }

    fn preparing_row_shown(&self) -> bool {
        transfer_row_shown(&self.overlay, true)
            && visible_texts(&self.overlay)
                .iter()
                .any(|text| text == "Preparing items…")
    }

    /// Records the thread that checks the item and holds the check until released.
    fn hold_the_item_check(
        &self,
    ) -> (
        Sender<()>,
        Arc<Mutex<Option<bool>>>,
        crate::adapters::FsQuirks,
    ) {
        let (release, held) = std::sync::mpsc::channel::<()>();
        let checked = Arc::new(Mutex::new(None));
        let seen = checked.clone();
        let scope = crate::adapters::FsQuirks::install(
            self.fixture.path(),
            crate::adapters::Quirks {
                before: vec![(
                    crate::adapters::FsOp::Stat,
                    "notes.txt",
                    0,
                    Box::new(move || {
                        *seen.lock().expect("check thread") =
                            Some(crate::util::ui_thread::is_current());
                        let _released = held.recv();
                    }),
                )],
                ..crate::adapters::Quirks::ext4()
            },
        );
        (release, checked, scope)
    }

    fn cancel_the_preparing_undo(&self, row: &str, release: &Sender<()>) {
        click_button(&self.overlay, "Cancel");
        wait_until(
            || !transfer_row_shown(&self.overlay, true),
            &format!("{row}: the preparing undo to go away"),
        );
        release.send(()).expect("held check");
        iterate_context(20);
        assert!(
            self.current.exists() && !self.original.exists(),
            "{row}: a cancelled undo moves nothing"
        );
        let (generation, records) = self
            .browser()
            .pending_undo_move()
            .expect("a cancelled undo stays in the history");
        assert!(self.view.state.undo_move(generation, records));
    }
}

#[test]
fn an_undo_checks_its_items_off_the_ui_thread_and_a_cancelled_check_keeps_it() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::undo::an_undo_checks_its_items_off_the_ui_thread_and_a_cancelled_check_keeps_it",
        || {
            for cancelled in [false, true] {
                let row = format!("cancelled {cancelled}");
                let item = moved_item(|path| {
                    std::fs::write(path, b"notes").expect("moved file");
                });
                let browser = item.browser();
                let (release, checked, _scope) = item.hold_the_item_check();
                let started = browser.last_started_operation();
                let (generation, records) = browser.pending_undo_move().expect("pending undo");

                assert!(item.view.state.undo_move(generation, records.clone()));
                wait_until(
                    || checked.lock().expect("check thread").is_some(),
                    &format!("{row}: the undo to check its items"),
                );
                assert_eq!(
                    *checked.lock().expect("check thread"),
                    Some(false),
                    "{row}: the items are checked off the UI thread"
                );
                assert!(
                    item.view.state.undo_move(generation, records),
                    "{row}: a repeated undo is taken while the first one checks"
                );
                wait_until(
                    || item.preparing_row_shown(),
                    &format!("{row}: the undo to show it prepares its items"),
                );
                assert_eq!(
                    browser.last_started_operation(),
                    started,
                    "{row}: nothing moves while the items are checked"
                );
                if cancelled {
                    item.cancel_the_preparing_undo(&row, &release);
                } else {
                    release.send(()).expect("held check");
                }
                wait_until(
                    || item.moved_back(),
                    &format!("{row}: the undo to move the item back"),
                );
                item.window.destroy();
            }
        },
    );
}

struct UndoCase {
    case: &'static str,
    make: fn(&Path),
    failed_check: Option<usize>,
    undone: bool,
}

fn undo_cases() -> [UndoCase; 4] {
    [
        UndoCase {
            case: "a dangling link",
            make: |path| std::os::unix::fs::symlink("nowhere", path).expect("dangling link"),
            failed_check: None,
            undone: true,
        },
        UndoCase {
            case: "a check of the item that fails",
            make: |path| std::fs::write(path, b"notes").expect("moved file"),
            failed_check: Some(0),
            undone: true,
        },
        UndoCase {
            case: "a check of its destination that fails",
            make: |path| std::fs::write(path, b"notes").expect("moved file"),
            failed_check: Some(1),
            undone: true,
        },
        UndoCase {
            case: "an item deleted since",
            make: |path| std::fs::write(path, b"notes").expect("moved file"),
            failed_check: None,
            undone: false,
        },
    ]
}

/// Notes each check of the item and fails the `failed_check`th one.
fn watch_the_item_check(
    root: &Path,
    failed_check: Option<usize>,
) -> (Arc<AtomicBool>, crate::adapters::FsQuirks) {
    let checked = Arc::new(AtomicBool::new(false));
    let seen = checked.clone();
    let scope = crate::adapters::FsQuirks::install(
        root,
        crate::adapters::Quirks {
            before: vec![(
                crate::adapters::FsOp::Stat,
                "notes.txt",
                0,
                Box::new(move || seen.store(true, Ordering::SeqCst)),
            )],
            fail: failed_check
                .map(|skipped| {
                    (
                        crate::adapters::FsOp::Stat,
                        "notes.txt",
                        skipped,
                        rustix::io::Errno::IO,
                    )
                })
                .into_iter()
                .collect(),
            ..crate::adapters::Quirks::ext4()
        },
    );
    (checked, scope)
}

fn expect_the_item_moved_back(item: &MovedItem, case: &str) {
    wait_until(
        || item.original.symlink_metadata().is_ok() && item.current.symlink_metadata().is_err(),
        &format!("{case}: the undo to move the item back"),
    );
    assert!(
        !visible_texts(&item.overlay)
            .iter()
            .any(|text| text == "File already exists"),
        "{case}: a destination that cannot be ruled out is not replaced"
    );
}

fn expect_the_undo_dropped(
    item: &MovedItem,
    case: &str,
    started: Option<crate::services::OperationRequestId>,
) {
    let browser = item.browser();
    wait_until(
        || browser.pending_undo_move().is_none(),
        &format!("{case}: the undo to be dropped"),
    );
    assert_eq!(
        browser.last_started_operation(),
        started,
        "{case}: nothing to move back"
    );
}

#[test]
fn an_undo_tries_items_it_cannot_rule_out_and_drops_only_items_proven_gone() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::undo::an_undo_tries_items_it_cannot_rule_out_and_drops_only_items_proven_gone",
        || {
            for undo in undo_cases() {
                let case = undo.case;
                let item = moved_item(undo.make);
                if !undo.undone {
                    std::fs::remove_file(&item.current).expect("item deleted since the move");
                }
                let (checked, _scope) =
                    watch_the_item_check(item.fixture.path(), undo.failed_check);
                let started = item.browser().last_started_operation();

                assert!(item.undo());

                wait_until(
                    || checked.load(Ordering::SeqCst),
                    &format!("{case}: the undo to check the item"),
                );
                if undo.undone {
                    expect_the_item_moved_back(&item, case);
                } else {
                    expect_the_undo_dropped(&item, case, started);
                }
                item.window.destroy();
            }
        },
    );
}
