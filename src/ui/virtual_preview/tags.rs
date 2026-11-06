// SPDX-License-Identifier: MIT

use gtk::prelude::*;

use crate::services::{DocumentListChildKind, DocumentSpanStyle, DocumentUnit, DocumentUnitKind};

/// The left and right margin of rendered document text views.
pub(super) const DOCUMENT_MARGIN: i32 = 16;
const CODE_INSET: i32 = 10;
/// Keeps the first code line clear of the copy button, which `.preview-code-copy`
/// places 24px from the right edge with a 27px content box, 3px padding and a 1px border.
const CODE_COPY_BUTTON_SPACE: i32 = 24 + 27 + 2 * 3 + 2 + CODE_INSET;

pub(super) fn apply_document_unit_tags(buffer: &gtk::TextBuffer, unit: &DocumentUnit) {
    let start = buffer.start_iter();
    let end = buffer.end_iter();
    let apply = |name: &str| buffer.apply_tag(&document_tag(buffer, name), &start, &end);
    match &unit.kind {
        DocumentUnitKind::Heading(level) => {
            apply(&format!(
                "document-heading-{}",
                usize::from(*level).clamp(1, 6)
            ));
            apply("document-accent");
        }
        DocumentUnitKind::ListItem { depth } => {
            apply(&if unit.first {
                format!("document-list-{}", depth.min(&32))
            } else {
                format!("document-list-child-{}", depth.min(&32))
            });
        }
        DocumentUnitKind::ListChild { depth, kind } => {
            apply(&format!("document-list-child-{}", depth.min(&32)));
            match kind {
                DocumentListChildKind::Heading(level) => {
                    apply(&format!(
                        "document-heading-{}",
                        usize::from(*level).clamp(1, 6)
                    ));
                    apply("document-accent");
                }
                DocumentListChildKind::Quote => {
                    apply("document-quote");
                    apply("document-dim");
                }
                DocumentListChildKind::Paragraph => {}
                DocumentListChildKind::Code(_) => unreachable!(),
            }
        }
        DocumentUnitKind::Quote => {
            apply("document-quote");
            apply("document-dim");
        }
        DocumentUnitKind::Code { list_depth, .. } => {
            apply("document-code");
            if let Some(depth) = list_depth {
                apply(&format!("document-code-list-{}", depth.min(&32)));
            }
            if unit.first && buffer.char_count() > 0 {
                buffer.apply_tag(
                    &document_tag(buffer, "document-code-start"),
                    &start,
                    &buffer.iter_at_offset(1),
                );
                let mut first_line_end = start;
                first_line_end.forward_to_line_end();
                buffer.apply_tag(
                    &document_tag(buffer, "document-code-copy-space"),
                    &start,
                    &first_line_end,
                );
            }
            if unit.last && buffer.char_count() > 0 {
                let mut last = end;
                last.set_line_offset(0);
                buffer.apply_tag(&document_tag(buffer, "document-code-end"), &last, &end);
            }
        }
        DocumentUnitKind::Rule { list_depth } => {
            apply("document-rule");
            apply("document-dim");
            if let Some(depth) = list_depth {
                apply(&format!("document-list-child-{}", depth.min(&32)));
            }
        }
        DocumentUnitKind::Paragraph
        | DocumentUnitKind::Table { .. }
        | DocumentUnitKind::Media { .. }
        | DocumentUnitKind::Gap => {}
    }

    for span in &unit.spans {
        let start = buffer.iter_at_offset(i32::try_from(span.range.start).unwrap_or(i32::MAX));
        let end = buffer.iter_at_offset(i32::try_from(span.range.end).unwrap_or(i32::MAX));
        let tag = document_tag(
            buffer,
            match span.style {
                DocumentSpanStyle::Accent => "document-accent",
                DocumentSpanStyle::Bold => "document-bold",
                DocumentSpanStyle::Italic => "document-italic",
                DocumentSpanStyle::Strikethrough => "document-strikethrough",
                DocumentSpanStyle::Monospace => "document-monospace",
                DocumentSpanStyle::Underline => "document-underline",
                DocumentSpanStyle::Link(_) => "document-link",
                DocumentSpanStyle::Math(_) => "document-monospace",
            },
        );
        buffer.apply_tag(&tag, &start, &end);
        if matches!(span.style, DocumentSpanStyle::Link(_)) {
            buffer.apply_tag(&document_tag(buffer, "document-accent"), &start, &end);
        }
    }
}

