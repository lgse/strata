// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn html_supports_semantic_blocks_formatting_entities_lists_tables_and_breaks() {
    let parsed = parse(
        DocumentKind::Html,
        "<!doctype html><html><body><h2>Title &amp; more</h2><p><strong>Bold</strong><br><em>line</em> <u>under</u></p><ol><li>one</li><li>two</li></ol><blockquote>quote</blockquote><pre>x &lt; y</pre><hr><table><thead><tr><th>A</th></tr></thead><tbody><tr><td>B</td></tr></tbody></table></body></html>",
        "supported HTML should render",
    );
    assert_debug_mentions(
        &debug_blocks(&parsed.document.blocks),
        &[
            "Title &amp; more",
            "<b>Bold</b>\\n<i>line</i> <u>under</u>",
            "marker: \"1.\"",
            "Quote(\"quote\")",
            "DocumentTableCell { header: true",
            "DocumentTableCell { header: false",
        ],
        &[],
    );
    assert!(parsed.document.blocks.iter().any(|block| matches!(
        block,
        DocumentBlock::Code { markup, .. } if markup == "x &lt; y"
    )));
    assert!(parsed.warnings.is_empty());
}

#[test]
fn html_collapses_flow_whitespace_and_preserves_preformatted_text() {
    assert_parses_to(
        DocumentKind::Html,
        "<p>\n  This paragraph has <strong>bold</strong>,\n  <em>emphasis</em>, a line break<br>\n  and <code>inline code</code>.\n</p><pre>  first\n    second\n</pre>",
        "flow whitespace should render like HTML",
        &[
            paragraph(
                "This paragraph has <b>bold</b>, <i>emphasis</i>, a line break\nand <tt>inline code</tt>.",
            ),
            DocumentBlock::Code {
                markup: "  first\n    second\n".to_owned(),
                language: None,
            },
        ],
    );
}

#[test]
fn html_omits_active_embedded_and_resource_content_without_retaining_urls() {
    let parsed = parse(
        DocumentKind::Html,
        "<html><body><p onclick=\"run()\"><a href=\"https://example.test\">safe</a> <a href=\"javascript:run()\">unsafe</a></p><script><img src=\"https://tracker.test/a\"></script><form action=\"https://submit.test\"><input></form><img src=\"file:///secret\"><video src=\"https://media.test/a\"></video><custom>kept</custom></body></html>",
        "safe mixed content should render",
    );
    assert_debug_mentions(
        &debug_blocks(&parsed.document.blocks),
        &["href=\\\"https://example.test\\\"", "<u>unsafe</u>", "kept"],
        &[
            "javascript:",
            "tracker.test",
            "submit.test",
            "file:///",
            "media.test",
            "onclick",
        ],
    );
    assert_eq!(parsed.warnings.len(), 1);
}

#[test]
fn html_contentless_and_malformed_documents_fall_back_to_source() {
    let contentless = parse_error(
        DocumentKind::Html,
        "<html><body><script>alert(1)</script></body></html>",
        "active-only HTML has no trustworthy rendering",
    );
    assert!(contentless.contains("no supported document content"));

    let empty_tables = "<table><tr></tr></table>".repeat(513);
    let contentless = parse_error(
        DocumentKind::Html,
        &empty_tables,
        "cell-less table rows must not create GTK grids",
    );
    assert!(contentless.contains("no supported document content"));

    let malformed = parse_error(
        DocumentKind::Html,
        "<p>text</span>",
        "an unmatched end tag should fall back",
    );
    assert!(malformed.contains("malformed"));
}

#[test]
fn html_links_remain_balanced_when_they_contain_blocks() {
    assert_parses_to(
        DocumentKind::Html,
        "<a href=\"https://e\">lead<h2>Title</h2>tail</a>",
        "anchors may contain flow content",
        &[
            paragraph("<a href=\"https://e\">lead</a>"),
            heading(2, "<a href=\"https://e\">Title</a>"),
            paragraph("<a href=\"https://e\">tail</a>"),
        ],
    );
}

#[test]
fn html_never_publishes_unbalanced_pango_markup() {
    assert!(
        parse_error(
            DocumentKind::Html,
            "<em>lead<h2>Title</h2>tail</em>",
            "invalid phrasing-content nesting must fall back safely",
        )
        .contains("unsupported document structure")
    );
}

#[test]
fn html_accepts_optional_end_tags_and_omitted_document_closures() {
    assert_parses_to(
        DocumentKind::Html,
        "<p>Hello, <ul><li>one<li>two</ul>",
        "optional paragraph and list-item end tags are valid",
        &[paragraph("Hello,"), bullet(0, "one"), bullet(0, "two")],
    );

    assert!(
        parse_document(
            DocumentKind::Html,
            "<html><body><p>open document",
            &Cancellation::default(),
        )
        .is_ok()
    );
}

#[test]
fn html_paragraphs_inside_blockquotes_keep_quote_semantics() {
    assert_parses_to(
        DocumentKind::Html,
        "<blockquote><p>quote</p></blockquote>",
        "normal blockquote markup should render",
        &[DocumentBlock::Quote("quote".to_owned())],
    );
}

#[test]
fn html_paragraphs_inside_list_items_keep_list_semantics() {
    assert_parses_to(
        DocumentKind::Html,
        "<ul><li><p>one</p></li></ul>",
        "paragraphs are valid list-item content",
        &[bullet(0, "one")],
    );
}

#[test]
fn html_closes_compact_table_rows_and_sections_without_losing_cells() {
    assert_parses_to(
        DocumentKind::Html,
        "<table><thead><tr><th>A<tbody><tr><td>B</table>",
        "optional table end tags are valid",
        &[
            table_row(vec![cell(true, "A")]),
            table_row(vec![cell(false, "B")]),
        ],
    );
}

#[test]
fn html_preserves_header_semantics_per_table_cell() {
    assert_parses_to(
        DocumentKind::Html,
        "<table><tr><th>Name</th><td>Alice</td></tr></table>",
        "th and td should retain distinct semantics",
        &[table_row(vec![cell(true, "Name"), cell(false, "Alice")])],
    );
}

#[test]
fn html_closes_nested_content_before_an_optional_table_cell_end() {
    assert_parses_to(
        DocumentKind::Html,
        "<table><tr><td><p>A<td>B</table>",
        "a new cell should close nested content in the previous cell",
        &[table_row(vec![cell(false, "A"), cell(false, "B")])],
    );
}

#[test]
fn html_pre_code_is_supported_without_an_omission_warning() {
    let parsed = parse(
        DocumentKind::Html,
        "<pre><code>x &lt; y</code></pre>",
        "canonical pre/code should render",
    );
    assert_eq!(
        parsed.document.blocks,
        vec![DocumentBlock::Code {
            markup: "x &lt; y".to_owned(),
            language: None,
        }]
    );
    assert!(parsed.warnings.is_empty());
}
