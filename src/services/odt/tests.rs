// SPDX-License-Identifier: MIT

use std::{io::Write, path::PathBuf};

use zip::{ZipWriter, write::SimpleFileOptions};

use super::*;
use crate::{
    sandbox::{Cancellation, DocumentFormat},
    services::DocumentBlock,
};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/documents/report.odt")
}

fn package(directory: &Path, parts: &[(&str, &str)]) -> PathBuf {
    let path = directory.join("document.odt");
    let mut writer = ZipWriter::new(File::create(&path).expect("package file"));
    for (name, contents) in parts {
        writer
            .start_file(*name, SimpleFileOptions::default())
            .expect("package part");
        writer
            .write_all(contents.as_bytes())
            .expect("part contents");
    }
    writer.finish().expect("finished package");
    path
}

fn content(body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"><office:body><office:text>{body}</office:text></office:body></office:document-content>"#
    )
}

fn read_body(body: &str) -> RichTextData {
    let directory = tempfile::tempdir().expect("temporary directory");
    read_document(&package(
        directory.path(),
        &[("content.xml", &content(body))],
    ))
    .expect("document package")
}

#[test]
fn routes_opendocument_text_and_templates_by_content_type_and_extension() {
    for (content_type, name, expected) in [
        (
            "application/vnd.oasis.opendocument.text",
            "report.bin",
            Some(DocumentFormat::Odt),
        ),
        (
            "application/vnd.oasis.opendocument.text-template",
            "letter",
            Some(DocumentFormat::Odt),
        ),
        (
            "application/octet-stream",
            "notes.ODT",
            Some(DocumentFormat::Odt),
        ),
        (
            "application/octet-stream",
            "letter.Ott",
            Some(DocumentFormat::Odt),
        ),
        (
            "application/octet-stream",
            "report.docx",
            Some(DocumentFormat::Docx),
        ),
        (
            "application/vnd.oasis.opendocument.spreadsheet",
            "book.ods",
            None,
        ),
        (
            "application/vnd.oasis.opendocument.presentation",
            "deck.odp",
            None,
        ),
    ] {
        assert_eq!(
            DocumentFormat::for_file(content_type, OsStr::new(name)),
            expected,
            "{content_type} {name}"
        );
    }
}

#[test]
fn reads_headings_styles_lists_tables_and_code_from_an_opendocument_text() {
    let data = read_document(&fixture()).expect("document fixture");
    assert!(!data.truncated);
    let html = data.html;
    assert!(
        html.starts_with("<h1>Quarterly Report</h1><h2>Regional summary</h2>"),
        "{html}"
    );
    assert!(
        html.contains(
            "<p>Mixed <b>bold</b>, <i>italic</i>, <u>underlined</u>, <s>struck</s>, \
             <b>strong</b> and <code>code</code> text with A &amp; B &lt; C.</p>"
        ),
        "{html}"
    );
    assert!(
        html.contains("<p><b>Bold paragraph</b> with a plain tail</p>"),
        "{html}"
    );
    assert!(
        html.contains("<blockquote>Growth held steady.</blockquote>"),
        "{html}"
    );
    assert!(
        html.contains(
            "<ul><li><p>First bullet</p><ul><li><p>Nested bullet</p></li></ul></li></ul>"
        ),
        "{html}"
    );
    assert!(
        html.contains("<ol><li><p>First step</p></li><li><p>Second step</p></li></ol>"),
        "{html}"
    );
    assert!(
        html.contains(
            "<table><tr><th>Region</th><th>Total</th></tr><tr><td>North</td><td>42</td></tr>"
        ),
        "{html}"
    );
    assert!(
        html.contains("<pre>fn main() {\n    run();\n}</pre>"),
        "{html}"
    );
    assert!(html.contains("<p>Line one<br>line two</p>"), "{html}");
    assert!(
        html.contains(
            "<p>See <a href=\"https://example.com/report\">the full report</a> and the totals.</p>"
        ),
        "{html}"
    );
    for omitted in [
        "Deleted draft",
        "Footnote body",
        "Reviewer comment",
        "Framed caption",
        "Running header",
    ] {
        assert!(!html.contains(omitted), "{omitted}: {html}");
    }
}