pub(super) fn document_tag(buffer: &gtk::TextBuffer, name: &str) -> gtk::TextTag {
    buffer
        .tag_table()
        .lookup(name)
        .unwrap_or_else(|| panic!("missing document text tag {name}"))
}

pub(super) fn ensure_document_text_tags(buffer: &gtk::TextBuffer) {
    let table = buffer.tag_table();
    if table.lookup("document-heading-1").is_some() {
        return;
    }
    let add = |tag: gtk::TextTag| {
        assert!(table.add(&tag));
        tag
    };
    let scales = [1.6, 1.35, 1.18, 1.04, 1.04, 1.04];
    for (index, scale) in scales.into_iter().enumerate() {
        add(gtk::TextTag::builder()
            .name(format!("document-heading-{}", index + 1))
            .weight(700)
            .scale(scale)
            .build());
    }
    add(gtk::TextTag::builder()
        .name("document-quote")
        .style(gtk::pango::Style::Italic)
        .left_margin(12)
        .right_margin(8)
        .line_height(1.2)
        .background_full_height(true)
        .build());
    // Code keeps the view's wrap mode, so wrapped lines need margins rather
    // than a first-line indent to stay inside the code panel.
    add(gtk::TextTag::builder()
        .name("document-code")
        .family("monospace")
        .left_margin(DOCUMENT_MARGIN + CODE_INSET)
        .right_margin(DOCUMENT_MARGIN + CODE_INSET)
        .line_height(1.15)
        .build());
    add(gtk::TextTag::builder()
        .name("document-code-start")
        .pixels_above_lines(super::super::document_view::CODE_TOP_SPACE)
        .build());
    add(gtk::TextTag::builder()
        .name("document-code-copy-space")
        .right_margin(CODE_COPY_BUTTON_SPACE)
        .build());
    add(gtk::TextTag::builder()
        .name("document-code-end")
        .pixels_below_lines(super::super::document_view::CODE_BOTTOM_SPACE)
        .build());
    add(gtk::TextTag::builder().name("document-rule").build());
    for depth in 0..=32 {
        add(gtk::TextTag::builder()
            .name(format!("document-list-{depth}"))
            .left_margin(20 + depth * 18)
            .indent(-18)
            .build());
        add(gtk::TextTag::builder()
            .name(format!("document-list-child-{depth}"))
            .left_margin(20 + depth * 18)
            .build());
        add(gtk::TextTag::builder()
            .name(format!("document-code-list-{depth}"))
            .left_margin(20 + depth * 18 + CODE_INSET)
            .build());
    }
    add(gtk::TextTag::builder().name("document-dim").build());
    add(gtk::TextTag::builder().name("document-accent").build());
    add(gtk::TextTag::builder()
        .name("document-link")
        .underline(gtk::pango::Underline::Single)
        .build());
    add(gtk::TextTag::builder()
        .name("document-bold")
        .weight(700)
        .build());
    add(gtk::TextTag::builder()
        .name("document-italic")
        .style(gtk::pango::Style::Italic)
        .build());
    add(gtk::TextTag::builder()
        .name("document-strikethrough")
        .strikethrough(true)
        .build());
    add(gtk::TextTag::builder()
        .name("document-monospace")
        .family("monospace")
        .build());
    add(gtk::TextTag::builder()
        .name("document-underline")
        .underline(gtk::pango::Underline::Single)
        .build());
    add(gtk::TextTag::builder().name("document-link-hover").build());
    add(gtk::TextTag::builder().name("document-selection").build());
}

pub(super) fn document_tag_table() -> gtk::TextTagTable {
    let table = gtk::TextTagTable::new();
    let buffer = gtk::TextBuffer::new(Some(&table));
    ensure_document_text_tags(&buffer);
    table
}
