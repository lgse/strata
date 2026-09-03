// SPDX-License-Identifier: GPL-3.0-or-later

use std::time::Duration;

use super::{
    Document, DocumentBlock, DocumentKind, DocumentListChildKind, DocumentSpan, DocumentSpanStyle,
    DocumentTableCell, DocumentUnitKind, LayoutBudget, ParseLimits, append_styled_text,
    document_kind, layout_document, parse_document, parse_document_with_limits, parse_markdown,
};
use crate::sandbox::Cancellation;

#[test]
fn markdown_renders_supported_formatting_as_blocks() {
    assert_eq!(
        parse_markdown("## Changes\n\n- **Fast** and `safe`\n- [Details](https://example.test)")
            .blocks,
        vec![
            DocumentBlock::Heading {
                level: 2,
                markup: "Changes".to_owned(),
            },
            DocumentBlock::ListItem {
                marker: "•".to_owned(),
                depth: 0,
                markup: "<b>Fast</b> and <tt>safe</tt>".to_owned(),
            },
            DocumentBlock::ListItem {
                marker: "•".to_owned(),
                depth: 0,
                markup: "<a href=\"https://example.test\">Details</a>".to_owned(),
            },
        ]
    );
}

#[test]
fn multiline_formatting_and_code_stay_in_balanced_blocks() {
    assert_eq!(
        parse_markdown("**first\nsecond**\n\n```text\none < two\n```").blocks,
        vec![
            DocumentBlock::Paragraph("<b>first\nsecond</b>".to_owned()),
            DocumentBlock::Code {
                markup: "one &lt; two\n".to_owned(),
                language: None,
            },
        ]
    );
}

#[test]
fn nested_and_ordered_lists_keep_markers_and_depth() {
    assert_eq!(
        parse_markdown("3. outer\n   - inner\n4. next").blocks,
        vec![
            DocumentBlock::ListItem {
                marker: "3.".to_owned(),
                depth: 0,
                markup: "outer".to_owned(),
            },
            DocumentBlock::ListItem {
                marker: "•".to_owned(),
                depth: 1,
                markup: "inner".to_owned(),
            },
            DocumentBlock::ListItem {
                marker: "4.".to_owned(),
                depth: 0,
                markup: "next".to_owned(),
            },
        ]
    );
}

#[test]
fn markdown_keeps_html_inert_and_does_not_retain_image_urls() {
    let blocks = parse_markdown(
        "<script>alert('no')</script>\n\n![tracking](https://example.test/pixel.png)",
    )
    .blocks;
    let debug = format!("{blocks:?}");
    assert!(!debug.contains("<script>"));
    assert!(debug.contains("&lt;script&gt;"));
    assert!(!debug.contains("pixel.png"));
    assert!(debug.contains("[Image: tracking]"));
}

#[test]
fn markdown_does_not_activate_non_web_links() {
    assert_eq!(
        parse_markdown("[Run](javascript:alert('no'))").blocks,
        vec![DocumentBlock::Paragraph("<u>Run</u>".to_owned())]
    );
}

#[test]
fn malformed_markdown_and_entities_remain_inert() {
    let blocks = parse_markdown("<broken & **unfinished").blocks;
    let debug = format!("{blocks:?}");
    assert!(debug.contains("&lt;broken &amp;"));
    assert!(!debug.contains("<broken"));
}

#[test]
fn empty_markdown_has_no_blocks() {
    assert!(parse_markdown("  \n").blocks.is_empty());
}

#[test]
fn classifies_supported_local_document_mimes_and_extensions_case_insensitively() {
    for mime in [
        "text/markdown",
        "text/x-markdown",
        "TEXT/HTML",
        "application/xhtml+xml",
    ] {
        assert!(document_kind(mime, std::ffi::OsStr::new("file.txt"), true).is_some());
    }
    for name in [
        "README.md",
        "README.MARKDOWN",
        "a.mdown",
        "a.mkd",
        "a.mkdn",
        "a.mdwn",
        "a.HTML",
        "a.htm",
        "a.xhtml",
    ] {
        assert!(document_kind("text/plain", std::ffi::OsStr::new(name), true).is_some());
    }
    assert_eq!(
        document_kind("text/plain", std::ffi::OsStr::new("notes.md"), false),
        None
    );
    assert_eq!(
        document_kind("text/plain", std::ffi::OsStr::new("notes.txt"), true),
        None
    );
}

#[test]
fn markdown_supports_quotes_tables_tasks_strikethrough_and_safe_images() {
    let parsed = parse_document(
        DocumentKind::Markdown,
        "> quoted\n\n- [x] ~~done~~\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n![alt](file:///secret)",
        &Cancellation::default(),
    )
    .expect("supported Markdown should render");
    assert!(matches!(parsed.document.blocks[0], DocumentBlock::Quote(_)));
    assert!(format!("{:?}", parsed.document.blocks).contains("☑ <s>done</s>"));
    assert!(parsed.document.blocks.iter().any(|block| matches!(
        block,
        DocumentBlock::TableRow { cells }
            if cells == &[
                DocumentTableCell { header: true, markup: "A".to_owned() },
                DocumentTableCell { header: true, markup: "B".to_owned() },
            ]
    )));
    let debug = format!("{:?}", parsed.document.blocks);
    assert!(debug.contains("[Image: alt]"));
    assert!(!debug.contains("file:///secret"));
}

