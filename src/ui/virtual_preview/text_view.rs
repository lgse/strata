// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::{glib, prelude::*};
use sourceview5::prelude::*;

use super::BoundRow;
use super::rows::{document_uses_source_buffer, document_wrap_mode};
use super::tags::{apply_document_unit_tags, ensure_document_text_tags};
use crate::services::{
    DocumentListChildKind, DocumentSpanStyle, DocumentUnit, DocumentUnitKind,
    normalize_preview_text,
};

pub(super) fn rendered_width(width: i32) -> i32 {
    width.min(super::super::preview::media_layout::MAX_CONTENT_WIDTH)
}

pub(super) fn size_document_row(bound: &BoundRow, width: i32) {
    if bound.mapped_root().is_none() {
        return;
    }
    let Some(view) = bound.view.upgrade() else {
        return;
    };
    size_document_text_view(&view, width);
}

pub(in crate::ui) fn schedule_document_view_size(
    view: &super::super::document_view::DocumentTextView,
    fallback_width: i32,
) {
    // Mapping schedules the first measurement; prefetched rows need no idle work.
    if !view.is_mapped() {
        return;
    }
    let view = view.downgrade();
    glib::idle_add_local_once(move || {
        let Some(view) = view.upgrade().filter(|view| view.is_mapped()) else {
            return;
        };
        let width = if view.width() > 0 {
            view.width()
        } else {
            fallback_width
        };
        size_document_text_view(&view, width);
    });
}

pub(super) fn size_document_text_view(
    view: &super::super::document_view::DocumentTextView,
    width: i32,
) {
    view.set_height_request(-1);
    // Unwrapped lines must widen the list, or the scroller has nothing to scroll.
    let width = if view.wrap_mode() == gtk::WrapMode::None {
        let unwrapped = unwrapped_document_width(view);
        view.set_width_request(unwrapped);
        width.max(unwrapped)
    } else {
        view.set_width_request(-1);
        width
    };
    let (minimum, natural, _, _) = view.measure(
        gtk::Orientation::Vertical,
        if width > 0 { width } else { -1 },
    );
    view.set_height_request(minimum.max(natural));
}

pub(super) fn unwrapped_document_width(
    view: &super::super::document_view::DocumentTextView,
) -> i32 {
    let buffer = view.buffer();
    let mut line = buffer.start_iter();
    let mut widest = 0;
    loop {
        let mut end = line;
        if !end.ends_line() {
            end.forward_to_line_end();
        }
        let location = view.iter_location(&end);
        widest = widest.max(location.x() + location.width() + line_right_margin(view, &line));
        if !line.forward_line() {
            break;
        }
    }
    widest
}

/// The larger of the view's right margin and the line's tagged one, so code
/// lines keep room for their inset and copy button without narrowing others.
fn line_right_margin(
    view: &super::super::document_view::DocumentTextView,
    line: &gtk::TextIter,
) -> i32 {
    line.tags()
        .iter()
        .rev()
        .find(|tag| tag.is_right_margin_set())
        .map_or(0, gtk::TextTag::right_margin)
        .max(view.right_margin())
}

pub(super) fn set_text_view_content(view: &gtk::TextView, text: &str, padding: i32) {
    view.buffer().set_text(text);
    let (width, height) = view.create_pango_layout(Some(text)).pixel_size();
    view.set_size_request(width.saturating_add(padding), height);
}

pub(super) fn plain_text_view(
    text: &str,
    source: bool,
) -> super::super::document_view::DocumentTextView {
    let text = normalize_preview_text(text);
    let buffer = gtk::TextBuffer::new(None);
    assert!(
        buffer
            .tag_table()
            .add(&gtk::TextTag::builder().name("document-selection").build())
    );
    super::super::theme::register_document_buffer(&buffer);
    let view = super::super::document_view::DocumentTextView::new(&buffer, Vec::new());
    view.set_editable(false);
    view.set_cursor_visible(false);
    view.set_accepts_tab(false);
    view.set_wrap_mode(gtk::WrapMode::None);
    view.set_left_margin(if source { 0 } else { 16 });
    view.set_right_margin(16);
    view.set_can_target(false);
    view.set_focusable(false);
    view.add_css_class(if source {
        "preview-text"
    } else {
        "preview-document"
    });
    super::super::theme::register_document_view(&view);
    set_text_view_content(view.upcast_ref(), &text, view.right_margin());
    view.set_hexpand(true);
    view
}

