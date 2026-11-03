// SPDX-License-Identifier: MIT

use super::*;

impl PreviewState {
    pub(super) fn render_document_preview(
        self: &Rc<Self>,
        source: PendingSourcePreview,
        document: Option<DocumentLayout>,
        fallback_reason: Option<String>,
        warnings: Vec<String>,
    ) {
        let render_pending = document.is_none() && fallback_reason.is_none();
        let rendered_available = document.is_some() || render_pending;
        let initial = initial_document_view(
            self.document_view.get() == DocumentView::Rendered,
            document.is_some(),
        );

        if let Some(reason) = fallback_reason.as_deref() {
            self.content
                .append(&document_notice(&crate::i18n::tr(reason)));
        }

        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(100)
            .hexpand(true)
            .vexpand(true)
            .build();
        self.document_preview.replace(Some(DocumentPreview {
            stack: stack.clone(),
            source: Some(source),
            rendered: document.map(|document| (document, warnings)),
            rendered_state: RefCell::new(None),
            render_pending,
        }));
        self.content.append(&stack);
        self.show_document_view(initial);
        self.document_view_button.set_visible(rendered_available);
    }

    pub(super) fn show_document_view(self: &Rc<Self>, view: DocumentView) {
        let mut preview_guard = self.document_preview.borrow_mut();
        let Some(preview) = preview_guard.as_mut() else {
            return;
        };
        if view == DocumentView::Rendered && preview.render_pending {
            preview.render_pending = false;
            drop(preview_guard);
            self.document_view.set(DocumentView::Rendered);
            self.update_document_view_action();
            let entry = self.current.borrow().clone();
            if let Some(entry) = entry {
                self.load_request(entry, 0, true, None);
            }
            return;
        }
        let name = document_view_name(view);
        if preview.stack.child_by_name(name).is_none() {
            let child = match view {
                DocumentView::Source => preview.source.take().map(|source| {
                    let (view, _) = self.source_preview.show(
                        &source.entry,
                        &source.content_type,
                        &source.content,
                        source.truncated,
                    );
                    view
                }),
                DocumentView::Rendered => preview.rendered.take().map(|(document, warnings)| {
                    let wrapped =
                        super::super::preferences::PreferenceManager::shared().preview_text_wrap();
                    let (view, state) = super::super::virtual_preview::rendered_document(
                        document,
                        warnings,
                        wrapped,
                        self.current
                            .borrow()
                            .as_ref()
                            .and_then(|entry| entry.location.native_path().map(ToOwned::to_owned)),
                    );
                    preview.rendered_state.replace(Some(state));
                    view.upcast()
                }),
            };
            let Some(child) = child else {
                return;
            };
            preview.stack.add_named(&child, Some(name));
        }
        preview.stack.set_visible_child_name(name);
        self.document_view.set(view);
        self.update_document_view_action();
        drop(preview_guard);
        self.refresh_find();
    }

    pub(super) fn update_document_view_action(&self) {
        let (label, icon) = document_view_action(self.document_view.get());
        let label = crate::i18n::tr(label);
        self.document_view_button.set_tooltip_text(Some(&label));
        super::super::accessibility::set_label(&self.document_view_button, &label);
        crate::assets::set_primary_icon(&self.document_view_icon, icon);
    }

    pub(super) fn apply_text_wrap(&self, wrapped: bool) {
        self.source_preview.set_wrapped(wrapped);
        if let Some(preview) = self.document_preview.borrow().as_ref()
            && let Some(state) = preview.rendered_state.borrow().as_ref()
        {
            state.set_wrapped(wrapped);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DocumentView {
    Rendered,
    Source,
}

pub(super) struct DocumentPreview {
    pub(super) stack: gtk::Stack,
    pub(super) source: Option<PendingSourcePreview>,
    pub(super) rendered: Option<(DocumentLayout, Vec<String>)>,
    pub(super) rendered_state:
        RefCell<Option<Rc<super::super::virtual_preview::VirtualPreviewState>>>,
    pub(super) render_pending: bool,
}

pub(super) fn initial_document_view(
    prefer_rendered: bool,
    rendered_available: bool,
) -> DocumentView {
    if prefer_rendered && rendered_available {
        DocumentView::Rendered
    } else {
        DocumentView::Source
    }
}

pub(super) fn document_view_name(view: DocumentView) -> &'static str {
    match view {
        DocumentView::Rendered => "rendered",
        DocumentView::Source => "source",
    }
}

pub(super) fn document_view_action(view: DocumentView) -> (&'static str, &'static str) {
    match view {
        DocumentView::Rendered => ("View source", crate::assets::icons::FILE_CODE),
        DocumentView::Source => ("View rendered", crate::assets::icons::DOCUMENTS),
    }
}

pub(in crate::ui) fn document_notice(message: &str) -> gtk::Label {
    let notice = gtk::Label::new(Some(message));
    notice.add_css_class("preview-document-notice");
    notice.set_hexpand(true);
    notice.set_selectable(true);
    notice.set_wrap(true);
    notice.set_xalign(0.0);
    notice
}
