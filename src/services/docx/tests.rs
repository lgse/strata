// SPDX-License-Identifier: MIT

use std::path::PathBuf;

use super::*;
use crate::services::DocumentBlock;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/documents/report.docx")
}

#[test]
fn recognizes_word_documents_by_content_type_and_extension() {
    assert!(is_document(
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        OsStr::new("report.bin")
    ));
    assert!(is_document(
        "application/octet-stream",
        OsStr::new("a.DOCX")
    ));
    assert!(!is_document("application/msword", OsStr::new("legacy.doc")));
}

#[test]
fn reads_headings_styles_lists_and_tables_from_a_word_document() {
    let data = read_document(&fixture()).expect("document fixture");
    assert!(!data.truncated);
    let html = data.html;
    assert!(html.contains("<h1>Quarterly Report</h1>"), "{html}");
    assert!(html.contains("<h2>Regional summary</h2>"), "{html}");
    assert!(html.contains("<b>bold</b>"), "{html}");
    assert!(html.contains("<i>italic</i>"), "{html}");
    assert!(html.contains("<u>underlined</u>"), "{html}");
    assert!(html.contains("<s>struck</s>"), "{html}");
    assert!(html.contains("A &amp; B &lt; C."), "{html}");
    assert!(
        html.contains("<blockquote>Growth held steady.</blockquote>"),
        "{html}"
    );
    assert!(
        html.contains("<ul><li>First bullet</li><ul><li>Nested bullet</li>"),
        "{html}"
    );
    assert!(
        html.contains("<ol><li>First step</li><li>Second step</li></ol>"),
        "{html}"
    );
    assert!(
        html.contains("<tr><th>Region</th><th>Total</th></tr>"),
        "{html}"
    );
    assert!(
        html.contains("<tr><td>North</td><td>42</td></tr>"),
        "{html}"
    );
    assert!(html.contains("Line one<br>line two"), "{html}");
}

#[test]
fn derives_rendered_blocks_and_warns_when_output_was_truncated() {
    let data = read_document(&fixture()).expect("document fixture");
    let parsed = RichTextData {
        html: data.html,
        truncated: true,
    }
    .into_document(&Cancellation::default())
    .expect("rendered document");
    assert!(matches!(
        parsed.document.blocks.first(),
        Some(DocumentBlock::Heading { level: 1, .. })
    ));
    assert!(
        parsed
            .document
            .blocks
            .iter()
            .any(|block| matches!(block, DocumentBlock::TableRow { .. }))
    );
    assert!(
        parsed
            .document
            .blocks
            .iter()
            .any(|block| matches!(block, DocumentBlock::ListItem { .. }))
    );
    assert!(
        parsed
            .warnings
            .iter()
            .any(|warning| warning.contains("size budget"))
    );
}

#[test]
fn oversized_documents_stop_at_the_output_budget() {
    let filler = "x".repeat(64 * 1024);
    let mut docx = docx_rs::Docx::new();
    for _ in 0..24 {
        docx = docx.add_paragraph(
            docx_rs::Paragraph::new().add_run(docx_rs::Run::new().add_text(&filler)),
        );
    }
    let data = to_html(&docx);
    assert!(data.truncated);
    assert!(data.html.len() <= DOCUMENT_INPUT_LIMIT);
}

#[test]
fn rejects_unreadable_files_and_oversized_helper_output() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("not-a-document.docx");
    std::fs::write(&path, b"not a zip archive").expect("fixture write");
    assert!(read_document(&path).is_err());

    assert!(RichTextData::from_json(b"{}").is_err());
    let oversized = serde_json::to_vec(&RichTextData {
        html: "x".repeat(DOCUMENT_INPUT_LIMIT + 1),
        truncated: false,
    })
    .expect("serialized payload");
    assert!(RichTextData::from_json(&oversized).is_err());
}
