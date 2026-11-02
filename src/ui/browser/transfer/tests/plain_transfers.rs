// SPDX-License-Identifier: MIT

use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

fn selected_entry(path: &str) -> FileEntry {
    FileEntry {
        location: Location::local(path),
        native_name: Path::new(path).file_name().unwrap_or_default().to_owned(),
        thumbnail_path: None,
        display_name: path.to_owned(),
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        mode: crate::model::MetadataValue::Unknown,
        recent_unix_seconds: crate::model::MetadataValue::Unknown,
        is_hidden: false,
        image_dimensions: crate::model::MetadataValue::Unknown,
        child_count: crate::model::MetadataValue::Unknown,
        duration_seconds: crate::model::MetadataValue::Unknown,
        listable: crate::model::MetadataValue::Unknown,
        recent_uri: None,
    }
}

#[test]
fn duplicate_transfer_uses_the_selected_entries_parent() {
    let first = selected_entry("/fixture/selected/first.txt");
    let second = selected_entry("/fixture/selected/second.txt");

    assert_eq!(
        duplicate_transfer(&[first.clone(), second.clone()]),
        Some((
            Location::local("/fixture/selected"),
            vec![first.location, second.location]
        ))
    );
    assert_eq!(
        duplicate_transfer(&[
            selected_entry("/fixture/one.txt"),
            selected_entry("/other/two.txt")
        ]),
        None
    );
    assert_eq!(duplicate_transfer(&[]), None);
    for uri in ["trash:///file.txt", "trash:///folder/file.txt"] {
        let trashed = FileEntry {
            location: Location::uri(uri),
            image_dimensions: crate::model::MetadataValue::Unknown,
            child_count: crate::model::MetadataValue::Unknown,
            duration_seconds: crate::model::MetadataValue::Unknown,
            listable: crate::model::MetadataValue::Unknown,
            ..selected_entry("file.txt")
        };
        assert_eq!(duplicate_transfer(&[trashed]), None);
    }
}

struct NoopTree {
    root: Location,
    source: Location,
    nested: Location,
    sibling: Location,
    file: Location,
}

fn noop_tree(root: Location) -> NoopTree {
    let root_file = gio_file_for_location(&root);
    NoopTree {
        source: Location::uri(root_file.child("source").uri()),
        nested: Location::uri(root_file.child("source/nested").uri()),
        sibling: Location::uri(root_file.child("source-other").uri()),
        file: Location::uri(root_file.child("photo.jpg").uri()),
        root,
    }
}

fn assert_same_folder_noops(tree: &NoopTree, moving: bool) {
    assert_eq!(
        (
            transfer_is_noop(&tree.source, &tree.source, moving),
            transfer_is_noop(&tree.source, &tree.nested, moving),
            transfer_is_noop(&tree.source, &tree.sibling, moving),
            transfer_is_noop(&tree.source, &tree.root, moving),
            transfer_is_noop(&tree.file, &tree.root, moving),
        ),
        (true, true, false, moving, moving),
        "{:?} moving {moving}: (itself, inside itself, a sibling, its parent, a file's parent)",
        tree.root
    );
}

#[test]
fn transfer_noops_preserve_same_folder_copies() {
    for root in [
        Location::local("/fixture"),
        Location::uri("file:///fixture"),
        Location::uri("sftp://example.test/fixture"),
    ] {
        let tree = noop_tree(root);
        for moving in [false, true] {
            assert_same_folder_noops(&tree, moving);
        }
    }
}

struct NoopSources {
    root: tempfile::TempDir,
    source_path: PathBuf,
    nested_path: PathBuf,
    other_path: PathBuf,
}

fn noop_sources() -> NoopSources {
    let root = tempfile::tempdir().expect("transfer fixture");
    let source_path = root.path().join("source");
    let nested_path = source_path.join("nested");
    let other_path = root.path().join("notes.txt");
    std::fs::create_dir_all(&nested_path).expect("nested source folder");
    std::fs::write(&other_path, "notes").expect("source file");
    NoopSources {
        root,
        source_path,
        nested_path,
        other_path,
    }
}

type StartedTransfers = Rc<RefCell<Vec<(usize, bool)>>>;

