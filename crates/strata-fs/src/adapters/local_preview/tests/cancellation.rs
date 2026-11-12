// SPDX-License-Identifier: MIT

use super::*;

/// Signals `started`, then blocks the worker until `finish` releases it, so
/// the test can cancel the load while the helper is still running.
fn gated_renderer(
    started: oneshot::Sender<()>,
    finish: std::sync::mpsc::Receiver<()>,
    output: Result<crate::sandbox::ParseOutput, String>,
) -> impl FnOnce(
    &Path,
    ParseOperation,
    i32,
    MediaPreviewBackend,
    &Cancellation,
) -> Result<crate::sandbox::ParseOutput, String>
+ Send
+ 'static {
    move |_, _, _, _, cancellation| {
        started.send(()).expect("notify renderer started");
        finish
            .recv_timeout(Duration::from_secs(10))
            .expect("release renderer");
        assert!(cancellation.is_cancelled());
        output
    }
}

fn late_document_output(succeeds: bool) -> Result<crate::sandbox::ParseOutput, String> {
    if succeeds {
        Ok(parse_output(
            br#"{"rows":[["a"],["1"]],"truncated":false}"#.to_vec(),
            1,
            2,
        ))
    } else {
        Err("late renderer error".into())
    }
}

fn assert_no_events(events: &RecordedEvents) {
    assert!(
        events.borrow().is_empty(),
        "cancelled load emitted an event"
    );
}

#[test]
fn cancelled_in_flight_document_renders_keep_the_permit_and_emit_no_stale_events() {
    with_default_main_context(|provider, context| {
        for (filename, succeeds) in [
            ("cancelled.pdf", false),
            ("cancelled.pdf", true),
            ("cancelled.xlsx", false),
            ("cancelled.xlsx", true),
            ("cancelled.stl", false),
            ("cancelled.stl", true),
        ] {
            let (events, emit) = event_recorder();
            let (started, receive_started) = oneshot::channel();
            let (finish, receive_finish) = std::sync::mpsc::channel();
            let request = PreviewRequest {
                pdf_page: 1,
                ..preview_request(crate::model::Location::local(filename), filename)
            };
            let handle = provider.load_with_renderer(
                request,
                emit,
                gated_renderer(started, receive_finish, late_document_output(succeeds)),
            );
            context.block_on(async {
                receive_started.await.expect("renderer started");
                drop(handle);
                let mut next = request_heavy_preview_permit();
                assert!(
                    heavy_preview_is_waiting(&mut next),
                    "cancellation must not release the permit before the helper exits"
                );
                finish.send(()).expect("finish cancelled renderer");
                drop(next.acquire().await.expect("next renderer can start"));
            });
            assert_no_events(&events);
        }
    });
}

#[test]
fn cancelled_archive_listings_emit_no_stale_events() {
    with_default_main_context(|provider, context| {
        let (events, emit) = event_recorder();
        let (started, receive_started) = oneshot::channel();
        let (finish, receive_finish) = std::sync::mpsc::channel();
        let empty_listing = parse_output(
            b"{\"status\":\"open\",\"entries\":[],\"message\":null}".to_vec(),
            0,
            0,
        );
        let handle = provider.load_with_renderer(
            preview_request(
                crate::model::Location::local("cancelled.zip"),
                "cancelled.zip",
            ),
            emit,
            gated_renderer(started, receive_finish, Ok(empty_listing)),
        );
        context.block_on(async {
            receive_started.await.expect("renderer started");
            drop(handle);
            finish.send(()).expect("finish cancelled renderer");
            glib::timeout_future(Duration::from_millis(50)).await;
        });
        assert_no_events(&events);
    });
}
