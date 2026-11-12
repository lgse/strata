// SPDX-License-Identifier: MIT

use super::*;
use crate::services::document::{Document, DocumentSpanStyle, DocumentUnitKind, layout_document};

fn lay_out_blocks(blocks: Vec<DocumentBlock>, expectation: &str) -> DocumentLayout {
    layout_document(Document { blocks }, &Cancellation::default()).expect(expectation)
}

fn lay_out_paragraph(markup: String, expectation: &str) -> DocumentLayout {
    lay_out_blocks(vec![DocumentBlock::Paragraph(markup)], expectation)
}

fn has_style(unit: &DocumentUnit, matches_style: impl Fn(&DocumentSpanStyle) -> bool) -> bool {
    unit.spans.iter().any(|span| matches_style(&span.style))
}

#[test]
fn code_language_hints_are_bounded_and_preserved_for_layout() {
    let markdown = parse(
        DocumentKind::Markdown,
        "```rs title=ignored\nfn main() {}\n```\n\n```made-up\nplain\n```",
        "fenced code should render",
    );
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

    let html = parse(
        DocumentKind::Html,
        "<pre><code class=\"language-js\">const safe = true;</code></pre>",
        "HTML code language class should render",
    );
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
fn document_layout_decodes_markup_and_bounds_pathological_lines() {
    let uri = "https://example.test/large";
    let markup = format!(
        "<b>first</b>\n<a href=\"{uri}\">{}</a>\nlast",
        "x".repeat(40 * 1024)
    );
    let layout = lay_out_paragraph(markup, "bounded layout");

    assert!(layout.units.len() > 3);
    let first = &layout.units[0];
    let last = layout.units.last().expect("last unit");
    let middle = &layout.units[1..layout.units.len() - 1];
    assert_eq!((first.text.as_str(), first.wrap), ("first", true));
    assert_eq!((last.text.as_str(), last.wrap), ("last", true));
    assert!(middle.iter().all(|unit| !unit.wrap));

    let copied = copied_text(&layout);
    assert!(units_fit_the_line_target(&layout.units));
    assert_eq!(copied, format!("first\n{}\nlast\n", "x".repeat(40 * 1024)));
    assert!(has_style(first, |style| *style == DocumentSpanStyle::Bold));
    let links_uri = |style: &DocumentSpanStyle| matches!(style, DocumentSpanStyle::Link(link) if link.as_ref() == uri);
    assert!(layout.units.iter().any(|unit| has_style(unit, links_uri)));
}

#[test]
fn long_paragraphs_split_between_words() {
    for (markup, plain, separator) in [
        (
            "alpha bravo <b>uninterrupted</b> ".repeat(300),
            "alpha bravo uninterrupted ".repeat(300),
            ' ',
        ),
        (
            "日本語の文章\u{3000}".repeat(400),
            "日本語の文章\u{3000}".repeat(400),
            '\u{3000}',
        ),
    ] {
        let layout = lay_out_paragraph(markup, "long paragraph layout");

        assert!(layout.units.len() > 2, "{plain:.20}");
        assert!(units_fit_the_line_target(&layout.units));
        let (last, rest) = layout.units.split_last().expect("units");
        assert!(
            rest.iter().all(|unit| unit.text.ends_with(separator)),
            "{:?}",
            rest.iter()
                .map(|unit| unit.text.chars().rev().take(8).collect::<String>())
                .collect::<Vec<_>>()
        );
        assert!(last.text.ends_with(separator));
        assert_eq!(copied_text(&layout), format!("{plain}\n"));
    }
}

#[test]
fn redundant_nested_styles_emit_one_span() {
    let markup = format!("<b>{}</b>", "<b>x</b>".repeat(10_000));
    let layout = lay_out_paragraph(
        markup,
        "redundant formatting should remain cheap to lay out",
    );

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
    let parsed = parse(
        DocumentKind::Html,
        &html,
        "nested anchors should retain their visible content",
    );
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
    let span_count = layout
        .units
        .iter()
        .map(|unit| unit.spans.len())
        .sum::<usize>();
    assert!(span_count <= segments * 2);
    let links_elsewhere = |style: &DocumentSpanStyle| matches!(style, DocumentSpanStyle::Link(uri) if uri.as_ref() != "https://example.test/0");
    assert!(
        !layout
            .units
            .iter()
            .any(|unit| has_style(unit, links_elsewhere))
    );
}

#[test]
fn document_layout_keeps_tables_atomic_and_copyable_as_tsv() {
    let layout = lay_out_blocks(
        vec![
            table_row(vec![cell(true, "<b>Name</b>"), cell(true, "Value")]),
            table_row(vec![cell(false, "Alice &amp; Bob"), cell(false, "One")]),
        ],
        "table layout",
    );

    assert_eq!(layout.units.len(), 1);
    assert_eq!(layout.units[0].copy_text, "Name\tValue\nAlice & Bob\tOne\n");
    assert!(matches!(
        &layout.units[0].kind,
        DocumentUnitKind::Table { rows, .. } if rows.len() == 2
    ));
}
