// SPDX-License-Identifier: MIT

use super::*;
use crate::services::document::{
    DocumentMedia, DocumentSpanStyle, DocumentUnitKind, layout_document,
};

fn lay_out_markdown(source: &str, expectation: &str, layout_expectation: &str) -> DocumentLayout {
    let cancellation = Cancellation::default();
    let parsed = parse_document(DocumentKind::Markdown, source, &cancellation).expect(expectation);
    layout_document(parsed.document, &cancellation).expect(layout_expectation)
}

fn unit_texts(units: &[DocumentUnit]) -> Vec<&str> {
    units.iter().map(|unit| unit.text.as_str()).collect()
}

fn is_image_at(unit: &DocumentUnit, expected: &str, depth: Option<usize>) -> bool {
    matches!(
        &unit.kind,
        DocumentUnitKind::Media { source: DocumentMedia::Image(path), list_depth }
            if path == expected && *list_depth == depth
    )
}

fn is_image(unit: &DocumentUnit) -> bool {
    matches!(
        &unit.kind,
        DocumentUnitKind::Media {
            source: DocumentMedia::Image(_),
            ..
        }
    )
}

fn is_mermaid(unit: &DocumentUnit) -> bool {
    matches!(
        &unit.kind,
        DocumentUnitKind::Media {
            source: DocumentMedia::Mermaid(_),
            ..
        }
    )
}

fn display_math(unit: &DocumentUnit) -> Option<&str> {
    match &unit.kind {
        DocumentUnitKind::Media {
            source:
                DocumentMedia::Math {
                    source,
                    display: true,
                },
            ..
        } => Some(source.as_str()),
        _ => None,
    }
}

#[test]
fn markdown_media_preserves_image_order_lists_and_diagram_source() {
    let layout = lay_out_markdown(
        "Before\n\n![bars](images/bars.png)\n\nAfter\n\n- Item ![nested](nested.svg)\n\n```mermaid\nflowchart LR\nA-->B\n```\n\n```text\nA-->B\n```",
        "parse media",
        "layout media",
    );
    assert_eq!(unit_texts(&layout.units[..3]), ["Before", "bars", "After"]);
    assert!(is_image_at(&layout.units[1], "images/bars.png", None));
    assert!(
        layout
            .units
            .iter()
            .any(|unit| is_image_at(unit, "nested.svg", Some(0)))
    );

    let diagram = layout
        .units
        .iter()
        .find(|unit| is_mermaid(unit))
        .expect("diagram");
    assert_eq!(
        (diagram.text.as_str(), diagram.copy_text.as_str()),
        ("flowchart LR\nA-->B", "flowchart LR\nA-->B\n")
    );
    assert!(
        matches!(&diagram.kind, DocumentUnitKind::Media { source: DocumentMedia::Mermaid(source), .. } if source == &diagram.copy_text)
    );
    assert!(layout.units.iter().any(|unit| matches!(
        unit.kind,
        DocumentUnitKind::Code {
            language: Some("text"),
            ..
        } | DocumentUnitKind::Code { language: None, .. }
    )));
}

#[test]
fn markdown_media_keeps_nested_content_and_large_code_fallbacks() {
    let layout = lay_out_markdown(
        "- Parent ![parent](parent.png)\n  - Child\n\n> ```mermaid\n> flowchart LR\n> A-->B\n> ```\n\n| Picture |\n|---|\n| ![table](table.png) |",
        "nested media",
        "nested layout",
    );
    let image = layout
        .units
        .iter()
        .position(is_image)
        .expect("parent image");
    let child = layout
        .units
        .iter()
        .position(|unit| unit.text.contains("Child"))
        .expect("child text");
    assert!(image < child, "parent media stays before the nested item");
    assert!(layout.units.iter().any(is_mermaid));
    assert!(
        layout
            .units
            .iter()
            .any(|unit| unit.copy_text.contains("[Image: table]"))
    );

    let source = format!("```mermaid\nflowchart LR\n{}```", "A-->B\n".repeat(300));
    let layout = lay_out_markdown(&source, "large code", "large code layout");
    assert!(
        layout
            .units
            .iter()
            .all(|unit| matches!(unit.kind, DocumentUnitKind::Code { .. }))
    );
    assert_eq!(
        copied_text(&layout),
        format!("flowchart LR\n{}", "A-->B\n".repeat(300))
    );
}

#[test]
fn markdown_math_preserves_prose_copy_and_code_boundaries() {
    let layout = lay_out_markdown(
        "Before $E=mc^2$ after.\n\n$$\\frac{a}{b}$$\n\n```latex\n\\sqrt{x}\n```\n\n`$not math$` and \\$5\n\n| Value |\n|---|\n| $x^2$ |",
        "math Markdown",
        "math layout",
    );
    let prose = &layout.units[0];
    assert_eq!(
        (prose.text.as_str(), prose.copy_text.as_str()),
        ("Before \u{fffc} after.", "Before $E=mc^2$ after.\n")
    );
    assert_eq!(prose.copy_range(7..8), "$E=mc^2$");
    assert!(prose.spans.iter().any(
        |span| matches!(&span.style, DocumentSpanStyle::Math(source) if source.as_ref() == "E=mc^2")
    ));

    let equations: Vec<_> = layout.units.iter().filter_map(display_math).collect();
    assert_eq!(equations, vec!["\\frac{a}{b}\n", "\\sqrt{x}\n"]);
    assert!(
        layout
            .units
            .iter()
            .any(|unit| unit.text == "$not math$ and $5")
    );
    assert!(
        layout
            .units
            .iter()
            .any(|unit| unit.copy_text.contains("$x^2$"))
    );
}