#[test]
fn markdown_raw_html_is_inert_and_warned() {
    let parsed = parse_document(
        DocumentKind::Markdown,
        "before <b onclick=\"run()\">after</b>",
        &Cancellation::default(),
    )
    .expect("raw HTML should remain inert text");
    let debug = format!("{:?}", parsed.document.blocks);
    assert!(debug.contains("&lt;b onclick=&quot;run()&quot;&gt;"));
    assert_eq!(parsed.warnings.len(), 1);
}

#[test]
fn html_supports_semantic_blocks_formatting_entities_lists_tables_and_breaks() {
    let parsed = parse_document(
        DocumentKind::Html,
        "<!doctype html><html><body><h2>Title &amp; more</h2><p><strong>Bold</strong><br><em>line</em></p><ol><li>one</li><li>two</li></ol><blockquote>quote</blockquote><pre>x &lt; y</pre><hr><table><thead><tr><th>A</th></tr></thead><tbody><tr><td>B</td></tr></tbody></table></body></html>",
        &Cancellation::default(),
    )
    .expect("supported HTML should render");
    let debug = format!("{:?}", parsed.document.blocks);
    assert!(debug.contains("Title &amp; more"));
    assert!(debug.contains("<b>Bold</b>\\n<i>line</i>"));
    assert!(debug.contains("marker: \"1.\""));
    assert!(debug.contains("Quote(\"quote\")"));
    assert!(parsed.document.blocks.iter().any(|block| matches!(
        block,
        DocumentBlock::Code { markup, .. } if markup == "x &lt; y"
    )));
    assert!(debug.contains("DocumentTableCell { header: true"));
    assert!(debug.contains("DocumentTableCell { header: false"));
    assert!(parsed.warnings.is_empty());
}

#[test]
fn html_collapses_flow_whitespace_and_preserves_preformatted_text() {
    let parsed = parse_document(
        DocumentKind::Html,
        "<p>\n  This paragraph has <strong>bold</strong>,\n  <em>emphasis</em>, a line break<br>\n  and <code>inline code</code>.\n</p><pre>  first\n    second\n</pre>",
        &Cancellation::default(),
    )
    .expect("flow whitespace should render like HTML");

    assert_eq!(
        parsed.document.blocks,
        vec![
            DocumentBlock::Paragraph(
                "This paragraph has <b>bold</b>, <i>emphasis</i>, a line break\nand <tt>inline code</tt>."
                    .to_owned(),
            ),
            DocumentBlock::Code {
                markup: "  first\n    second\n".to_owned(),
                language: None,
            },
        ]
    );
}

#[test]
fn html_omits_active_embedded_and_resource_content_without_retaining_urls() {
    let parsed = parse_document(
        DocumentKind::Html,
        "<html><body><p onclick=\"run()\"><a href=\"https://example.test\">safe</a> <a href=\"javascript:run()\">unsafe</a></p><script><img src=\"https://tracker.test/a\"></script><form action=\"https://submit.test\"><input></form><img src=\"file:///secret\"><video src=\"https://media.test/a\"></video><custom>kept</custom></body></html>",
        &Cancellation::default(),
    )
    .expect("safe mixed content should render");
    let debug = format!("{:?}", parsed.document.blocks);
    assert!(debug.contains("href=\\\"https://example.test\\\""));
    assert!(debug.contains("<u>unsafe</u>"));
    assert!(debug.contains("kept"));
    for omitted in [
        "javascript:",
        "tracker.test",
        "submit.test",
        "file:///",
        "media.test",
        "onclick",
    ] {
        assert!(!debug.contains(omitted), "must not retain {omitted}");
    }
    assert_eq!(parsed.warnings.len(), 1);
}

#[test]
fn html_contentless_and_malformed_documents_fall_back_to_source() {
    let contentless = parse_document(
        DocumentKind::Html,
        "<html><body><script>alert(1)</script></body></html>",
        &Cancellation::default(),
    )
    .expect_err("active-only HTML has no trustworthy rendering");
    assert!(contentless.contains("no supported document content"));

    let empty_tables = "<table><tr></tr></table>".repeat(513);
    let contentless = parse_document(DocumentKind::Html, &empty_tables, &Cancellation::default())
        .expect_err("cell-less table rows must not create GTK grids");
    assert!(contentless.contains("no supported document content"));

    let malformed = parse_document(
        DocumentKind::Html,
        "<p>text</span>",
        &Cancellation::default(),
    )
    .expect_err("an unmatched end tag should fall back");
    assert!(malformed.contains("malformed"));
}

