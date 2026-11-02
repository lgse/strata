// SPDX-License-Identifier: MIT

use super::*;

/// One source per route that must parse to the same blocks.
struct SharedOutline<'a> {
    markdown: (&'a str, &'a str),
    html: (&'a str, &'a str),
}

fn assert_markdown_and_html_match(sources: SharedOutline<'_>, expected: &[DocumentBlock]) {
    let (markdown, markdown_expectation) = sources.markdown;
    assert_parses_to(
        DocumentKind::Markdown,
        markdown,
        markdown_expectation,
        expected,
    );
    let (html, html_expectation) = sources.html;
    assert_parses_to(DocumentKind::Html, html, html_expectation, expected);
}

fn resumed_parent_item() -> Vec<DocumentBlock> {
    vec![
        bullet(0, "outer"),
        bullet(1, "inner"),
        list_child(DocumentListChildKind::Paragraph, "tail"),
    ]
}

fn semantic_list_children(language: &'static str, code: &str) -> Vec<DocumentBlock> {
    vec![
        bullet(0, "outer"),
        list_child(DocumentListChildKind::Heading(2), "heading"),
        list_child(DocumentListChildKind::Quote, "quote"),
        list_child(DocumentListChildKind::Code(Some(language)), code),
    ]
}

fn list_table_row(cells: Vec<DocumentTableCell>) -> DocumentBlock {
    DocumentBlock::ListTableRow { depth: 0, cells }
}

#[derive(Debug, PartialEq)]
enum Shape {
    Item,
    Rule,
    ListRow,
    Row,
    Boundary,
    Other,
}

fn container_shape(blocks: &[DocumentBlock]) -> Vec<Shape> {
    blocks
        .iter()
        .map(|block| match block {
            DocumentBlock::ListItem { .. } => Shape::Item,
            DocumentBlock::ListRule { .. } => Shape::Rule,
            DocumentBlock::ListTableRow { .. } => Shape::ListRow,
            DocumentBlock::TableRow { .. } => Shape::Row,
            DocumentBlock::ContainerBoundary => Shape::Boundary,
            _ => Shape::Other,
        })
        .collect()
}

#[test]
fn html_restores_parent_list_items_after_nested_lists() {
    assert_parses_to(
        DocumentKind::Html,
        "<ul><li><p>outer</p><ul><li>inner</li></ul><p>tail</p></li></ul>",
        "nested lists should resume their parent item",
        &resumed_parent_item(),
    );
}

#[test]
fn markdown_restores_parent_list_items_after_nested_lists() {
    assert_parses_to(
        DocumentKind::Markdown,
        "- outer\n  - inner\n\n  tail",
        "nested Markdown lists should resume their parent item",
        &resumed_parent_item(),
    );
}

#[test]
fn list_item_paragraphs_remain_separate_children() {
    assert_markdown_and_html_match(
        SharedOutline {
            markdown: (
                "- first\n\n  second",
                "Markdown list paragraphs should render separately",
            ),
            html: (
                "<ul><li><p>first</p><p>second</p></li></ul>",
                "HTML list paragraphs should render separately",
            ),
        },
        &[
            bullet(0, "first"),
            list_child(DocumentListChildKind::Paragraph, "second"),
        ],
    );
}

#[test]
fn markdown_keeps_semantic_block_children_inside_list_items() {
    assert_parses_to(
        DocumentKind::Markdown,
        "- outer\n\n  ## heading\n\n  > quote\n\n  ```rust\n  code\n  ```",
        "Markdown block children should retain their list context",
        &semantic_list_children("rust", "code\n"),
    );
}

#[test]
fn html_keeps_semantic_block_children_inside_list_items() {
    assert_parses_to(
        DocumentKind::Html,
        "<ul><li>outer<h2>heading</h2><blockquote><p>quote</p></blockquote><pre><code class=\"language-js\">code</code></pre></li></ul>",
        "HTML block children should retain their list context",
        &semantic_list_children("js", "code"),
    );
}

#[test]
fn list_blockquote_paragraphs_keep_both_semantics() {
    assert_markdown_and_html_match(
        SharedOutline {
            markdown: (
                "- > first\n  >\n  > second",
                "Markdown quote paragraphs should remain inside their list item",
            ),
            html: (
                "<ul><li><blockquote><p>first</p><p>second</p></blockquote></li></ul>",
                "HTML quote paragraphs should remain inside their list item",
            ),
        },
        &[
            bullet(0, ""),
            list_child(DocumentListChildKind::Quote, "first"),
            list_child(DocumentListChildKind::Quote, "second"),
        ],
    );
}

#[test]
fn list_rules_and_tables_do_not_orphan_trailing_content() {
    assert_markdown_and_html_match(
        SharedOutline {
            markdown: (
                "- outer\n\n  ---\n\n  | A |\n  |---|\n  | B |\n\n  tail",
                "Markdown rules and tables should remain inside their list item",
            ),
            html: (
                "<ul><li>outer<hr><table><tr><th>A</th></tr><tr><td>B</td></tr></table><p>tail</p></li></ul>",
                "HTML rules and tables should remain inside their list item",
            ),
        },
        &[
            bullet(0, "outer"),
            DocumentBlock::ListRule { depth: 0 },
            list_table_row(vec![cell(true, "A")]),
            list_table_row(vec![cell(false, "B")]),
            list_child(DocumentListChildKind::Paragraph, "tail"),
        ],
    );
}

#[test]
fn separate_lists_and_tables_keep_container_boundaries() {
    use Shape::{Boundary, Item, ListRow, Row, Rule};

    for (kind, source, expectation, shape) in [
        (
            DocumentKind::Markdown,
            "- one\n\n1. two",
            "separate Markdown lists should render",
            vec![Item, Boundary, Item],
        ),
        (
            DocumentKind::Html,
            "<ul><li>one</ul><ol><li>two</ol>",
            "separate HTML lists should render",
            vec![Item, Boundary, Item],
        ),
        (
            DocumentKind::Markdown,
            "- first\n\n  ---\n\n1. second",
            "a rule must not merge separate Markdown lists",
            vec![Item, Rule, Boundary, Item],
        ),
        (
            DocumentKind::Html,
            "<ul><li>first<hr></li></ul><ol><li>second</li></ol>",
            "a rule must not merge separate HTML lists",
            vec![Item, Rule, Boundary, Item],
        ),
        (
            DocumentKind::Markdown,
            "- first\n\n  | A |\n  | - |\n  | B |\n\n1. second",
            "a table must not merge separate Markdown lists",
            vec![Item, ListRow, ListRow, Boundary, Item],
        ),
        (
            DocumentKind::Html,
            "<ul><li>first<table><tr><td>A</td></tr></table></li></ul><ol><li>second</li></ol>",
            "a table must not merge separate HTML lists",
            vec![Item, ListRow, Boundary, Item],
        ),
        (
            DocumentKind::Markdown,
            "| A |\n| - |\n| 1 |\n\n| B |\n| - |\n| 2 |",
            "adjacent Markdown tables should render",
            vec![Row, Row, Boundary, Row, Row],
        ),
        (
            DocumentKind::Html,
            "<table><tr><td>A</table><table><tr><td>B</table>",
            "adjacent HTML tables should render",
            vec![Row, Boundary, Row],
        ),
    ] {
        let blocks = parsed_blocks(kind, source, expectation);
        assert_eq!(container_shape(&blocks), shape, "{source:?}");
    }
}
