// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn a_partial_replacing_move_undo_restores_only_completed_originals() {
    crate::test_support::gtk_test(
        "adapters::local_operations::tests::undo::replacements::partial::a_partial_replacing_move_undo_restores_only_completed_originals",
        || {
            let first = ReplacingMove::new(false).expect("first replacing move");
            let second = ReplacingMove::new(false).expect("second replacing move");
            fs::write(second.source(), b"newer").expect("occupy second source");
            let request = undo_request(
                2078,
                vec![
                    fail_if_exists(first.record.clone()),
                    fail_if_exists(second.record.clone()),
                ],
            );

            let events = undo_move(request);

            assert_eq!(
                completed_before_failing(&events),
                std::slice::from_ref(&first.record.current)
            );
            expect_contents(&[
                (first.source().to_path_buf(), b"incoming"),
                (first.target().to_path_buf(), b"original"),
                (second.source().to_path_buf(), b"newer"),
                (second.target().to_path_buf(), b"incoming"),
                (second.staged, b"original"),
            ]);
            assert!(!first.staged.exists());
        },
    );
}

#[test]
fn a_missing_original_does_not_hold_back_the_undo() {
    crate::test_support::gtk_test(
        "adapters::local_operations::tests::undo::replacements::partial::a_missing_original_does_not_hold_back_the_undo",
        || {
            let emptied = ReplacingMove::new(false).expect("first replacing move");
            let kept = ReplacingMove::new(false).expect("second replacing move");
            fs::remove_file(&emptied.staged).expect("remove staged original");
            let request = undo_request(
                2078,
                vec![
                    fail_if_exists(emptied.record.clone()),
                    fail_if_exists(kept.record.clone()),
                ],
            );

            let events = undo_move(request);

            assert_eq!(
                completed_before_failing(&events),
                [emptied.record.current.clone(), kept.record.current.clone()]
            );
            expect_contents(&[
                (emptied.source().to_path_buf(), b"incoming"),
                (kept.source().to_path_buf(), b"incoming"),
                (kept.target().to_path_buf(), b"original"),
            ]);
            assert!(!emptied.target().exists());
            assert!(!kept.staged.exists());
            assert!(matches!(
                events.last(),
                Some(OperationEvent::TransferFailed { message, .. })
                    if message.contains("no longer available")
            ));
        },
    );
}

#[test]
fn cancelling_a_replacing_move_undo_restores_committed_items_only() {
    crate::test_support::gtk_test(
        "adapters::local_operations::tests::undo::replacements::partial::cancelling_a_replacing_move_undo_restores_committed_items_only",
        || {
            let first = ReplacingMove::new(false).expect("first replacing move");
            let second = ReplacingMove::new(false).expect("second replacing move");
            let operation = Rc::new(RefCell::new(None));
            let cancel = operation.clone();
            let request = undo_request(
                2078,
                vec![
                    fail_if_exists(first.record.clone()),
                    fail_if_exists(second.record.clone()),
                ],
            );
            let (load, events) = start_undo_move_observing(request, move |event| {
                if matches!(event, OperationEvent::ItemMoved { .. }) {
                    drop(cancel.take());
                }
            });
            operation.replace(Some(load));

            drive_until_transfer_settles(&events);

            let events = events.take();
            let result = cancelled_result(&events, "cancel after the first move committed");
            assert_eq!(
                result.completed,
                std::slice::from_ref(&first.record.current)
            );
            assert_eq!(
                result.not_attempted,
                std::slice::from_ref(&second.record.current)
            );
            assert!(result.failed.is_empty());
            assert!(!second.source().exists());
            expect_contents(&[
                (first.source().to_path_buf(), b"incoming"),
                (first.target().to_path_buf(), b"original"),
                (second.target().to_path_buf(), b"incoming"),
                (second.staged, b"original"),
            ]);
            assert!(!first.staged.exists());
        },
    );
}

#[test]
fn a_replacing_move_restores_its_original_only_after_the_move_back_is_durable() {
    crate::test_support::gtk_test(
        "adapters::local_operations::tests::undo::replacements::partial::a_replacing_move_restores_its_original_only_after_the_move_back_is_durable",
        || {
            for flushed in [true, false] {
                let fixture = ReplacingMove::new(false).expect("replacing move");
                let (target, staged) = (fixture.target().to_path_buf(), fixture.staged.clone());
                let source = FsQuirks::install(
                    fixture.source().parent().expect("source parent"),
                    Quirks {
                        before: vec![(
                            Op::SyncFs,
                            "*",
                            0,
                            Box::new(move || {
                                expect_contents(&[
                                    (target.clone(), b"incoming"),
                                    (staged.clone(), b"original"),
                                ]);
                            }),
                        )],
                        fail: if flushed {
                            Vec::new()
                        } else {
                            vec![(Op::SyncFs, "*", 0, Errno::IO)]
                        },
                        ..Quirks::ext4()
                    },
                );

                let events = fixture.undo();

                assert!(
                    source.calls().iter().any(|call| call.op == Op::SyncFs),
                    "the move copied across volumes"
                );
                expect_contents(&[(fixture.source().to_path_buf(), b"incoming")]);
                if flushed {
                    assert_pasted(&events, "durable move-back");
                    expect_contents(&[(fixture.target().to_path_buf(), b"original")]);
                    assert!(!fixture.staged.exists());
                } else {
                    assert!(completed_before_failing(&events).is_empty());
                    expect_contents(&[
                        (fixture.target().to_path_buf(), b"incoming"),
                        (fixture.staged, b"original"),
                    ]);
                }
            }
        },
    );
}