pub(super) fn document_text_view(
    unit: &DocumentUnit,
    document_tags: &gtk::TextTagTable,
    wrapped: bool,
) -> super::super::document_view::DocumentTextView {
    let buffer = if document_uses_source_buffer(unit) {
        let buffer = super::super::theme::new_source_buffer();
        buffer.upcast::<gtk::TextBuffer>()
    } else {
        gtk::TextBuffer::new(Some(document_tags))
    };
    ensure_document_text_tags(&buffer);
    super::super::theme::register_document_buffer(&buffer);
    let view = super::super::document_view::DocumentTextView::new(&buffer, Vec::new());
    view.connect_map(|view| schedule_document_view_size(view, view.width()));
    view.set_editable(false);
    view.set_cursor_visible(false);
    view.set_accepts_tab(false);
    view.set_left_margin(super::tags::DOCUMENT_MARGIN);
    view.set_right_margin(super::tags::DOCUMENT_MARGIN);
    view.set_can_target(false);
    view.set_focusable(false);
    view.add_css_class("preview-document");
    super::super::theme::register_document_view(&view);
    bind_document_text_view(&view, unit, wrapped);
    view
}

pub(super) fn bind_document_text_view(
    view: &super::super::document_view::DocumentTextView,
    unit: &DocumentUnit,
    wrapped: bool,
) {
    let buffer = view.buffer();
    let text = normalize_preview_text(&unit.text);
    if let Ok(source_buffer) = buffer.clone().downcast::<sourceview5::Buffer>() {
        let language = highlighted_code_language(unit)
            .and_then(|language| super::super::theme::source_languages().language(language));
        source_buffer.set_highlight_syntax(language.is_some());
        source_buffer.set_language(language.as_ref());
        source_buffer.set_text(&text);
        if language.is_some() {
            source_buffer.ensure_highlight(&source_buffer.start_iter(), &source_buffer.end_iter());
        }
    } else {
        buffer.set_text(&text);
    }
    apply_document_unit_tags(&buffer, unit);

    let is_code = matches!(unit.kind, DocumentUnitKind::Code { .. });
    view.set_code_blocks(
        is_code
            .then(|| super::super::document_view::DocumentCodeBlock {
                start: 0,
                end: buffer.char_count(),
                first: unit.first,
                last: unit.last,
            })
            .into_iter()
            .collect(),
    );
    view.set_selection_range(None);
    view.set_wrap_mode(document_wrap_mode(wrapped));
    set_document_accessibility(view, unit);
}

pub(super) fn bind_inline_math(
    view: &super::super::document_view::DocumentTextView,
    unit: &DocumentUnit,
    index: usize,
    cache: &Rc<super::super::document_media::MediaCache>,
) {
    let buffer = view.buffer();
    for span in &unit.spans {
        let DocumentSpanStyle::Math(source) = &span.style else {
            continue;
        };
        let mut start = buffer.iter_at_offset(span.range.start as i32);
        let mut end = buffer.iter_at_offset(span.range.end as i32);
        buffer.delete(&mut start, &mut end);
        let anchor = buffer.create_child_anchor(&mut start);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.set_valign(gtk::Align::Baseline);
        view.add_child_at_anchor(&content, &anchor);
        cache.bind(
            (index, Some(span.range.start)),
            &crate::services::DocumentMedia::Math {
                source: source.to_string(),
                display: false,
            },
            source,
            &content,
        );
    }
}

pub(super) fn highlighted_code_language(unit: &DocumentUnit) -> Option<&'static str> {
    match unit.kind {
        DocumentUnitKind::Code {
            language: Some(language),
            ..
        } if unit.first && unit.last => Some(language),
        _ => None,
    }
}

pub(super) fn set_document_accessibility(
    view: &super::super::document_view::DocumentTextView,
    unit: &DocumentUnit,
) {
    view.reset_property(gtk::AccessibleProperty::Level);
    view.reset_property(gtk::AccessibleProperty::Label);
    let has_math = unit
        .spans
        .iter()
        .any(|span| matches!(span.style, DocumentSpanStyle::Math(_)));
    match unit.kind {
        DocumentUnitKind::Heading(level)
        | DocumentUnitKind::ListChild {
            kind: DocumentListChildKind::Heading(level),
            ..
        } => {
            view.set_accessible_role(gtk::AccessibleRole::Heading);
            view.update_property(&[gtk::accessible::Property::Level(i32::from(level))]);
        }
        DocumentUnitKind::ListItem { .. } => {
            view.set_accessible_role(gtk::AccessibleRole::ListItem);
        }
        _ => view.set_accessible_role(if has_math {
            gtk::AccessibleRole::Group
        } else {
            gtk::AccessibleRole::Generic
        }),
    }
    if has_math {
        // GTK's Text interface omits child anchors. Keep complete prose and TeX
        // available to assistive technology rather than exposing an empty object.
        view.update_property(&[gtk::accessible::Property::Label(unit.copy_text.trim_end())]);
    }
}
