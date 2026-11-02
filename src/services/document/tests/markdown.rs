// SPDX-License-Identifier: MIT

use super::*;
use crate::services::document::{document_kind, parse_markdown};

#[test]
fn markdown_renders_supported_formatting_as_blocks() {
    assert_eq!(
        parse_markdown("## Changes\n\n- **Fast** and `safe`\n- [Details](https://example.test)")
            .blocks,
        vec![
            heading(2, "Changes"),
            bullet(0, "<b>Fast</b> and <tt>safe</tt>"),
            bullet(0, "<a href=\"https://example.test\">Details</a>"),
        ]
    );
}

#[test]
fn multiline_formatting_and_code_stay_in_balanced_blocks() {
    assert_eq!(
        parse_markdown("**first\nsecond**\n\n```text\none < two\n```").blocks,
        vec![
            paragraph("<b>first\nsecond</b>"),
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
            list_item("3.", 0, "outer"),
            bullet(1, "inner"),
            list_item("4.", 0, "next"),
        ]
    );
}

#[test]
fn markdown_keeps_html_inert_and_does_not_retain_image_urls() {
    let blocks = parse_markdown(
        "<script>alert('no')</script>\n\n![tracking](https://example.test/pixel.png)",
    )
    .blocks;
    assert_debug_mentions(
        &debug_blocks(&blocks),
        &["&lt;script&gt;", "[Image: tracking]"],
        &["<script>", "pixel.png"],
    );
}

#[test]
fn markdown_does_not_activate_non_web_links() {
    assert_eq!(
        parse_markdown("[Run](javascript:alert('no'))").blocks,
        vec![paragraph("<u>Run</u>")]
    );
}

#[test]
fn malformed_markdown_and_entities_remain_inert() {
    let blocks = parse_markdown("<broken & **unfinished").blocks;
    assert_debug_mentions(&debug_blocks(&blocks), &["&lt;broken &amp;"], &["<broken"]);
}

#[test]
fn empty_markdown_has_no_blocks() {
    assert!(parse_markdown("  \n").blocks.is_empty());
}

#[test]
fn release_notes_hide_html_comments_but_keep_other_html_inert() {
    let item = |markup: &str| bullet(0, markup);
    let heading = |markup: &str| heading(2, markup);
    for (input, expected) in [
        (
            "<!-- Release notes generated using configuration in .github/release.yml at v0.21.0 -->\n\n## What's Changed\n* fix(a): b by @x in https://example.test/pull/1\n",
            vec![
                heading("What&apos;s Changed"),
                item("fix(a): b by @x in https://example.test/pull/1"),
            ],
        ),
        ("<!--\nmulti\nline\n-->\n\nAfter", vec![paragraph("After")]),
        // Indented HTML blocks also emit synthesised whitespace text events.
        (
            "  <!-- indented\n  comment -->\n\nAfter",
            vec![paragraph("After")],
        ),
        // CommonMark ends a comment block at the end of the line holding
        // `-->`, so trailing text on that line belongs to the hidden block.
        ("<!-- a --> tail\n\nAfter", vec![paragraph("After")]),
        ("<!-- only a comment -->\n", Vec::new()),
        // An unterminated comment runs to the end of the input.
        ("<!-- never closed\n## Hidden", Vec::new()),
        (
            "Before <!-- inline --> after",
            vec![paragraph("Before  after")],
        ),
        (
            "## Title <!-- c -->\n\ntext",
            vec![heading("Title "), paragraph("text")],
        ),
        (
            "- item\n  <!-- c -->\n- next",
            vec![item("item"), item("next")],
        ),
        (
            "<b>x</b>\n\n<div>\nraw\n</div>",
            vec![
                paragraph("&lt;b&gt;x&lt;/b&gt;"),
                paragraph("&lt;div&gt;\nraw\n&lt;/div&gt;"),
            ],
        ),
    ] {
        assert_eq!(parse_markdown(input).blocks, expected, "{input:?}");
    }
}

#[test]
fn classifies_supported_local_document_mimes_and_extensions_case_insensitively() {
    for mime in [
        "text/markdown",
        "text/x-markdown",
        "TEXT/HTML",
        "application/xhtml+xml",
        "application/rtf",
        "text/rtf",
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
        "a.RTF",
    ] {
        assert!(document_kind("text/plain", std::ffi::OsStr::new(name), true).is_some());
    }
    for (name, enabled) in [("notes.md", false), ("notes.txt", true)] {
        assert_eq!(
            document_kind("text/plain", std::ffi::OsStr::new(name), enabled),
            None
        );
    }
}

#[test]
fn markdown_supports_quotes_tables_tasks_strikethrough_and_safe_images() {
    let parsed = parse(
        DocumentKind::Markdown,
        "> quoted\n\n- [x] ~~done~~\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n![alt](file:///secret)",
        "supported Markdown should render",
    );
    let blocks = &parsed.document.blocks;
    assert!(matches!(blocks[0], DocumentBlock::Quote(_)));
    assert!(blocks.contains(&table_row(vec![cell(true, "A"), cell(true, "B")])));
    assert_debug_mentions(
        &debug_blocks(blocks),
        &["☑ <s>done</s>", "[Image: alt]"],
        &["file:///secret"],
    );
}

#[test]
fn markdown_raw_html_is_inert_and_warned() {
    for (source, inert) in [
        (
            "before <b onclick=\"run()\">after</b>",
            "&lt;b onclick=&quot;run()&quot;&gt;",
        ),
        ("<!-- c -->\n\nbody", "&lt;!-- c --&gt;"),
    ] {
        let parsed = parse(
            DocumentKind::Markdown,
            source,
            "raw HTML should remain inert text",
        );
        let debug = debug_blocks(&parsed.document.blocks);
        assert!(debug.contains(inert), "{source:?}: {debug}");
        assert_eq!(parsed.warnings.len(), 1, "{source:?}");
    }
}
