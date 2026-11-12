// SPDX-License-Identifier: MIT

mod html;
mod layout;
mod limits;
mod lists;
mod markdown;
mod media;

use super::{
    DocumentBlock, DocumentKind, DocumentLayout, DocumentListChildKind, DocumentTableCell,
    DocumentUnit, ParsedDocument, parse_document,
};
use crate::sandbox::Cancellation;

fn parse(kind: DocumentKind, source: &str, expectation: &str) -> ParsedDocument {
    parse_document(kind, source, &Cancellation::default()).expect(expectation)
}

fn parsed_blocks(kind: DocumentKind, source: &str, expectation: &str) -> Vec<DocumentBlock> {
    parse(kind, source, expectation).document.blocks
}

fn parse_error(kind: DocumentKind, source: &str, expectation: &str) -> String {
    parse_document(kind, source, &Cancellation::default()).expect_err(expectation)
}

fn assert_parses_to(
    kind: DocumentKind,
    source: &str,
    expectation: &str,
    expected: &[DocumentBlock],
) {
    assert_eq!(
        parsed_blocks(kind, source, expectation),
        expected,
        "{source:?}"
    );
}

fn debug_blocks(blocks: &[DocumentBlock]) -> String {
    format!("{blocks:?}")
}

/// Asserts the debug rendering of parsed blocks keeps every `kept` fragment
/// and none of the `omitted` ones.
fn assert_debug_mentions(debug: &str, kept: &[&str], omitted: &[&str]) {
    for fragment in kept {
        assert!(debug.contains(fragment), "must keep {fragment}: {debug}");
    }
    for fragment in omitted {
        assert!(!debug.contains(fragment), "must not retain {fragment}");
    }
}

fn paragraph(markup: &str) -> DocumentBlock {
    DocumentBlock::Paragraph(markup.to_owned())
}

fn heading(level: u8, markup: &str) -> DocumentBlock {
    DocumentBlock::Heading {
        level,
        markup: markup.to_owned(),
    }
}

fn list_item(marker: &str, depth: usize, markup: &str) -> DocumentBlock {
    DocumentBlock::ListItem {
        marker: marker.to_owned(),
        depth,
        markup: markup.to_owned(),
    }
}

fn bullet(depth: usize, markup: &str) -> DocumentBlock {
    list_item("•", depth, markup)
}

fn list_child(kind: DocumentListChildKind, markup: &str) -> DocumentBlock {
    DocumentBlock::ListChild {
        depth: 0,
        kind,
        markup: markup.to_owned(),
    }
}

fn cell(header: bool, markup: &str) -> DocumentTableCell {
    DocumentTableCell {
        header,
        markup: markup.to_owned(),
    }
}

fn table_row(cells: Vec<DocumentTableCell>) -> DocumentBlock {
    DocumentBlock::TableRow { cells }
}

fn copied_text(layout: &DocumentLayout) -> String {
    layout
        .units
        .iter()
        .map(|unit| unit.copy_text.as_str())
        .collect()
}

fn units_fit_the_line_target(units: &[DocumentUnit]) -> bool {
    units
        .iter()
        .all(|unit| unit.text.len() <= super::DOCUMENT_UNIT_LINE_TARGET)
}
