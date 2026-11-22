// SPDX-License-Identifier: MIT

//! A preview provider that keeps every request for a test to answer, and how a test sets it
//! up in place of the sandboxed one.

use std::{
    cell::{Cell, RefCell},
    io::Cursor,
    rc::Rc,
};

use rune::Tx;
use strata_core::model::{FileEntry, Location};
use strata_fs::services::{
    LoadHandle, ModelPreviewStage, PdfTextIndex, Preview, PreviewContent, PreviewEvent,
    PreviewProvider, PreviewRequest,
};

use crate::{App, host::offscreen::Offscreen, test_support::PATIENCE};

/// A request the provider was handed: what it asked, how to answer, and whether its load was
/// dropped before an answer.
struct Asked {
    request: PreviewRequest,
    emit: Rc<dyn Fn(PreviewEvent)>,
    answered: Rc<Cell<bool>>,
    dropped_unanswered: Rc<Cell<bool>>,
}

struct AskedPdfText {
    emit: Rc<dyn Fn(Result<PdfTextIndex, String>)>,
    answered: Rc<Cell<bool>>,
    dropped_unanswered: Rc<Cell<bool>>,
}

#[derive(Default)]
struct RecordedInner {
    requests: Vec<Asked>,
    supports_pdf_text: bool,
    pdf_texts: Vec<AskedPdfText>,
}

#[derive(Clone, Default)]
pub(super) struct Recorded(Rc<RefCell<RecordedInner>>);

impl PreviewProvider for Recorded {
    fn load(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        let answered = Rc::new(Cell::new(false));
        let dropped_unanswered = Rc::new(Cell::new(false));
        let (seen, dropped) = (answered.clone(), dropped_unanswered.clone());
        self.0.borrow_mut().requests.push(Asked {
            request,
            emit,
            answered,
            dropped_unanswered,
        });
        LoadHandle::new(move || dropped.set(!seen.get()))
    }

    fn extract_pdf_text(
        &self,
        _entry: &FileEntry,
        emit: Rc<dyn Fn(Result<PdfTextIndex, String>)>,
    ) -> Option<LoadHandle> {
        let mut inner = self.0.borrow_mut();
        if !inner.supports_pdf_text {
            return None;
        }
        let answered = Rc::new(Cell::new(false));
        let dropped_unanswered = Rc::new(Cell::new(false));
        let (seen, dropped) = (answered.clone(), dropped_unanswered.clone());
        inner.pdf_texts.push(AskedPdfText {
            emit,
            answered,
            dropped_unanswered,
        });
        Some(LoadHandle::new(move || dropped.set(!seen.get())))
    }
}

impl Recorded {
    pub(super) fn enable_pdf_text(&self) {
        self.0.borrow_mut().supports_pdf_text = true;
    }

    pub(super) fn pdf_text_count(&self) -> usize {
        self.0.borrow().pdf_texts.len()
    }

    pub(super) fn wait_for_pdf_text(&self, host: &mut Offscreen, index: usize) {
        let asked = host.run_until(PATIENCE, |_| self.pdf_text_count() > index);
        assert!(asked, "pdf text request {index} asked of the provider");
    }

    pub(super) fn answer_pdf_text(&self, index: usize, result: Result<PdfTextIndex, String>) {
        let emit = {
            let inner = self.0.borrow();
            let asked = &inner.pdf_texts[index];
            asked.answered.set(true);
            asked.emit.clone()
        };
        emit(result);
    }

    pub(super) fn pdf_text_dropped_unanswered(&self, index: usize) -> bool {
        self.0.borrow().pdf_texts[index].dropped_unanswered.get()
    }

    pub(super) fn count(&self) -> usize {
        self.0.borrow().requests.len()
    }

    pub(super) fn request(&self, index: usize) -> PreviewRequest {
        self.0.borrow().requests[index].request.clone()
    }

    pub(super) fn dropped_unanswered(&self, index: usize) -> bool {
        self.0.borrow().requests[index].dropped_unanswered.get()
    }

    /// Answers request `index` with `content`.
    pub(super) fn answer(&self, index: usize, content: PreviewContent) {
        self.answer_as(index, "application/octet-stream", content);
    }

    /// Answers request `index` with `content` of the type `content_type`.
    pub(super) fn answer_as(&self, index: usize, content_type: &str, content: PreviewContent) {
        self.finish(index, |request| {
            PreviewEvent::Ready(Preview {
                request_id: request.id,
                entry: request.entry.clone(),
                content_type: content_type.to_owned(),
                content,
            })
        });
    }

    /// Fails request `index` with `message`.
    pub(super) fn fail(&self, index: usize, message: &str) {
        self.finish(index, |request| PreviewEvent::Failed {
            request_id: request.id,
            entry: request.entry.clone(),
            message: message.to_owned(),
        });
    }

    /// Tells request `index` its load reached `stage`.
    pub(super) fn progress(&self, index: usize, stage: ModelPreviewStage) {
        let inner = self.0.borrow();
        let request_id = inner.requests[index].request.id;
        (inner.requests[index].emit)(PreviewEvent::Progress { request_id, stage });
    }

    /// Ends request `index` with the event `event` makes of it.
    fn finish(&self, index: usize, event: impl FnOnce(&PreviewRequest) -> PreviewEvent) {
        let inner = self.0.borrow();
        let asked = &inner.requests[index];
        asked.answered.set(true);
        (asked.emit)(event(&asked.request));
    }

    /// Waits for request `index`.
    pub(super) fn wait_for(&self, host: &mut Offscreen, index: usize) {
        let asked = host.run_until(PATIENCE, |_| self.count() > index);
        assert!(asked, "request {index} asked of the provider");
    }
}

/// Puts `provider` in the preview's place of the sandboxed one.
pub(super) fn install(host: &mut Offscreen, provider: &Recorded) {
    let provider = provider.clone();
    host.runtime().update(move |app: &mut App, _: &mut Tx| {
        app.preview.loader.set_provider(Rc::new(provider));
    });
    while host.turn() {}
}

/// A blank PNG `width` × `height`.
pub(super) fn png(width: u32, height: u32) -> Vec<u8> {
    let page = image::RgbaImage::from_pixel(width, height, image::Rgba([255, 255, 255, 255]));
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(page)
        .write_to(&mut png, image::ImageFormat::Png)
        .expect("a PNG");
    png.into_inner()
}

/// The canonical fixture with `files` added, listed.
pub(super) fn listed_with(host: &mut Offscreen, files: &[(&str, &str)]) -> tempfile::TempDir {
    let folder = crate::test_support::fixture();
    for (name, text) in files {
        std::fs::write(folder.path().join(name), text).expect("a fixture file");
    }
    host.app().open(Location::local(folder.path()));
    let counted = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
    assert!(host.run_until(PATIENCE, counted), "the folder's listing");
    while host.turn() {}
    folder
}
