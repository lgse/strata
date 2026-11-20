// SPDX-License-Identifier: MIT

use std::time::Duration;

use super::*;
use crate::services::document::{
    DOCUMENT_INPUT_LIMIT, DOCUMENT_TABLE_COLUMN_LIMIT, Document, DocumentSpan, LayoutBudget,
    ParseLimits, append_styled_text, layout_document, parse_document_with_limits, parse_markdown,
};

fn limited_error(
    kind: DocumentKind,
    source: &str,
    limits: ParseLimits,
    expectation: &str,
) -> String {
    parse_document_with_limits(kind, source, &Cancellation::default(), limits)
        .expect_err(expectation)
}

fn rejection(
    kind: DocumentKind,
    source: &str,
    cancellation: &Cancellation,
    expectation: &str,
) -> String {
    parse_document(kind, source, cancellation).expect_err(expectation)
}

fn laid_out(
    source: &str,
    cancellation: &Cancellation,
    expectation: &str,
    layout_expectation: &str,
) -> DocumentLayout {
    let parsed = parse_document(DocumentKind::Markdown, source, cancellation).expect(expectation);
    layout_document(parsed.document, cancellation).expect(layout_expectation)
}

fn assert_input_event_and_depth_limits(cancellation: &Cancellation) {
    let oversized = "x".repeat(1024 * 1024 + 1);
    assert!(
        rejection(
            DocumentKind::Markdown,
            &oversized,
            cancellation,
            "oversized input"
        )
        .contains("1 MB")
    );

    let many_events = "x\n\n".repeat(7_000);
    assert!(
        rejection(
            DocumentKind::Markdown,
            &many_events,
            cancellation,
            "event-heavy input"
        )
        .contains("parser-event")
    );

    let deep = format!("{}x{}", "<div>".repeat(33), "</div>".repeat(33));
    assert!(
        rejection(DocumentKind::Html, &deep, cancellation, "deep input").contains("nesting-depth")
    );
}

fn html_table_row(columns: usize) -> String {
    format!("<table><tr>{}</tr></table>", "<td>x</td>".repeat(columns))
}

fn assert_blocks_and_tables_are_virtualized(cancellation: &Cancellation) {
    let many_blocks = "x\n\n".repeat(513);
    let layout = laid_out(
        &many_blocks,
        cancellation,
        "ordinary blocks are virtualized",
        "many-block layout",
    );
    assert_eq!(layout.units.len(), 513);

    let many_tables = "<table><tr><td>x</td></tr></table>".repeat(257);
    assert!(
        parse_document(DocumentKind::Html, &many_tables, cancellation).is_ok(),
        "separate tables are virtualized independently"
    );

    let table_at_limit = html_table_row(DOCUMENT_TABLE_COLUMN_LIMIT);
    assert!(parse_document(DocumentKind::Html, &table_at_limit, cancellation).is_ok());
    let table_over_limit = html_table_row(DOCUMENT_TABLE_COLUMN_LIMIT + 1);
    assert!(
        rejection(
            DocumentKind::Html,
            &table_over_limit,
            cancellation,
            "one row must keep a resident column bound",
        )
        .contains("one table")
    );

    let large_table = format!(
        "<table>{}</table>",
        "<tr><td>a</td><td>b</td></tr>".repeat(1000)
    );
    let parsed = parse_document(DocumentKind::Html, &large_table, cancellation)
        .expect("row virtualization removes the 512-cell ceiling");
    let layout = layout_document(parsed.document, cancellation).expect("large table layout");
    assert_eq!(layout.units[0].copy_text.lines().count(), 1000);
}

fn assert_long_lines_are_bounded(cancellation: &Cancellation) {
    let maximum_line = "x".repeat(DOCUMENT_INPUT_LIMIT);
    let layout = laid_out(
        &maximum_line,
        cancellation,
        "a maximum-size logical line is virtualized",
        "maximum-line layout",
    );
    assert_eq!(layout.units.len(), 512);
    assert!(units_fit_the_line_target(&layout.units));
    assert!(layout.units.iter().all(|unit| !unit.wrap));

    let one_mib_long_lines = format!("{}\n", "x".repeat(64 * 1024 - 1)).repeat(16);
    assert_eq!(one_mib_long_lines.len(), 1024 * 1024);
    let layout = laid_out(
        &one_mib_long_lines,
        cancellation,
        "one MiB split into bounded logical lines",
        "bounded layout",
    );
    assert!(units_fit_the_line_target(&layout.units));
    assert_eq!(copied_text(&layout), one_mib_long_lines);
}