fn observe_transfer_lifecycle(browser: &Browser) -> (StartedTransfers, Rc<Cell<bool>>) {
    let started = StartedTransfers::default();
    let finished = Rc::new(Cell::new(false));
    let observed_started = started.clone();
    let observed_finished = finished.clone();
    browser.observe(move |event| match operation_event(event) {
        crate::app::BrowserEvent::TransferStarted { total, moving } => {
            observed_started.borrow_mut().push((*total, *moving));
        }
        crate::app::BrowserEvent::TransferFinished { .. } => {
            observed_finished.set(true);
        }
        crate::app::BrowserEvent::OperationFailed { message, .. } => {
            panic!("transfer failed: {message}");
        }
        _ => {}
    });
    (started, finished)
}

fn request_noop_transfers(
    view: &crate::ui::browser::BrowserView,
    sources: &NoopSources,
    moving: bool,
) {
    let source = Location::local(&sources.source_path);
    for destination in [source.clone(), Location::local(&sources.nested_path)] {
        view.state
            .start_transfer(destination, vec![source.clone()], moving);
    }
    view.state.start_transfer(
        Location::local(sources.root.path()),
        vec![source, Location::local(&sources.other_path)],
        true,
    );
}

fn assert_only_the_real_transfer_landed(sources: &NoopSources, moving: bool) {
    let copied =
        std::fs::read_to_string(sources.source_path.join("notes.txt")).expect("copied notes");
    assert_eq!(
        (
            copied,
            sources.other_path.exists(),
            sources.nested_path.is_dir(),
            sources.source_path.join("source").exists(),
        ),
        ("notes".to_owned(), !moving, true, false),
        "(copied notes, source kept, nested kept, folder copied into itself)"
    );
}

#[test]
fn start_transfer_skips_noops_before_emitting_progress() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::plain_transfers::start_transfer_skips_noops_before_emitting_progress",
        || {
            for moving in [false, true] {
                let sources = noop_sources();
                let source = Location::local(&sources.source_path);
                let view = operating_view();
                let browser = view.browser();
                let (started, finished) = observe_transfer_lifecycle(&browser);

                request_noop_transfers(&view, &sources, moving);
                assert!(started.borrow().is_empty());
                assert!(!finished.get());

                view.state.start_transfer(
                    source.clone(),
                    vec![source, Location::local(&sources.other_path)],
                    moving,
                );
                wait_until(|| !started.borrow().is_empty(), "the transfer to start");
                assert_eq!(*started.borrow(), vec![(1, moving)]);
                wait_until(|| finished.get(), "the transfer to finish");
                assert_only_the_real_transfer_landed(&sources, moving);
                browser.clear_observer();
            }
        },
    );
}

struct NormalTransfer {
    _fixture: tempfile::TempDir,
    source_dir: PathBuf,
    source: PathBuf,
    created_destination: PathBuf,
}

fn normal_transfer_fixture() -> NormalTransfer {
    let fixture = tempfile::tempdir().expect("normal transfer fixture");
    let source_dir = fixture.path().join("source");
    std::fs::create_dir(&source_dir).expect("source directory");
    let source = source_dir.join("source.txt");
    std::fs::write(&source, b"source").expect("source file");
    let created_destination = source_dir.join("new folder");
    NormalTransfer {
        _fixture: fixture,
        source_dir,
        source,
        created_destination,
    }
}

fn choose_a_created_folder(
    window: &gtk::Window,
    home_destination: &Path,
    transfer: &NormalTransfer,
) {
    let (chooser_window, chooser) = destination_chooser(window);
    navigate_destination(&chooser, home_destination);
    navigate_destination(&chooser, &transfer.source_dir);
    chooser.create_new_folder();
    wait_until(
        || transfer.created_destination.is_dir(),
        "chooser creates a destination folder",
    );
    chooser.cancel_rename();
    navigate_destination(&chooser, &transfer.created_destination);
    confirm_destination(&chooser_window);
}

