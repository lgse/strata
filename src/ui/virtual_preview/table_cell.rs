// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;

use gtk::{glib, prelude::*};

use super::{TABLE_CELL_DISPLAY_BYTES, TABLE_CELL_MARKUP_BYTES};
use crate::services::{DocumentSpan, DocumentSpanStyle, DocumentTableCellLayout};

pub(in crate::ui) fn set_table_cell(label: &gtk::Label, cell: &DocumentTableCellLayout) {
    crate::ui::accessibility::set_description(label, None);
    if cell.text.len() > TABLE_CELL_DISPLAY_BYTES {
        label.set_text(&format!(
            "{}…",
            bounded_text_prefix(&cell.text, TABLE_CELL_DISPLAY_BYTES)
        ));
        crate::ui::accessibility::set_description(
            label,
            Some(&crate::i18n::tr(
                "Cell shortened for responsive preview; copying the table keeps the complete text.",
            )),
        );
    } else if let Some(markup) = styled_markup(&cell.text, &cell.spans) {
        label.set_markup(&markup);
    } else {
        label.set_text(&cell.text);
        crate::ui::accessibility::set_description(
            label,
            Some(&crate::i18n::tr(
                "Cell formatting omitted for responsive preview; copying keeps the complete text.",
            )),
        );
    }
    if cell.header {
        label.add_css_class("header");
    } else {
        label.remove_css_class("header");
    }
}

pub(super) fn styled_markup(text: &str, spans: &[DocumentSpan]) -> Option<String> {
    let mut events = Vec::with_capacity(spans.len() * 2);
    for (index, span) in spans.iter().enumerate() {
        events.push((span.range.start, false, index));
        events.push((span.range.end, true, index));
    }
    events.sort_unstable_by_key(|(position, ending, index)| (*position, !*ending, *index));
    let boundaries = text
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(text.len()))
        .collect::<Vec<_>>();
    let mut output = String::with_capacity(text.len());
    let mut active = BTreeSet::<usize>::new();
    let mut cursor = 0;
    let mut event = 0;
    while event < events.len() {
        let position = events[event].0.min(boundaries.len().saturating_sub(1));
        append_table_markup(
            &mut output,
            &glib::markup_escape_text(&text[boundaries[cursor]..boundaries[position]]),
        )?;
        for index in active.iter().rev() {
            append_table_markup(&mut output, span_close(&spans[*index].style))?;
        }
        while event < events.len() && events[event].0 == position && events[event].1 {
            active.remove(&events[event].2);
            event += 1;
        }
        while event < events.len() && events[event].0 == position && !events[event].1 {
            active.insert(events[event].2);
            event += 1;
        }
        for index in &active {
            append_table_markup(&mut output, &span_open(&spans[*index].style))?;
        }
        cursor = position;
    }
    append_table_markup(
        &mut output,
        &glib::markup_escape_text(&text[boundaries[cursor]..]),
    )?;
    for index in active.iter().rev() {
        append_table_markup(&mut output, span_close(&spans[*index].style))?;
    }
    Some(output)
}

pub(super) fn append_table_markup(output: &mut String, markup: &str) -> Option<()> {
    if output.len().saturating_add(markup.len()) > TABLE_CELL_MARKUP_BYTES {
        return None;
    }
    output.push_str(markup);
    Some(())
}

pub(super) fn span_open(style: &DocumentSpanStyle) -> String {
    match style {
        DocumentSpanStyle::Accent | DocumentSpanStyle::Math(_) => String::new(),
        DocumentSpanStyle::Bold => "<b>".to_owned(),
        DocumentSpanStyle::Italic => "<i>".to_owned(),
        DocumentSpanStyle::Strikethrough => "<s>".to_owned(),
        DocumentSpanStyle::Monospace => "<tt>".to_owned(),
        DocumentSpanStyle::Underline => "<u>".to_owned(),
        DocumentSpanStyle::Link(uri) => {
            format!("<a href=\"{}\">", glib::markup_escape_text(uri))
        }
    }
}

pub(super) fn span_close(style: &DocumentSpanStyle) -> &'static str {
    match style {
        DocumentSpanStyle::Accent | DocumentSpanStyle::Math(_) => "",
        DocumentSpanStyle::Bold => "</b>",
        DocumentSpanStyle::Italic => "</i>",
        DocumentSpanStyle::Strikethrough => "</s>",
        DocumentSpanStyle::Monospace => "</tt>",
        DocumentSpanStyle::Underline => "</u>",
        DocumentSpanStyle::Link(_) => "</a>",
    }
}

pub(super) fn bounded_text_prefix(text: &str, bytes: usize) -> &str {
    let mut end = text.len().min(bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
