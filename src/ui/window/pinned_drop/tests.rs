// SPDX-License-Identifier: MIT

use std::cell::Cell;

use super::*;

#[test]
fn pending_folder_validation_is_shared_and_delivers_the_whole_selection_once() {
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            context.block_on(async {
                let (send, receive) = futures_channel::oneshot::channel();
                let queries = Rc::new(Cell::new(0));
                let counted = queries.clone();
                let validation = FolderValidation::start(
                    async move {
                        counted.set(counted.get() + 1);
                        receive.await.expect("query response")
                    },
                    Duration::from_secs(1),
                );
                let results = Rc::new(RefCell::new(Vec::new()));
                for _ in 0..3 {
                    let results = results.clone();
                    validation.on_ready(move |result| results.borrow_mut().push(result));
                }
                assert_eq!(
                    validation.action(gdk::DragAction::COPY),
                    gdk::DragAction::COPY
                );
                let sources: Rc<[Folder]> = vec![
                    (Location::uri("sftp://host/a"), "A".to_owned()),
                    (Location::local("/mount/b"), "B".to_owned()),
                ]
                .into();
                send.send(Some(sources.clone())).expect("complete query");
                let (done, received) = futures_channel::oneshot::channel();
                validation.on_ready(move |result| {
                    let _ = done.send(result);
                });
                assert_eq!(received.await.expect("completion"), Some(sources.clone()));
                assert_eq!(queries.get(), 1);
                assert_eq!(*results.borrow(), vec![Some(sources.clone()); 3]);
                validation.finish(None);
                validation.on_ready(move |result| assert_eq!(result, Some(sources)));
            })
        })
        .expect("private main context");
}

#[test]
fn a_stalled_query_times_out_and_drops_its_pending_io() {
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            context.block_on(async {
                let (query_send, query_receive) =
                    futures_channel::oneshot::channel::<FolderResult>();
                let validation = FolderValidation::start(
                    async move { query_receive.await.expect("query response") },
                    Duration::from_millis(1),
                );
                let (done, received) = futures_channel::oneshot::channel();
                validation.on_ready(move |result| {
                    let _ = done.send(result);
                });
                assert!(received.await.expect("timeout completion").is_none());
                assert_eq!(
                    validation.action(gdk::DragAction::COPY),
                    gdk::DragAction::empty()
                );
                assert!(
                    query_send
                        .send(Some(
                            vec![(Location::local("/late"), "Late".to_owned())].into()
                        ))
                        .is_err()
                );
            })
        })
        .expect("private main context");
}

#[test]
fn abandoning_a_request_rejects_its_pending_drop_and_cancels_io() {
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let (send, receive) = futures_channel::oneshot::channel::<FolderResult>();
            let validation = FolderValidation::start(
                async move { receive.await.expect("query response") },
                Duration::from_secs(1),
            );
            let completed = Rc::new(Cell::new(false));
            let observed = completed.clone();
            validation.on_ready(move |result| {
                assert!(result.is_none());
                observed.set(true);
            });
            drop(validation);
            assert!(completed.get());
            assert!(send.send(None).is_err());
        })
        .expect("private main context");
}

#[test]
fn a_rejected_selection_cannot_become_a_copy_or_partial_pin() {
    let validation = FolderValidation::ready(None);
    validation.on_ready(|result| assert!(result.is_none()));
    for offered in [gdk::DragAction::COPY, gdk::DragAction::MOVE] {
        assert_eq!(validation.action(offered), gdk::DragAction::empty());
    }
}

#[test]
fn pin_validation_preserves_the_choosers_local_only_boundary_before_any_probe() {
    for (location, browser, chooser) in [
        (Location::local("/home/user/folder"), true, true),
        (Location::uri("sftp://host/folder"), true, false),
        (Location::uri("trash:///folder"), false, false),
        (Location::uri("http://example.com/"), false, false),
        (Location::uri("HTTPS://example.com/folder"), false, false),
    ] {
        assert_eq!(can_pin(&location, false), browser, "browser: {location:?}");
        assert_eq!(can_pin(&location, true), chooser, "chooser: {location:?}");
    }
}