#[test]
fn normal_copy_and_move_to_keep_home_navigation_creation_and_reveal() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::plain_transfers::normal_copy_and_move_to_keep_home_navigation_creation_and_reveal",
        || {
            let home_destination = glib::home_dir().join("normal-home-search-target");
            std::fs::create_dir_all(&home_destination).expect("home search directory");
            for move_sources in [false, true] {
                let transfer = normal_transfer_fixture();
                let (view, _overlay, window, events) = open_transfer_browser(&transfer.source_dir);

                view.state
                    .show_transfer_dialog(vec![transfer_entry(&transfer.source)], move_sources);
                choose_a_created_folder(&window, &home_destination, &transfer);
                let created = &transfer.created_destination;
                wait_until(
                    || created.join("source.txt").exists() && finished(&events),
                    "the normal transfer to finish",
                );
                wait_until(
                    || view.browser().active_location() == Some(Location::local(created)),
                    "normal completion to navigate to the destination",
                );
                assert_eq!(
                    transfer.source.exists(),
                    !move_sources,
                    "Copy leaves its source and Move removes it"
                );
                assert!(revealed(&events));
                view.browser().clear_observer();
                window.destroy();
            }
        },
    );
}

/// A disk whose first look at `notes.txt` waits until `release` lets it answer.
struct SlowDisk {
    _quirks: crate::adapters::FsQuirks,
    release: std::sync::mpsc::Sender<()>,
    answered: std::sync::Arc<AtomicBool>,
    source: PathBuf,
    disk: PathBuf,
    _fixture: tempfile::TempDir,
}

fn slow_disk() -> SlowDisk {
    let fixture = tempfile::tempdir().expect("slow fixture");
    let source = fixture.path().join("source/notes.txt");
    let disk = fixture.path().join("disk");
    std::fs::create_dir_all(&disk).expect("disk dir");
    std::fs::create_dir_all(fixture.path().join("source")).expect("source dir");
    std::fs::write(&source, b"notes").expect("source file");
    let (release, held) = std::sync::mpsc::channel::<()>();
    let answered = std::sync::Arc::new(AtomicBool::new(false));
    let looked_up = answered.clone();
    let quirks = crate::adapters::FsQuirks::install(
        &disk,
        crate::adapters::Quirks {
            before: vec![(
                crate::adapters::FsOp::Stat,
                "notes.txt",
                0,
                Box::new(move || {
                    let _released = held.recv();
                    looked_up.store(true, Ordering::SeqCst);
                }),
            )],
            ..crate::adapters::Quirks::ext4()
        },
    );
    SlowDisk {
        _quirks: quirks,
        release,
        answered,
        source,
        disk,
        _fixture: fixture,
    }
}

impl SlowDisk {
    fn paste(&self, view: &crate::ui::browser::BrowserView, moving: bool) {
        view.state.start_transfer(
            Location::local(&self.disk),
            vec![Location::local(&self.source)],
            moving,
        );
    }

    fn release(&self) {
        self.release.send(()).expect("held listing");
    }

    fn wait_answered(&self, what: &str) {
        wait_until(|| self.answered.load(Ordering::SeqCst), what);
    }

    fn untouched(&self) -> bool {
        !self.disk.join("notes.txt").exists() && self.source.exists()
    }

    fn transferred(&self, moving: bool) -> bool {
        holds(&self.disk.join("notes.txt"), b"notes") && self.source.exists() != moving
    }
}

#[test]
fn closing_the_view_while_a_transfer_prepares_dismisses_its_row_and_copies_nothing() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::plain_transfers::closing_the_view_while_a_transfer_prepares_dismisses_its_row_and_copies_nothing",
        || {
            for moving in [false, true] {
                let disk = slow_disk();
                let (view, overlay, window) = conflict_view();
                disk.paste(&view, moving);
                wait_until(
                    || transfer_row_shown(&overlay, moving),
                    &format!("moving {moving}: the transfer to show it prepares its items"),
                );

                view.dispose_file_progress();

                assert!(
                    !transfer_row_shown(&overlay, moving),
                    "moving {moving}: a closed view keeps showing its preparing transfer"
                );
                disk.release();
                disk.wait_answered(&format!("moving {moving}: the destination to answer"));
                iterate_context(20);
                assert!(
                    disk.untouched(),
                    "moving {moving}: a transfer whose view closed while it prepared went on"
                );
                window.destroy();
            }
        },
    );
}

