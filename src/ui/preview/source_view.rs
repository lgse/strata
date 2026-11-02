// SPDX-License-Identifier: MIT

use super::*;

impl SourcePreviewView {
    pub(super) fn new() -> Self {
        let buffer = super::super::theme::new_source_buffer();
        let view = sourceview5::View::builder()
            .buffer(&buffer)
            .cursor_visible(true)
            .editable(false)
            .highlight_current_line(false)
            .left_margin(14)
            .right_margin(14)
            .top_margin(12)
            .bottom_margin(12)
            .monospace(true)
            .show_line_numbers(true)
            .wrap_mode(gtk::WrapMode::None)
            .build();
        view.add_css_class("preview-text");
        Self {
            view,
            scroll: RefCell::new(None),
            virtual_state: RefCell::new(None),
            generation: Rc::new(Cell::new(0)),
        }
    }

    pub(super) fn show(
        &self,
        entry: &FileEntry,
        content_type: &str,
        content: &str,
        truncated: bool,
    ) -> (gtk::Widget, bool) {
        self.cancel();
        self.scroll.borrow_mut().take();
        self.virtual_state.borrow_mut().take();
        if let Some(parent) = self.view.parent() {
            if let Ok(scroll) = parent.downcast::<gtk::ScrolledWindow>() {
                scroll.set_child(None::<&gtk::Widget>);
            } else {
                self.view.unparent();
            }
        }
        let languages = super::super::theme::source_languages();
        let language = languages.guess_language(entry.location.native_path(), Some(content_type));
        if language.is_none() && super::super::virtual_preview::use_virtual_plain_source(content) {
            let wrapped =
                super::super::preferences::PreferenceManager::shared().preview_text_wrap();
            let (container, state) =
                super::super::virtual_preview::source_document(content, truncated, wrapped);
            self.virtual_state.replace(Some(state));
            return (container.upcast(), true);
        }
        let display = normalize_preview_text(content).into_owned();
        let buffer = super::super::theme::new_source_buffer();
        buffer.set_highlight_syntax(false);
        buffer.set_language(language.as_ref());
        self.view.set_buffer(Some(&buffer));
        let wrapped = super::super::preferences::PreferenceManager::shared().preview_text_wrap();
        self.view.set_wrap_mode(text_wrap_mode(wrapped));
        fill_source_buffer(
            &buffer,
            display,
            self.generation.clone(),
            self.generation.get(),
            language.is_some(),
        );

        let scroll = gtk::ScrolledWindow::builder()
            .child(&self.view)
            .hscrollbar_policy(text_hscroll_policy(wrapped))
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .hexpand(true)
            .vexpand(true)
            .build();
        self.scroll.replace(Some(scroll.clone()));
        (scroll.upcast(), false)
    }

    pub(super) fn set_wrapped(&self, wrapped: bool) {
        self.view.set_wrap_mode(text_wrap_mode(wrapped));
        if let Some(scroll) = self.scroll.borrow().as_ref() {
            scroll.set_hscrollbar_policy(text_hscroll_policy(wrapped));
        }
        if let Some(state) = self.virtual_state.borrow().as_ref() {
            state.set_wrapped(wrapped);
        }
    }

    pub(super) fn cancel(&self) {
        self.generation.set(self.generation.get().saturating_add(1));
    }
}

pub(super) struct PendingSourcePreview {
    pub(super) entry: FileEntry,
    pub(super) content_type: String,
    pub(super) content: String,
    pub(super) truncated: bool,
}

pub(super) struct SourcePreviewView {
    pub(super) view: sourceview5::View,
    pub(super) scroll: RefCell<Option<gtk::ScrolledWindow>>,
    pub(super) virtual_state:
        RefCell<Option<Rc<super::super::virtual_preview::VirtualPreviewState>>>,
    pub(super) generation: Rc<Cell<u64>>,
}

pub(super) fn fill_source_buffer(
    buffer: &sourceview5::Buffer,
    text: String,
    generation: Rc<Cell<u64>>,
    expected_generation: u64,
    highlight_syntax: bool,
) {
    let text = normalize_preview_text(&text).into_owned();
    let first_end = source_chunk_end(&text, 0);
    buffer.set_text(&text[..first_end]);
    if first_end == text.len() {
        buffer.set_modified(false);
        buffer.set_highlight_syntax(highlight_syntax);
        return;
    }

    let weak_buffer = buffer.downgrade();
    let mut offset = first_end;
    glib::idle_add_local(move || {
        if generation.get() != expected_generation {
            return glib::ControlFlow::Break;
        }
        let Some(buffer) = weak_buffer.upgrade() else {
            return glib::ControlFlow::Break;
        };
        let end = source_chunk_end(&text, offset);
        buffer.insert(&mut buffer.end_iter(), &text[offset..end]);
        offset = end;
        if offset == text.len() {
            buffer.set_modified(false);
            buffer.set_highlight_syntax(highlight_syntax);
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

pub(super) fn source_chunk_end(text: &str, start: usize) -> usize {
    let mut end = start
        .saturating_add(SOURCE_INSERT_CHUNK_BYTES)
        .min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    if let Some(line_end) = text[start..end]
        .match_indices('\n')
        .nth(SOURCE_INSERT_CHUNK_LINES - 1)
        .map(|(offset, _)| start + offset + 1)
    {
        end = line_end;
    }
    end
}

pub(super) fn text_wrap_mode(wrapped: bool) -> gtk::WrapMode {
    if wrapped {
        gtk::WrapMode::Word
    } else {
        gtk::WrapMode::None
    }
}

pub(super) fn text_hscroll_policy(wrapped: bool) -> gtk::PolicyType {
    if wrapped {
        gtk::PolicyType::Never
    } else {
        gtk::PolicyType::Automatic
    }
}
