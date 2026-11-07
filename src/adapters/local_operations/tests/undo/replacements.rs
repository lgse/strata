// SPDX-License-Identifier: MIT

use super::*;
use crate::adapters::local_operations::trashed_merge_original;

mod partial;

struct ReplacingMove {
    _root: tempfile::TempDir,
    record: MoveRecord,
    staged: PathBuf,
}

impl ReplacingMove {
    fn new(directory: bool) -> Result<Self, Box<dyn Error>> {
        let root = tempfile::tempdir()?;
        let source = root.path().join("b/note");
        let target = root.path().join("a/note");
        for (path, contents) in [(&source, b"incoming"), (&target, b"original")] {
            fs::create_dir_all(path.parent().ok_or("parent")?)?;
            if directory {
                fs::create_dir(path)?;
            }
            fs::write(contents_path(path, directory), contents)?;
        }
        let events = run_paste(PasteRequest {
            id: OperationRequestId(2077),
            destination: Location::local(target.parent().ok_or("parent")?),
            items: vec![PasteItem {
                source: Location::local(&source),
                conflict: TransferConflict::ReplaceExisting,
                target_name: None,
            }],
            move_sources: true,
        });
        assert_pasted(&events, "replacing move");
        let mut record = record(&source, &target);
        record.replaced = events.iter().find_map(|event| match event {
            OperationEvent::ItemTrashed {
                location, identity, ..
            } if *location == record.current => Some(*identity),
            _ => None,
        });
        let identity = record.replaced.ok_or("original was not staged in Trash")?;
        let entry = glib::MainContext::default().block_on(trashed_merge_original(
            record.current.clone(),
            gio::Cancellable::new(),
            HashMap::from([(record.current.clone(), identity)]),
        ))?;
        Ok(Self {
            _root: root,
            record,
            staged: entry.physical_path.ok_or("staged path")?,
        })
    }

    fn undo(&self) -> Vec<OperationEvent> {
        undo_move(undo_request(
            2078,
            vec![fail_if_exists(self.record.clone())],
        ))
    }

    fn source(&self) -> &Path {
        self.record.original.native_path().expect("local source")
    }

    fn target(&self) -> &Path {
        self.record.current.native_path().expect("local target")
    }
}

fn contents_path(path: &Path, directory: bool) -> PathBuf {
    if directory {
        path.join("contents.txt")
    } else {
        path.to_path_buf()
    }
}

#[test]
fn undoing_a_replacing_move_restores_both_items() {
    crate::test_support::gtk_test(
        "adapters::local_operations::tests::undo::replacements::undoing_a_replacing_move_restores_both_items",
        || {
            for directory in [false, true] {
                let fixture = ReplacingMove::new(directory).expect("replacing move");

                let events = fixture.undo();

                assert_pasted(&events, "replacing move undo");
                expect_contents(&[
                    (contents_path(fixture.source(), directory), b"incoming"),
                    (contents_path(fixture.target(), directory), b"original"),
                ]);
                assert!(!fixture.staged.exists(), "the original left Trash");
                assert!(events.iter().any(|event| matches!(event,
                    OperationEvent::ItemRestored { location, identity, .. }
                    if *location == fixture.record.current
                        && Some(*identity) == fixture.record.replaced
                )));
            }
        },
    );
}

#[test]
fn restoring_a_replaced_move_never_overwrites_a_new_arrival() {
    crate::test_support::gtk_test(
        "adapters::local_operations::tests::undo::replacements::restoring_a_replaced_move_never_overwrites_a_new_arrival",
        || {
            let fixture = ReplacingMove::new(false).expect("replacing move");
            let target = fixture.target().to_path_buf();
            let (_operation, events) = start_undo_move_observing(
                undo_request(2078, vec![fail_if_exists(fixture.record.clone())]),
                move |event| {
                    if matches!(event, OperationEvent::ItemMoved { .. }) {
                        fs::write(&target, b"new arrival").expect("occupy vacated path");
                    }
                },
            );
            drive_until_transfer_settles(&events);
            let events = events.take();

            assert_eq!(
                completed_before_failing(&events),
                std::slice::from_ref(&fixture.record.current)
            );
            expect_contents(&[
                (fixture.source().to_path_buf(), b"incoming"),
                (fixture.target().to_path_buf(), b"new arrival"),
                (fixture.staged, b"original"),
            ]);
        },
    );
}