fn assert_markup_and_large_blocks_are_bounded(cancellation: &Cancellation) {
    let markup = "&".repeat(1024 * 1024);
    assert!(
        rejection(
            DocumentKind::Markdown,
            &markup,
            cancellation,
            "escaped markup expansion",
        )
        .contains("markup")
    );

    assert_long_lines_are_bounded(cancellation);

    let monolithic_paragraph = format!("{}\n", "x".repeat(4 * 1024)).repeat(17);
    let parsed = parse_document(DocumentKind::Markdown, &monolithic_paragraph, cancellation)
        .expect("large multiline paragraphs are virtualized");
    assert!(layout_document(parsed.document, cancellation).is_ok());

    let large_code = format!("```text\n{}\n```", "x\n".repeat(40 * 1024));
    assert!(
        parse_document(DocumentKind::Markdown, &large_code, cancellation).is_ok(),
        "large code blocks are split into virtual units"
    );
}

fn assert_time_and_cancellation_limits(cancellation: &Cancellation) {
    let zero_time = ParseLimits {
        time: Duration::ZERO,
        ..ParseLimits::default()
    };
    assert!(
        parse_document_with_limits(DocumentKind::Markdown, "text", cancellation, zero_time)
            .expect_err("zero time budget")
            .contains("500 ms")
    );

    cancellation.cancel();
    assert!(
        rejection(
            DocumentKind::Markdown,
            "text",
            cancellation,
            "cancelled parse"
        )
        .contains("cancelled")
    );
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
        limited_error(
            DocumentKind::Html,
            &html,
            limits,
            "repeated link markup must stop at the output limit",
        )
        .contains("markup limit")
    );
}

#[test]
fn markdown_reference_expansion_stops_at_the_markup_limit_during_parsing() {
    let limits = ParseLimits {
        events: 30,
        markup: 128,
        ..ParseLimits::default()
    };
    for body in [
        "[x][ref] ".repeat(100),
        "[x][ref]\n\n".repeat(100),
        format!("| A |\n| - |\n{}", "| [x][ref] |\n".repeat(100)),
    ] {
        let markdown = format!("{body}\n\n[ref]: https://example.test/{}", "x".repeat(64));
        assert!(
            limited_error(
                DocumentKind::Markdown,
                &markdown,
                limits,
                "reference expansion must stop before the later event limit",
            )
            .contains("markup limit")
        );
    }
}

#[test]
fn parser_enforces_input_event_depth_table_markup_time_and_cancellation_limits() {
    let cancellation = Cancellation::default();
    assert_input_event_and_depth_limits(&cancellation);
    assert_blocks_and_tables_are_virtualized(&cancellation);
    assert_markup_and_large_blocks_are_bounded(&cancellation);
    assert_time_and_cancellation_limits(&cancellation);
}

#[test]
fn document_parsers_reject_nul_bytes_without_panicking() {
    for (kind, source) in [
        (DocumentKind::Markdown, "before\0after"),
        (DocumentKind::Html, "<p>before\0after</p>"),
    ] {
        assert!(
            parse_error(
                kind,
                source,
                "NUL bytes must not reach GLib markup escaping"
            )
            .contains("NUL")
        );
    }

    assert_eq!(
        parse_markdown("before\0after").blocks,
        vec![paragraph("before�after")]
    );
}

#[test]
fn pre_cancelled_html_stops_before_tokenization() {
    let html = format!("<div {}>", "data-value='x' ".repeat(32_000));
    let cancellation = Cancellation::default();
    cancellation.cancel();

    assert!(
        rejection(
            DocumentKind::Html,
            &html,
            &cancellation,
            "pre-cancelled HTML must stop before requesting a token",
        )
        .contains("cancelled")
    );
}

#[test]
fn cancelled_document_layout_stops_before_publishing_units() {
    let cancellation = Cancellation::default();
    cancellation.cancel();
    assert!(
        layout_document(
            Document {
                blocks: vec![paragraph("text")],
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