#[test]
fn derives_rendered_blocks_without_omission_warnings() {
    let parsed = read_document(&fixture())
        .expect("document fixture")
        .into_document(&Cancellation::default())
        .expect("rendered document");
    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
    let blocks = &parsed.document.blocks;
    assert!(matches!(
        blocks.first(),
        Some(DocumentBlock::Heading { level: 1, .. })
    ));
    assert!(
        blocks
            .iter()
            .any(|block| matches!(block, DocumentBlock::Quote(_)))
    );
    assert!(blocks.iter().any(|block| matches!(
        block,
        DocumentBlock::ListItem { depth: 1, markup, .. } if markup == "Nested bullet"
    )));
    assert!(blocks.iter().any(|block| matches!(
        block,
        DocumentBlock::Code { markup, .. } if markup.contains("\n    run();\n")
    )));
    assert!(blocks.iter().any(|block| matches!(
        block,
        DocumentBlock::TableRow { cells } if cells.iter().all(|cell| cell.header)
    )));
}

#[test]
fn long_documents_truncate_within_the_rendered_parser_budgets() {
    let long_paragraph = format!("<text:p>{}</text:p>", "x".repeat(64 * 1024));
    let cell = "<text:p><text:line-break/>x<text:line-break/></text:p>".repeat(50);
    let row =
        format!("<table:table-row><table:table-cell>{cell}</table:table-cell></table:table-row>");
    for (case, body) in [
        ("bytes", long_paragraph.repeat(24)),
        ("events", "<text:p>x</text:p>".repeat(12_000)),
        (
            "cell paragraph separators",
            format!("<table:table>{}</table:table>", row.repeat(130)),
        ),
    ] {
        let data = read_body(&body);
        assert!(data.truncated, "{case}");
        let parsed = data
            .into_document(&Cancellation::default())
            .unwrap_or_else(|error| panic!("{case}: {error}"));
        assert!(
            parsed
                .warnings
                .iter()
                .any(|warning| warning.contains("size budget")),
            "{case}"
        );
    }
}

#[test]
fn deeply_nested_lists_stay_within_the_rendered_nesting_limit() {
    let depth = 20;
    let body = format!(
        "{}<text:p>Deepest item</text:p>{}",
        "<text:list><text:list-item>".repeat(depth),
        "</text:list-item></text:list>".repeat(depth)
    );
    let parsed = read_body(&body)
        .into_document(&Cancellation::default())
        .expect("rendered document");
    assert!(parsed.document.blocks.iter().any(|block| matches!(
        block,
        DocumentBlock::ListItem { markup, .. } | DocumentBlock::ListChild { markup, .. }
            if markup == "Deepest item"
    )));
}

#[test]
fn table_cells_keep_merged_columns_and_flatten_nested_tables() {
    let html = read_body(
        "<table:table><table:table-row>\
         <table:table-cell table:number-columns-spanned=\"2\"><text:p>Wide</text:p></table:table-cell>\
         <table:covered-table-cell/>\
         <table:table-cell><text:p>Outer</text:p><table:table><table:table-row>\
         <table:table-cell><text:p>Inner</text:p></table:table-cell>\
         <table:table-cell><text:list><text:list-item><text:p>Listed</text:p></text:list-item></text:list></table:table-cell>\
         </table:table-row></table:table></table:table-cell>\
         </table:table-row></table:table>",
    )
    .html;
    assert_eq!(
        html,
        "<table><tr><td>Wide</td><td></td><td>Outer Inner Listed</td></tr></table>"
    );
}

#[test]
fn rejects_unreadable_packages() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let not_zip = directory.path().join("not-a-document.odt");
    std::fs::write(&not_zip, b"not a zip archive").expect("fixture write");
    assert!(read_document(&not_zip).is_err());

    let empty = package(
        directory.path(),
        &[("mimetype", "application/vnd.oasis.opendocument.text")],
    );
    assert!(read_document(&empty).is_err());

    let declared = package(
        directory.path(),
        &[(
            "content.xml",
            "<?xml version=\"1.0\"?><!DOCTYPE x [<!ENTITY a \"b\">]><x>&a;</x>",
        )],
    );
    assert!(read_document(&declared).is_err());

    let unclosed = package(
        directory.path(),
        &[(
            "content.xml",
            &content("<text:p>Open")
                .replace("</office:text></office:body></office:document-content>", ""),
        )],
    );
    assert!(read_document(&unclosed).is_err());
}
