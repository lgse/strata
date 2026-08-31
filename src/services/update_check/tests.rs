// SPDX-License-Identifier: GPL-3.0-or-later

use super::{
    ReleaseNoteBlock, ReleaseResponse, is_newer, metadata, parse_markdown, release_page_url,
    request_error_message,
};

#[test]
fn newer_patch_version_is_detected() {
    assert!(is_newer("0.2.1", "0.2.0"));
}

#[test]
fn newer_minor_version_is_detected() {
    assert!(is_newer("0.3.0", "0.2.9"));
}

#[test]
fn equal_version_is_not_newer() {
    assert!(!is_newer("0.2.0", "0.2.0"));
}

#[test]
fn older_version_is_not_newer() {
    assert!(!is_newer("0.1.9", "0.2.0"));
}

#[test]
fn missing_or_malformed_segments_fall_back_to_zero() {
    assert!(!is_newer("0.2", "0.2.0"));
    assert!(!is_newer("0.2.x", "0.2.1"));
}

#[test]
fn release_body_is_retained() {
    let response: ReleaseResponse = serde_json::from_str(
        r###"{"tag_name":"v1.2.3","html_url":"https://example.test/release","body":"## Changes\n\n- Fast"}"###,
    )
    .expect("release fixture should deserialize");
    let release = metadata(&response);
    assert_eq!(release.version, "1.2.3");
    assert_eq!(release.notes, "## Changes\n\n- Fast");
    assert!(!release.note_blocks.is_empty());
}

#[test]
fn missing_release_body_becomes_empty_notes() {
    let response: ReleaseResponse =
        serde_json::from_str(r#"{"tag_name":"v1.2.3","html_url":"https://example.test/release"}"#)
            .expect("release fixture should deserialize");
    assert!(metadata(&response).notes.is_empty());
}

#[test]
fn null_release_body_becomes_empty_notes() {
    let response: ReleaseResponse = serde_json::from_str(
        r#"{"tag_name":"v1.2.3","html_url":"https://example.test/release","body":null}"#,
    )
    .expect("release fixture should deserialize");
    assert!(metadata(&response).notes.is_empty());
}

#[test]
fn rate_limit_failures_have_a_distinct_message() {
    assert_eq!(
        request_error_message(&ureq::Error::StatusCode(429)),
        "GitHub API rate limit reached"
    );
}

#[test]
fn other_api_failures_include_the_status() {
    assert_eq!(
        request_error_message(&ureq::Error::StatusCode(500)),
        "GitHub API returned HTTP 500"
    );
}

#[test]
fn release_markdown_renders_supported_formatting_as_blocks() {
    assert_eq!(
        parse_markdown("## Changes\n\n- **Fast** and `safe`\n- [Details](https://example.test)"),
        vec![
            ReleaseNoteBlock::Heading {
                level: 2,
                markup: "Changes".to_owned(),
            },
            ReleaseNoteBlock::ListItem {
                marker: "•".to_owned(),
                depth: 0,
                markup: "<b>Fast</b> and <tt>safe</tt>".to_owned(),
            },
            ReleaseNoteBlock::ListItem {
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
        parse_markdown("**first\nsecond**\n\n```text\none < two\n```"),
        vec![
            ReleaseNoteBlock::Paragraph("<b>first\nsecond</b>".to_owned()),
            ReleaseNoteBlock::Code("one &lt; two\n".to_owned()),
        ]
    );
}

#[test]
fn nested_and_ordered_lists_keep_markers_and_depth() {
    let blocks = parse_markdown("3. outer\n   - inner\n4. next");
    assert_eq!(
        blocks,
        vec![
            ReleaseNoteBlock::ListItem {
                marker: "3.".to_owned(),
                depth: 0,
                markup: "outer".to_owned(),
            },
            ReleaseNoteBlock::ListItem {
                marker: "•".to_owned(),
                depth: 1,
                markup: "inner".to_owned(),
            },
            ReleaseNoteBlock::ListItem {
                marker: "4.".to_owned(),
                depth: 0,
                markup: "next".to_owned(),
            },
        ]
    );
}

#[test]
fn release_markdown_keeps_html_inert_and_does_not_load_images() {
    let blocks = parse_markdown(
        "<script>alert('no')</script>\n\n![tracking](https://example.test/pixel.png)",
    );
    let debug = format!("{blocks:?}");
    assert!(!debug.contains("<script>"));
    assert!(debug.contains("&lt;script&gt;"));
    assert!(!debug.contains("pixel.png"));
    assert!(debug.contains("[Image: tracking]"));
}

#[test]
fn release_markdown_does_not_activate_non_web_links() {
    assert_eq!(
        parse_markdown("[Run](javascript:alert('no'))"),
        vec![ReleaseNoteBlock::Paragraph("<u>Run</u>".to_owned())]
    );
}

#[test]
fn malformed_markdown_and_entities_remain_inert() {
    let blocks = parse_markdown("<broken & **unfinished");
    let debug = format!("{blocks:?}");
    assert!(debug.contains("&lt;broken &amp;"));
    assert!(!debug.contains("<broken"));
}

#[test]
fn empty_release_markdown_has_no_blocks() {
    assert!(parse_markdown("  \n").is_empty());
}

#[test]
fn current_release_fallback_uses_exact_version_tag() {
    assert_eq!(
        release_page_url("1.2.3"),
        "https://github.com/lgse/strata/releases/tag/v1.2.3"
    );
}