struct SlowPaste {
    view: crate::ui::browser::BrowserView,
    overlay: gtk::Overlay,
    window: gtk::Window,
    disk: SlowDisk,
    moving: bool,
    row: String,
}

impl SlowPaste {
    fn new(moving: bool, cancelled: bool) -> Self {
        let disk = slow_disk();
        let (view, overlay, window) = conflict_view();
        Self {
            view,
            overlay,
            window,
            disk,
            moving,
            row: format!("moving {moving}, cancelled {cancelled}"),
        }
    }

    /// Records, for each transfer start, whether a preparing row was showing.
    fn observe_row_at_start(&self) -> Rc<RefCell<Vec<bool>>> {
        let started = Rc::new(RefCell::new(Vec::new()));
        let observed = started.clone();
        let shown_on = self.overlay.clone();
        let moving = self.moving;
        self.view.browser().observe(move |event| {
            if matches!(
                operation_event(event),
                crate::app::BrowserEvent::TransferStarted { .. }
            ) {
                observed
                    .borrow_mut()
                    .push(transfer_row_shown(&shown_on, moving));
            }
        });
        started
    }

    fn paste(&self) {
        self.disk.paste(&self.view, self.moving);
    }

    fn row_shown(&self) -> bool {
        transfer_row_shown(&self.overlay, self.moving)
    }

    fn assert_row_prepares_items(&self) {
        let row = &self.row;
        assert!(
            visible_texts(&self.overlay).contains(&"notes.txt".to_owned()) || self.moving,
            "{row}: the docked row names the item"
        );
        let shown = visible_texts(&self.overlay);
        let preparing = shown.iter().any(|text| text == "Preparing items…");
        assert!(
            preparing && !shown.iter().any(|text| text == "0/0"),
            "{row}: the transfer says it prepares its items, with no count: {shown:?}"
        );
    }

    fn cancel_preparing_row(&self) {
        if self.moving {
            click_button(&self.overlay, "Cancel");
            return;
        }
        visible_widget(&self.overlay, |button: &gtk::Button| {
            button.has_css_class("progress-cancel")
        })
        .expect("the docked row's cancel")
        .emit_clicked();
    }

    fn cancel_and_paste_again(&self) {
        self.cancel_preparing_row();
        wait_until(
            || !self.row_shown(),
            &format!("{}: the preparing transfer to go away", self.row),
        );
        // On a loaded machine the new paste's own conflict check can outlast the
        // usual delay and show its own row, which is not the cancelled one.
        crate::ui::browser::progress::set_file_progress_delay_for_test(
            std::time::Duration::from_secs(3600),
        );
        self.paste();
    }
}

#[test]
fn a_destination_slow_to_answer_shows_the_transfer_preparing_where_its_row_will_be() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::plain_transfers::a_destination_slow_to_answer_shows_the_transfer_preparing_where_its_row_will_be",
        || {
            for (moving, cancelled) in [(false, true), (true, true), (false, false), (true, false)]
            {
                let paste = SlowPaste::new(moving, cancelled);
                let row = &paste.row;
                let started = paste.observe_row_at_start();

                paste.paste();
                wait_until(
                    || paste.row_shown(),
                    &format!("{row}: the transfer to show it prepares its items"),
                );
                paste.assert_row_prepares_items();
                paste.paste();
                if cancelled {
                    paste.cancel_and_paste_again();
                } else {
                    paste.disk.release();
                }
                wait_until(
                    || paste.disk.transferred(moving),
                    &format!("{row}: the transfer"),
                );
                if cancelled {
                    paste.disk.release();
                }
                paste
                    .disk
                    .wait_answered(&format!("{row}: the destination to answer"));
                iterate_context(20);
                assert_eq!(
                    *started.borrow(),
                    [!cancelled],
                    "{row}: one transfer started, which took the place of the row that \
                     showed it preparing unless that was cancelled"
                );
                crate::ui::browser::progress::reset_file_progress_delay_for_test();
                paste.view.browser().clear_observer();
                paste.window.destroy();
            }
        },
    );
}