#[test]
fn html_links_remain_balanced_when_they_contain_blocks() {
    let blocks = parse_document(
        DocumentKind::Html,
        "<a href=\"https://e\">lead<h2>Title</h2>tail</a>",
        &Cancellation::default(),
    )
    .expect("anchors may contain flow content")
    .document
    .blocks;
    assert_eq!(
        blocks,
        vec![
            DocumentBlock::Paragraph("<a href=\"https://e\">lead</a>".to_owned()),
            DocumentBlock::Heading {
                level: 2,
                markup: "<a href=\"https://e\">Title</a>".to_owned(),
            },
            DocumentBlock::Paragraph("<a href=\"https://e\">tail</a>".to_owned()),
        ]
    );
}

#[test]
fn html_never_publishes_unbalanced_pango_markup() {
    assert!(
        parse_document(
            DocumentKind::Html,
            "<em>lead<h2>Title</h2>tail</em>",
            &Cancellation::default(),
        )
        .expect_err("invalid phrasing-content nesting must fall back safely")
        .contains("unsupported document structure")
    );
}

#[test]
fn html_accepts_optional_end_tags_and_omitted_document_closures() {
    let parsed = parse_document(
        DocumentKind::Html,
        "<p>Hello, <ul><li>one<li>two</ul>",
        &Cancellation::default(),
    )
    .expect("optional paragraph and list-item end tags are valid");
    assert_eq!(
        parsed.document.blocks,
        vec![
            DocumentBlock::Paragraph("Hello,".to_owned()),
            DocumentBlock::ListItem {
                marker: "•".to_owned(),
                depth: 0,
                markup: "one".to_owned(),
            },
            DocumentBlock::ListItem {
                marker: "•".to_owned(),
                depth: 0,
                markup: "two".to_owned(),
            },
        ]
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
    assert_eq!(
        parse_document(
            DocumentKind::Html,
            "<blockquote><p>quote</p></blockquote>",
            &Cancellation::default(),
        )
        .expect("normal blockquote markup should render")
        .document
        .blocks,
        vec![DocumentBlock::Quote("quote".to_owned())]
    );
}

#[test]
fn html_paragraphs_inside_list_items_keep_list_semantics() {
    assert_eq!(
        parse_document(
            DocumentKind::Html,
            "<ul><li><p>one</p></li></ul>",
            &Cancellation::default(),
        )
        .expect("paragraphs are valid list-item content")
        .document
        .blocks,
        vec![DocumentBlock::ListItem {
            marker: "•".to_owned(),
            depth: 0,
            markup: "one".to_owned(),
        }]
    );
}

#[test]
fn html_restores_parent_list_items_after_nested_lists() {
    assert_eq!(
        parse_document(
            DocumentKind::Html,
            "<ul><li><p>outer</p><ul><li>inner</li></ul><p>tail</p></li></ul>",
            &Cancellation::default(),
        )
        .expect("nested lists should resume their parent item")
        .document
        .blocks,
        vec![
            DocumentBlock::ListItem {
                marker: "•".to_owned(),
                depth: 0,
                markup: "outer".to_owned(),
            },
            DocumentBlock::ListItem {
                marker: "•".to_owned(),
                depth: 1,
                markup: "inner".to_owned(),
            },
            DocumentBlock::ListChild {
                depth: 0,
                kind: DocumentListChildKind::Paragraph,
                markup: "tail".to_owned(),
            },
        ]
    );
}

#[test]
fn markdown_restores_parent_list_items_after_nested_lists() {
    assert_eq!(
        parse_document(
            DocumentKind::Markdown,
            "- outer\n  - inner\n\n  tail",
            &Cancellation::default(),
        )
        .expect("nested Markdown lists should resume their parent item")
        .document
        .blocks,
        vec![
            DocumentBlock::ListItem {
                marker: "•".to_owned(),
                depth: 0,
                markup: "outer".to_owned(),
            },
            DocumentBlock::ListItem {
                marker: "•".to_owned(),
                depth: 1,
                markup: "inner".to_owned(),
            },
            DocumentBlock::ListChild {
                depth: 0,
                kind: DocumentListChildKind::Paragraph,
                markup: "tail".to_owned(),
            },
        ]
    );
}

#[test]
fn list_item_paragraphs_remain_separate_children() {
    let expected = vec![
        DocumentBlock::ListItem {
            marker: "•".to_owned(),
            depth: 0,
            markup: "first".to_owned(),
        },
        DocumentBlock::ListChild {
            depth: 0,
            kind: DocumentListChildKind::Paragraph,
            markup: "second".to_owned(),
        },
    ];

    assert_eq!(
        parse_document(
            DocumentKind::Markdown,
            "- first\n\n  second",
            &Cancellation::default(),
        )
        .expect("Markdown list paragraphs should render separately")
        .document
        .blocks,
        expected
    );
    assert_eq!(
        parse_document(
            DocumentKind::Html,
            "<ul><li><p>first</p><p>second</p></li></ul>",
            &Cancellation::default(),
        )
        .expect("HTML list paragraphs should render separately")
        .document
        .blocks,
        expected
    );
}

#[test]
fn markdown_keeps_semantic_block_children_inside_list_items() {
    assert_eq!(
        parse_document(
            DocumentKind::Markdown,
            "- outer\n\n  ## heading\n\n  > quote\n\n  ```rust\n  code\n  ```",
            &Cancellation::default(),
        )
        .expect("Markdown block children should retain their list context")
        .document
        .blocks,
        vec![
            DocumentBlock::ListItem {
                marker: "•".to_owned(),
                depth: 0,
                markup: "outer".to_owned(),
            },
            DocumentBlock::ListChild {
                depth: 0,
                kind: DocumentListChildKind::Heading(2),
                markup: "heading".to_owned(),
            },
            DocumentBlock::ListChild {
                depth: 0,
                kind: DocumentListChildKind::Quote,
                markup: "quote".to_owned(),
            },
            DocumentBlock::ListChild {
                depth: 0,
                kind: DocumentListChildKind::Code(Some("rust")),
                markup: "code\n".to_owned(),
            },
        ]
    );
}

#[test]
fn html_keeps_semantic_block_children_inside_list_items() {
    assert_eq!(
        parse_document(
            DocumentKind::Html,
            "<ul><li>outer<h2>heading</h2><blockquote><p>quote</p></blockquote><pre><code class=\"language-js\">code</code></pre></li></ul>",
            &Cancellation::default(),
        )
        .expect("HTML block children should retain their list context")
        .document
        .blocks,
        vec![
            DocumentBlock::ListItem {
                marker: "•".to_owned(),
                depth: 0,
                markup: "outer".to_owned(),
            },
            DocumentBlock::ListChild {
                depth: 0,
                kind: DocumentListChildKind::Heading(2),
                markup: "heading".to_owned(),
            },
            DocumentBlock::ListChild {
                depth: 0,
                kind: DocumentListChildKind::Quote,
                markup: "quote".to_owned(),
            },
            DocumentBlock::ListChild {
                depth: 0,
                kind: DocumentListChildKind::Code(Some("js")),
                markup: "code".to_owned(),
            },
        ]
    );
}

#[test]
fn list_blockquote_paragraphs_keep_both_semantics() {
    let expected = vec![
        DocumentBlock::ListItem {
            marker: "•".to_owned(),
            depth: 0,
            markup: String::new(),
        },
        DocumentBlock::ListChild {
            depth: 0,
            kind: DocumentListChildKind::Quote,
            markup: "first".to_owned(),
        },
        DocumentBlock::ListChild {
            depth: 0,
            kind: DocumentListChildKind::Quote,
            markup: "second".to_owned(),
        },
    ];

    assert_eq!(
        parse_document(
            DocumentKind::Markdown,
            "- > first\n  >\n  > second",
            &Cancellation::default(),
        )
        .expect("Markdown quote paragraphs should remain inside their list item")
        .document
        .blocks,
        expected
    );
    assert_eq!(
        parse_document(
            DocumentKind::Html,
            "<ul><li><blockquote><p>first</p><p>second</p></blockquote></li></ul>",
            &Cancellation::default(),
        )
        .expect("HTML quote paragraphs should remain inside their list item")
        .document
        .blocks,
        expected
    );
}

#[test]
fn list_rules_and_tables_do_not_orphan_trailing_content() {
    let expected = vec![
        DocumentBlock::ListItem {
            marker: "•".to_owned(),
            depth: 0,
            markup: "outer".to_owned(),
        },
        DocumentBlock::ListRule { depth: 0 },
        DocumentBlock::ListTableRow {
            depth: 0,
            cells: vec![DocumentTableCell {
                header: true,
                markup: "A".to_owned(),
            }],
        },
        DocumentBlock::ListTableRow {
            depth: 0,
            cells: vec![DocumentTableCell {
                header: false,
                markup: "B".to_owned(),
            }],
        },
        DocumentBlock::ListChild {
            depth: 0,
            kind: DocumentListChildKind::Paragraph,
            markup: "tail".to_owned(),
        },
    ];

    assert_eq!(
        parse_document(
            DocumentKind::Markdown,
            "- outer\n\n  ---\n\n  | A |\n  |---|\n  | B |\n\n  tail",
            &Cancellation::default(),
        )
        .expect("Markdown rules and tables should remain inside their list item")
        .document
        .blocks,
        expected
    );
    assert_eq!(
        parse_document(
            DocumentKind::Html,
            "<ul><li>outer<hr><table><tr><th>A</th></tr><tr><td>B</td></tr></table><p>tail</p></li></ul>",
            &Cancellation::default(),
        )
        .expect("HTML rules and tables should remain inside their list item")
        .document
        .blocks,
        expected
    );
}

#[test]
fn separate_lists_and_tables_keep_container_boundaries() {
    let markdown = parse_document(
        DocumentKind::Markdown,
        "- one\n\n1. two",
        &Cancellation::default(),
    )
    .expect("separate Markdown lists should render");
    assert!(matches!(
        markdown.document.blocks.as_slice(),
        [
            DocumentBlock::ListItem { .. },
            DocumentBlock::ContainerBoundary,
            DocumentBlock::ListItem { .. }
        ]
    ));

    let html_lists = parse_document(
        DocumentKind::Html,
        "<ul><li>one</ul><ol><li>two</ol>",
        &Cancellation::default(),
    )
    .expect("separate HTML lists should render");
    assert!(matches!(
        html_lists.document.blocks.as_slice(),
        [
            DocumentBlock::ListItem { .. },
            DocumentBlock::ContainerBoundary,
            DocumentBlock::ListItem { .. }
        ]
    ));

    let markdown_rule = parse_document(
        DocumentKind::Markdown,
        "- first\n\n  ---\n\n1. second",
        &Cancellation::default(),
    )
    .expect("a rule must not merge separate Markdown lists");
    assert!(matches!(
        markdown_rule.document.blocks.as_slice(),
        [
            DocumentBlock::ListItem { .. },
            DocumentBlock::ListRule { .. },
            DocumentBlock::ContainerBoundary,
            DocumentBlock::ListItem { .. }
        ]
    ));

    let html_rule = parse_document(
        DocumentKind::Html,
        "<ul><li>first<hr></li></ul><ol><li>second</li></ol>",
        &Cancellation::default(),
    )
    .expect("a rule must not merge separate HTML lists");
    assert!(matches!(
        html_rule.document.blocks.as_slice(),
        [
            DocumentBlock::ListItem { .. },
            DocumentBlock::ListRule { .. },
            DocumentBlock::ContainerBoundary,
            DocumentBlock::ListItem { .. }
        ]
    ));

    let markdown_table = parse_document(
        DocumentKind::Markdown,
        "- first\n\n  | A |\n  | - |\n  | B |\n\n1. second",
        &Cancellation::default(),
    )
    .expect("a table must not merge separate Markdown lists");
    assert!(matches!(
        markdown_table.document.blocks.as_slice(),
        [
            DocumentBlock::ListItem { .. },
            DocumentBlock::ListTableRow { .. },
            DocumentBlock::ListTableRow { .. },
            DocumentBlock::ContainerBoundary,
            DocumentBlock::ListItem { .. }
        ]
    ));

    let html_table = parse_document(
        DocumentKind::Html,
        "<ul><li>first<table><tr><td>A</td></tr></table></li></ul><ol><li>second</li></ol>",
        &Cancellation::default(),
    )
    .expect("a table must not merge separate HTML lists");
    assert!(matches!(
        html_table.document.blocks.as_slice(),
        [
            DocumentBlock::ListItem { .. },
            DocumentBlock::ListTableRow { .. },
            DocumentBlock::ContainerBoundary,
            DocumentBlock::ListItem { .. }
        ]
    ));

    let markdown_tables = parse_document(
        DocumentKind::Markdown,
        "| A |\n| - |\n| 1 |\n\n| B |\n| - |\n| 2 |",
        &Cancellation::default(),
    )
    .expect("adjacent Markdown tables should render");
    assert!(matches!(
        markdown_tables.document.blocks.as_slice(),
        [
            DocumentBlock::TableRow { .. },
            DocumentBlock::TableRow { .. },
            DocumentBlock::ContainerBoundary,
            DocumentBlock::TableRow { .. },
            DocumentBlock::TableRow { .. }
        ]
    ));

    let html = parse_document(
        DocumentKind::Html,
        "<table><tr><td>A</table><table><tr><td>B</table>",
        &Cancellation::default(),
    )
    .expect("adjacent HTML tables should render");
    assert!(matches!(
        html.document.blocks.as_slice(),
        [
            DocumentBlock::TableRow { .. },
            DocumentBlock::ContainerBoundary,
            DocumentBlock::TableRow { .. }
        ]
    ));
}

#[test]
fn html_markup_limit_applies_while_links_are_reemitted() {
    let limits = ParseLimits {
        events: 2,
        markup: 128,
        ..ParseLimits::default()
    };
    let html = format!(
        "<a href=\"https://example.test/{}\">{}</a>",
        "x".repeat(64),
        "<p>x</p>".repeat(100)
    );
    assert!(
        parse_document_with_limits(DocumentKind::Html, &html, &Cancellation::default(), limits,)
            .expect_err("repeated link markup must stop at the output limit")
            .contains("markup limit")
    );
}

#[test]
fn html_closes_compact_table_rows_and_sections_without_losing_cells() {
    assert_eq!(
        parse_document(
            DocumentKind::Html,
            "<table><thead><tr><th>A<tbody><tr><td>B</table>",
            &Cancellation::default(),
        )
        .expect("optional table end tags are valid")
        .document
        .blocks,
        vec![
            DocumentBlock::TableRow {
                cells: vec![DocumentTableCell {
                    header: true,
                    markup: "A".to_owned(),
                }],
            },
            DocumentBlock::TableRow {
                cells: vec![DocumentTableCell {
                    header: false,
                    markup: "B".to_owned(),
                }],
            },
        ]
    );
}

#[test]
fn html_preserves_header_semantics_per_table_cell() {
    assert_eq!(
        parse_document(
            DocumentKind::Html,
            "<table><tr><th>Name</th><td>Alice</td></tr></table>",
            &Cancellation::default(),
        )
        .expect("th and td should retain distinct semantics")
        .document
        .blocks,
        vec![DocumentBlock::TableRow {
            cells: vec![
                DocumentTableCell {
                    header: true,
                    markup: "Name".to_owned(),
                },
                DocumentTableCell {
                    header: false,
                    markup: "Alice".to_owned(),
                },
            ],
        }]
    );
}

#[test]
fn html_closes_nested_content_before_an_optional_table_cell_end() {
    assert_eq!(
        parse_document(
            DocumentKind::Html,
            "<table><tr><td><p>A<td>B</table>",
            &Cancellation::default(),
        )
        .expect("a new cell should close nested content in the previous cell")
        .document
        .blocks,
        vec![DocumentBlock::TableRow {
            cells: vec![
                DocumentTableCell {
                    header: false,
                    markup: "A".to_owned(),
                },
                DocumentTableCell {
                    header: false,
                    markup: "B".to_owned(),
                },
            ],
        }]
    );
}

#[test]
fn html_pre_code_is_supported_without_an_omission_warning() {
    let parsed = parse_document(
        DocumentKind::Html,
        "<pre><code>x &lt; y</code></pre>",
        &Cancellation::default(),
    )
    .expect("canonical pre/code should render");
    assert_eq!(
        parsed.document.blocks,
        vec![DocumentBlock::Code {
            markup: "x &lt; y".to_owned(),
            language: None,
        }]
    );
    assert!(parsed.warnings.is_empty());
}

#[test]
fn code_language_hints_are_bounded_and_preserved_for_layout() {
    let markdown = parse_document(
        DocumentKind::Markdown,
        "```rs title=ignored\nfn main() {}\n```\n\n```made-up\nplain\n```",
        &Cancellation::default(),
    )
    .expect("fenced code should render");
    assert!(matches!(
        &markdown.document.blocks[0],
        DocumentBlock::Code {
            language: Some("rust"),
            ..
        }
    ));
    assert!(matches!(
        &markdown.document.blocks[1],
        DocumentBlock::Code { language: None, .. }
    ));
    let layout = layout_document(markdown.document, &Cancellation::default())
        .expect("code layout should succeed");
    assert!(matches!(
        layout.units[0].kind,
        DocumentUnitKind::Code {
            language: Some("rust"),
            ..
        }
    ));

    let html = parse_document(
        DocumentKind::Html,
        "<pre><code class=\"language-js\">const safe = true;</code></pre>",
        &Cancellation::default(),
    )
    .expect("HTML code language class should render");
    assert!(html.warnings.is_empty());
    assert!(matches!(
        &html.document.blocks[0],
        DocumentBlock::Code {
            language: Some("js"),
            ..
        }
    ));
}

#[test]
fn parser_enforces_input_event_depth_table_markup_time_and_cancellation_limits() {
    let cancellation = Cancellation::default();
    assert!(
        parse_document(
            DocumentKind::Markdown,
            &"x".repeat(1024 * 1024 + 1),
            &cancellation
        )
        .expect_err("oversized input")
        .contains("1 MB")
    );

    let many_events = "x\n\n".repeat(7_000);
    assert!(
        parse_document(DocumentKind::Markdown, &many_events, &cancellation)
            .expect_err("event-heavy input")
            .contains("parser-event")
    );

    let deep = format!("{}x{}", "<div>".repeat(33), "</div>".repeat(33));
    assert!(
        parse_document(DocumentKind::Html, &deep, &cancellation)
            .expect_err("deep input")
            .contains("nesting-depth")
    );

    let many_blocks = "x\n\n".repeat(513);
    let parsed = parse_document(DocumentKind::Markdown, &many_blocks, &cancellation)
        .expect("ordinary blocks are virtualized");
    assert_eq!(
        layout_document(parsed.document, &cancellation)
            .expect("many-block layout")
            .units
            .len(),
        513
    );

    let many_tables = "<table><tr><td>x</td></tr></table>".repeat(257);
    assert!(
        parse_document(DocumentKind::Html, &many_tables, &cancellation).is_ok(),
        "separate tables are virtualized independently"
    );

    let table_at_limit = format!(
        "<table><tr>{}</tr></table>",
        "<td>x</td>".repeat(super::DOCUMENT_TABLE_CELL_LIMIT)
    );
    assert!(parse_document(DocumentKind::Html, &table_at_limit, &cancellation).is_ok());
    let table_over_limit = format!(
        "<table><tr>{}</tr></table>",
        "<td>x</td>".repeat(super::DOCUMENT_TABLE_CELL_LIMIT + 1)
    );
    assert!(
        parse_document(DocumentKind::Html, &table_over_limit, &cancellation)
            .expect_err("one atomic table must keep a resident cell bound")
            .contains("one table")
    );

    let markup = "&".repeat(1024 * 1024);
    assert!(
        parse_document(DocumentKind::Markdown, &markup, &cancellation)
            .expect_err("escaped markup expansion")
            .contains("markup")
    );

    let maximum_line = "x".repeat(super::DOCUMENT_INPUT_LIMIT);
    let parsed = parse_document(DocumentKind::Markdown, &maximum_line, &cancellation)
        .expect("a maximum-size logical line is virtualized");
    let layout = layout_document(parsed.document, &cancellation).expect("maximum-line layout");
    assert_eq!(layout.units.len(), 512);
    assert!(
        layout
            .units
            .iter()
            .all(|unit| unit.text.len() <= super::DOCUMENT_UNIT_LINE_TARGET)
    );
    assert!(layout.units.iter().all(|unit| !unit.wrap));

    let one_mib_long_lines = format!("{}\n", "x".repeat(64 * 1024 - 1)).repeat(16);
    assert_eq!(one_mib_long_lines.len(), 1024 * 1024);
    let parsed = parse_document(DocumentKind::Markdown, &one_mib_long_lines, &cancellation)
        .expect("one MiB split into bounded logical lines");
    let layout = layout_document(parsed.document, &cancellation).expect("bounded layout");
    assert!(
        layout
            .units
            .iter()
            .all(|unit| unit.text.len() <= super::DOCUMENT_UNIT_LINE_TARGET)
    );
    assert_eq!(
        layout
            .units
            .iter()
            .map(|unit| unit.copy_text.as_str())
            .collect::<String>(),
        one_mib_long_lines
    );

    let monolithic_paragraph = format!("{}\n", "x".repeat(4 * 1024)).repeat(17);
    let parsed = parse_document(DocumentKind::Markdown, &monolithic_paragraph, &cancellation)
        .expect("large multiline paragraphs are virtualized");
    assert!(layout_document(parsed.document, &cancellation).is_ok());

    let large_code = format!("```text\n{}\n```", "x\n".repeat(40 * 1024));
    assert!(
        parse_document(DocumentKind::Markdown, &large_code, &cancellation).is_ok(),
        "large code blocks are split into virtual units"
    );

    let zero_time = ParseLimits {
        time: Duration::ZERO,
        ..ParseLimits::default()
    };
    assert!(
        parse_document_with_limits(DocumentKind::Markdown, "text", &cancellation, zero_time)
            .expect_err("zero time budget")
            .contains("500 ms")
    );

    cancellation.cancel();
    assert!(
        parse_document(DocumentKind::Markdown, "text", &cancellation)
            .expect_err("cancelled parse")
            .contains("cancelled")
    );
}

#[test]
fn document_parsers_reject_nul_bytes_without_panicking() {
    for (kind, source) in [
        (DocumentKind::Markdown, "before\0after"),
        (DocumentKind::Html, "<p>before\0after</p>"),
    ] {
        assert!(
            parse_document(kind, source, &Cancellation::default())
                .expect_err("NUL bytes must not reach GLib markup escaping")
                .contains("NUL")
        );
    }

    assert_eq!(
        parse_markdown("before\0after").blocks,
        vec![DocumentBlock::Paragraph("before�after".to_owned())]
    );
}

#[test]
fn pre_cancelled_html_stops_before_tokenization() {
    let html = format!("<div {}>", "data-value='x' ".repeat(32_000));
    let cancellation = Cancellation::default();
    cancellation.cancel();

    assert!(
        parse_document(DocumentKind::Html, &html, &cancellation)
            .expect_err("pre-cancelled HTML must stop before requesting a token")
            .contains("cancelled")
    );
}

#[test]
fn document_layout_decodes_markup_and_bounds_pathological_lines() {
    let uri = "https://example.test/large";
    let markup = format!(
        "<b>first</b>\n<a href=\"{uri}\">{}</a>\nlast",
        "x".repeat(40 * 1024)
    );
    let layout = layout_document(
        Document {
            blocks: vec![DocumentBlock::Paragraph(markup)],
        },
        &Cancellation::default(),
    )
    .expect("bounded layout");

    assert!(layout.units.len() > 3);
    assert_eq!(layout.units[0].text, "first");
    assert_eq!(layout.units.last().expect("last unit").text, "last");
    assert!(layout.units[0].wrap);
    assert!(layout.units.last().expect("last unit").wrap);
    assert!(
        layout.units[1..layout.units.len() - 1]
            .iter()
            .all(|unit| !unit.wrap)
    );
    assert!(
        layout
            .units
            .iter()
            .all(|unit| unit.text.len() <= super::DOCUMENT_UNIT_LINE_TARGET)
    );
    assert_eq!(
        layout
            .units
            .iter()
            .map(|unit| unit.copy_text.as_str())
            .collect::<String>(),
        format!("first\n{}\nlast\n", "x".repeat(40 * 1024))
    );
    assert!(
        layout.units[0]
            .spans
            .iter()
            .any(|span| span.style == DocumentSpanStyle::Bold)
    );
    assert!(layout.units.iter().any(|unit| {
        unit.spans.iter().any(
            |span| matches!(&span.style, DocumentSpanStyle::Link(link) if link.as_ref() == uri),
        )
    }));
}

#[test]
fn redundant_nested_styles_emit_one_span() {
    let markup = format!("<b>{}</b>", "<b>x</b>".repeat(10_000));
    let layout = layout_document(
        Document {
            blocks: vec![DocumentBlock::Paragraph(markup)],
        },
        &Cancellation::default(),
    )
    .expect("redundant formatting should remain cheap to lay out");

    assert!(
        layout.units.len() > 1,
        "the long line should be virtualized"
    );
    assert!(layout.units.iter().all(|unit| unit.spans.len() == 1));
    assert!(
        layout
            .units
            .iter()
            .all(|unit| unit.spans[0].style == DocumentSpanStyle::Bold)
    );
}

#[test]
fn distinct_nested_html_links_are_inert_and_keep_spans_bounded() {
    let links = (0..31)
        .map(|index| format!("<a href=\"https://example.test/{index}\">"))
        .collect::<String>();
    let segments = 1_900;
    let html = format!(
        "{links}{}{}",
        "<b>x</b>".repeat(segments),
        "</a>".repeat(31)
    );
    let parsed = parse_document(DocumentKind::Html, &html, &Cancellation::default())
        .expect("nested anchors should retain their visible content");
    assert_eq!(
        parsed.warnings,
        ["Unsupported or active HTML content was omitted."]
    );
    let [DocumentBlock::Paragraph(markup)] = parsed.document.blocks.as_slice() else {
        panic!("expected one paragraph");
    };
    assert_eq!(markup.matches("<a href=").count(), 1);

    let layout = layout_document(parsed.document, &Cancellation::default())
        .expect("nested anchors must not amplify layout work");
    assert!(
        layout
            .units
            .iter()
            .map(|unit| unit.spans.len())
            .sum::<usize>()
            <= segments * 2
    );
    assert!(
        layout
            .units
            .iter()
            .flat_map(|unit| &unit.spans)
            .all(|span| {
                !matches!(
                    &span.style,
                    DocumentSpanStyle::Link(uri) if uri.as_ref() != "https://example.test/0"
                )
            })
    );
}

#[test]
fn document_layout_keeps_tables_atomic_and_copyable_as_tsv() {
    let layout = layout_document(
        Document {
            blocks: vec![
                DocumentBlock::TableRow {
                    cells: vec![
                        DocumentTableCell {
                            header: true,
                            markup: "<b>Name</b>".to_owned(),
                        },
                        DocumentTableCell {
                            header: true,
                            markup: "Value".to_owned(),
                        },
                    ],
                },
                DocumentBlock::TableRow {
                    cells: vec![
                        DocumentTableCell {
                            header: false,
                            markup: "Alice &amp; Bob".to_owned(),
                        },
                        DocumentTableCell {
                            header: false,
                            markup: "One".to_owned(),
                        },
                    ],
                },
            ],
        },
        &Cancellation::default(),
    )
    .expect("table layout");

    assert_eq!(layout.units.len(), 1);
    assert_eq!(layout.units[0].copy_text, "Name\tValue\nAlice & Bob\tOne\n");
    assert!(matches!(
        &layout.units[0].kind,
        DocumentUnitKind::Table { rows, .. } if rows.len() == 2
    ));
}

#[test]
fn cancelled_document_layout_stops_before_publishing_units() {
    let cancellation = Cancellation::default();
    cancellation.cancel();
    assert!(
        layout_document(
            Document {
                blocks: vec![DocumentBlock::Paragraph("text".to_owned())],
            },
            &cancellation,
        )
        .expect_err("cancelled layout")
        .contains("cancelled")
    );
}

#[test]
fn markup_decoding_rechecks_the_deadline_after_expensive_work() {
    let escaped = "x".repeat(1024 * 1024);
    let cancellation = Cancellation::default();
    let budget = LayoutBudget::new(&cancellation, Duration::from_millis(1));
    budget.check().expect("the budget starts live");
    let mut output = String::new();
    let mut spans = Vec::<DocumentSpan>::new();
    let mut characters = 0;

    assert!(
        append_styled_text(
            &mut output,
            &mut spans,
            &[],
            &escaped,
            &mut characters,
            &budget,
        )
        .expect_err("the inner markup work must recheck its deadline")
        .contains("500 ms")
    );
}
