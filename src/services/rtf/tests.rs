// SPDX-License-Identifier: MIT

use super::*;
use crate::services::document::{DocumentBlock, DocumentKind, parse_document};

const SAMPLE: &str = r"{\rtf1\ansi\deff0{\fonttbl{\f0\fnil Helvetica;}}{\*\generator Strata 1.0;}
\pard Plain \b bold\b0  \i italic\i0  \ul underlined\ulnone  \strike struck\strike0  and A & B < C.\par
\pard \'93Smart\'94 quotes\u8212?dashes\line second line\par}";

#[test]
fn styled_runs_escape_text_and_drop_control_destinations() {
    let html = to_html(SAMPLE, &Cancellation::default()).expect("sample RTF");
    assert!(html.contains("<b>bold</b>"), "{html}");
    assert!(html.contains("<i>italic</i>"), "{html}");
    assert!(html.contains("<u>underlined</u>"), "{html}");
    assert!(html.contains("<s>struck</s>"), "{html}");
    assert!(html.contains("A &amp; B &lt; C."), "{html}");
    assert!(!html.contains("Helvetica"), "{html}");
    assert!(!html.contains("Strata 1.0"), "{html}");
}

#[test]
fn code_page_and_unicode_escapes_replace_their_legacy_fallbacks() {
    let html = to_html(SAMPLE, &Cancellation::default()).expect("sample RTF");
    assert!(
        html.contains("\u{201c}Smart\u{201d} quotes\u{2014}dashes"),
        "{html}"
    );
    assert!(!html.contains('?'), "{html}");
    let wrapped = to_html(r"{\rtf1\ansi \uc2 \u233??e\par}", &Cancellation::default())
        .expect("wide fallback");
    assert!(wrapped.contains("<p>\u{e9}e</p>"), "{wrapped}");
}

#[test]
fn rendered_documents_keep_one_block_per_paragraph() {
    let parsed = parse_document(DocumentKind::Rtf, SAMPLE, &Cancellation::default())
        .expect("rendered RTF document");
    assert_eq!(parsed.document.blocks.len(), 2);
    let DocumentBlock::Paragraph(markup) = &parsed.document.blocks[1] else {
        panic!("expected a paragraph, got {:?}", parsed.document.blocks[1]);
    };
    assert!(markup.contains("\nsecond line"), "{markup}");
}

#[test]
fn binary_payload_lengths_skip_whole_characters() {
    let html =
        to_html(r"{\rtf1\ansi\bin1 ékept\par}", &Cancellation::default()).expect("binary payload");
    assert_eq!(html, "<p>kept</p>");
    let wide = to_html(
        r"{\rtf1\ansi\bin9999 é\par after\par}",
        &Cancellation::default(),
    )
    .expect("payload longer than the document");
    assert_eq!(wide, "");
}

#[test]
fn cancellation_and_invalid_input_report_errors() {
    let cancellation = Cancellation::default();
    cancellation.cancel();
    assert!(to_html(SAMPLE, &cancellation).is_err());
    assert!(to_html("plain text, not RTF", &Cancellation::default()).is_err());
}
