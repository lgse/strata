// SPDX-License-Identifier: MIT

use std::{fs, time::Duration};

mod archive;
mod cache;
mod cancellation;
mod cover_preview;
mod heavy_queue;
mod model_preview;
mod pdf;
mod remote_preview;
mod resolution;
mod route_preview;

use super::*;
use crate::services::{MediaPreviewSize, PreviewContent};

type RecordedEvents = Rc<RefCell<Vec<PreviewEvent>>>;

fn preview_entry(location: crate::model::Location, name: &str) -> crate::model::FileEntry {
    use crate::model::{EntryKind, FileEntry, MetadataValue};

    FileEntry {
        location,
        thumbnail_path: None,
        native_name: name.into(),
        display_name: name.into(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        mode: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        recent_uri: None,
    }
}

fn preview_request(location: crate::model::Location, name: &str) -> PreviewRequest {
    PreviewRequest {
        id: crate::services::PreviewRequestId(1),
        entry: preview_entry(location, name),
        text_byte_limit: 1024,
        render_document: false,
        pdf_page: 0,
        media_size: MediaPreviewSize::new(640, 800),
        model_palette: crate::services::ModelPalette::default(),
        model_view: Default::default(),
        archive_password: None,
    }
}

fn software_provider() -> LocalPreviewProvider {
    LocalPreviewProvider::new(Rc::new(|| MediaPreviewBackend::Software))
}

fn parse_output(data: Vec<u8>, page: i32, pages: i32) -> crate::sandbox::ParseOutput {
    crate::sandbox::ParseOutput {
        data,
        page,
        pages,
        text_layer: None,
        model_mesh: None,
    }
}

fn event_recorder() -> (RecordedEvents, Rc<dyn Fn(PreviewEvent)>) {
    let events = RecordedEvents::default();
    let events_for_emit = events.clone();
    (
        events,
        Rc::new(move |event| events_for_emit.borrow_mut().push(event)),
    )
}

fn wait_for_first_event(context: &glib::MainContext, events: &RecordedEvents) {
    context.block_on(async {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while events.borrow().is_empty() && std::time::Instant::now() < deadline {
            glib::timeout_future(Duration::from_millis(1)).await;
        }
    });
}

/// Loads `request` with an injected renderer and returns the events seen
/// until the first one arrives or five seconds pass.
fn load_until_first_event(
    provider: &LocalPreviewProvider,
    context: &glib::MainContext,
    request: PreviewRequest,
    render: impl FnOnce(
        &Path,
        ParseOperation,
        i32,
        MediaPreviewBackend,
        &Cancellation,
    ) -> Result<crate::sandbox::ParseOutput, String>
    + Send
    + 'static,
) -> Vec<PreviewEvent> {
    let (events, emit) = event_recorder();
    let _handle = provider.load_with_renderer(request, emit, render);
    wait_for_first_event(context, &events);
    events.borrow().clone()
}

/// Runs `test` while owning the shared default main context.
fn with_default_main_context(test: impl FnOnce(&LocalPreviewProvider, &glib::MainContext)) {
    let _main_context = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("main context lock");
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("main context owner");
    test(&software_provider(), &context);
}

fn heavy_preview_is_waiting(waiter: &mut HeavyPreviewWaiter) -> bool {
    waiter
        .receive
        .as_mut()
        .expect("heavy preview receiver")
        .try_recv()
        .expect("heavy preview receiver open")
        .is_none()
}
