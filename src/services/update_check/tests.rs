// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    io::{Read, Write},
    net::TcpListener,
};

use super::{
    BuildKind, CHECK_INTERVAL, Channel, ReleaseNoteBlock, ReleaseResponse, ReleaseSummary,
    UpdateCheck, UpdateCheckCache, Version, archive_name, cache_is_fresh, cached_releases,
    check_from_cache, fetch_package_update, from_cached_release, package_update_from_response,
    parse_markdown, release_metadata, release_page_url, request_error_message,
    request_json_conditional, select_cached_update, select_update, to_cached_release,
    to_release_summary,
};

fn version(tag: &str) -> Version {
    Version::parse(tag).unwrap_or_else(|| panic!("expected {tag} to parse"))
}

fn release_response(json: &str) -> ReleaseResponse {
    serde_json::from_str(json).expect("release fixture should deserialize")
}

fn release_response_list(json: &str) -> Vec<ReleaseResponse> {
    serde_json::from_str(json).expect("release fixture list should deserialize")
}

/// A GitHub asset JSON object whose `name` matches [`archive_name`] for
/// `version`, i.e. the asset this platform's architecture would download.
fn matching_asset_json(version: &str) -> String {
    let name = archive_name(version);
    format!(r#"{{"name":"{name}","browser_download_url":"https://example.invalid/{name}"}}"#)
}

#[test]
fn package_update_requires_the_release_matching_the_repository_version() {
    let asset = matching_asset_json("0.8.2");
    let response = release_response(&format!(
        r#"{{"tag_name":"v0.8.2","draft":false,"prerelease":false,"assets":[{asset}]}}"#
    ));

    assert!(matches!(
        package_update_from_response(&version("0.8.1"), &response),
        UpdateCheck::Failed(_)
    ));
}

#[test]
fn package_update_accepts_the_stable_release_in_the_repository() {
    let asset = matching_asset_json("0.8.1");
    let response = release_response(&format!(
        r#"{{"tag_name":"v0.8.1","draft":false,"prerelease":false,"assets":[{asset}]}}"#
    ));

    assert!(matches!(
        package_update_from_response(&version("0.8.1"), &response),
        UpdateCheck::Available { .. }
    ));
}

#[test]
fn package_check_stays_quiet_when_the_repository_has_no_newer_version() {
    let result = fetch_package_update(&version("0.8.1"), || Ok(version("0.8.1")));

    assert_eq!(result, UpdateCheck::UpToDate);
}

// --- Version::parse migration -------------------------------------------
//
// `parse_version`/`is_newer` are gone, replaced by `release_channel::Version`.
// These migrate the coverage that used to live here rather than dropping it.

#[test]
fn newer_patch_version_is_detected() {
    assert!(version("0.2.1") > version("0.2.0"));
}

#[test]
fn newer_minor_version_is_detected() {
    assert!(version("0.3.0") > version("0.2.9"));
}

#[test]
fn equal_version_is_not_newer() {
    assert_eq!(version("0.2.0"), version("0.2.0"));
}

#[test]
fn older_version_is_not_newer() {
    assert!(version("0.1.9") < version("0.2.0"));
}

/// The old `parse_version` silently zero-filled missing/malformed segments,
/// which was the exact bug behind issue #61 (a malformed tag could compare
/// as a real version instead of being rejected). `Version::parse` must
/// reject these outright rather than falling back to zero.
#[test]
fn missing_or_malformed_segments_are_rejected() {
    assert!(Version::parse("0.2").is_none());
    assert!(Version::parse("0.2.x").is_none());
}

// --- ReleaseResponse -> ReleaseSummary conversion ------------------------

#[test]
fn malformed_tag_is_dropped() {
    let response = release_response(
        r#"{"tag_name":"not-a-version","draft":false,"prerelease":false,"assets":[]}"#,
    );
    assert!(to_release_summary(&response).is_none());
}

#[test]
fn non_matching_arch_asset_yields_no_download_url() {
    let response = release_response(
        r#"{"tag_name":"v0.5.0","draft":false,"prerelease":false,
        "assets":[{"name":"strata-0.5.0-bogus-arch.tar.gz","browser_download_url":"https://example.invalid/x"}]}"#,
    );
    let summary = to_release_summary(&response).expect("tag should parse");
    assert!(summary.download_url.is_none());
}

#[test]
fn matching_arch_asset_resolves_download_url() {
    let name = archive_name("0.5.0-rc.1");
    let response = release_response(&format!(
        r#"{{"tag_name":"v0.5.0-rc.1","draft":false,"prerelease":true,
        "assets":[{{"name":"{name}","browser_download_url":"https://example.invalid/{name}"}}]}}"#
    ));
    let summary = to_release_summary(&response).expect("tag should parse");
    assert_eq!(
        summary.download_url.as_deref(),
        Some(format!("https://example.invalid/{name}")).as_deref()
    );
}

#[test]
fn release_metadata_tolerates_missing_publication_fields() {
    let asset = matching_asset_json("0.5.0");
    let response = release_response(&format!(
        r#"{{"tag_name":"v0.5.0","draft":false,"prerelease":false,"assets":[{asset}]}}"#
    ));
    let summary = to_release_summary(&response).expect("tag should parse");
    assert!(summary.published_at.is_none());
}

#[test]
fn release_metadata_carries_channel_identity_fields() {
    let asset = matching_asset_json("0.5.0-rc.1");
    let response = release_response(&format!(
        r#"{{"tag_name":"v0.5.0-rc.1","draft":false,"prerelease":true,
        "published_at":"2026-08-01T00:00:00Z","target_commitish":"main","assets":[{asset}]}}"#
    ));
    let summary = to_release_summary(&response).expect("tag should parse");
    let metadata = release_metadata(&summary);
    assert_eq!(metadata.version, "0.5.0-rc.1");
    assert_eq!(metadata.tag, "v0.5.0-rc.1");
    assert_eq!(metadata.kind, BuildKind::Rc);
    assert_eq!(
        metadata.published_at.as_deref(),
        Some("2026-08-01T00:00:00Z")
    );
    // `target_commitish` is never the commit: GitHub returns the default
    // branch for a release published against an existing tag. The metadata
    // leaves `commit` unresolved for `resolve_commit` to fill in.
    assert!(metadata.commit.is_none());
    assert_eq!(
        metadata.url,
        "https://github.com/lgse/strata/releases/tag/v0.5.0-rc.1"
    );
}

// --- release notes plumbing, unchanged --------------------------------

#[test]
fn release_body_is_retained() {
    let asset = matching_asset_json("1.2.3");
    let response = release_response(&format!(
        r###"{{"tag_name":"v1.2.3","draft":false,"prerelease":false,"assets":[{asset}],"body":"## Changes\n\n- Fast"}}"###
    ));
    let summary = to_release_summary(&response).expect("tag should parse");
    let release = release_metadata(&summary);
    assert_eq!(release.version, "1.2.3");
    assert_eq!(release.notes, "## Changes\n\n- Fast");
    assert!(!release.note_blocks.is_empty());
}

#[test]
fn missing_release_body_becomes_empty_notes() {
    let response =
        release_response(r#"{"tag_name":"v1.2.3","draft":false,"prerelease":false,"assets":[]}"#);
    let summary = to_release_summary(&response).expect("tag should parse");
    assert!(release_metadata(&summary).notes.is_empty());
}

#[test]
fn null_release_body_becomes_empty_notes() {
    let response = release_response(
        r#"{"tag_name":"v1.2.3","draft":false,"prerelease":false,"assets":[],"body":null}"#,
    );
    let summary = to_release_summary(&response).expect("tag should parse");
    assert!(release_metadata(&summary).notes.is_empty());
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
fn release_page_url_uses_the_exact_published_tag() {
    assert_eq!(
        release_page_url("v1.2.3"),
        "https://github.com/lgse/strata/releases/tag/v1.2.3"
    );
}

// --- channel selection, fixture-driven, no network -----------------------

#[test]
fn stable_feed_flagged_prerelease_is_never_offered() {
    let asset = matching_asset_json("0.5.0");
    let response = release_response(&format!(
        r#"{{"tag_name":"v0.5.0","draft":false,"prerelease":true,"assets":[{asset}]}}"#
    ));
    let summary = to_release_summary(&response).expect("tag should parse");
    let installed = version("0.4.0");
    assert_eq!(
        select_update(Channel::Stable, &installed, &[summary]),
        UpdateCheck::UpToDate
    );
}

#[test]
fn stable_feed_prerelease_tag_with_false_flag_is_never_offered() {
    let asset = matching_asset_json("0.5.0-rc.1");
    let response = release_response(&format!(
        r#"{{"tag_name":"v0.5.0-rc.1","draft":false,"prerelease":false,"assets":[{asset}]}}"#
    ));
    let summary = to_release_summary(&response).expect("tag should parse");
    let installed = version("0.4.0");
    assert_eq!(
        select_update(Channel::Stable, &installed, &[summary]),
        UpdateCheck::UpToDate
    );
}

/// Exercises `fetch_preview`'s exact filtering pipeline against a fixture
/// list: a draft, an unparsable tag, a release with no asset for this
/// architecture, and one valid newer release. Only the valid release may
/// ever be offered.
#[test]
fn preview_feed_skips_drafts_unparsable_tags_and_assetless_releases() {
    let draft_asset = matching_asset_json("0.7.0");
    let valid_asset = matching_asset_json("0.5.0");
    let responses = release_response_list(&format!(
        r#"[
            {{"tag_name":"v0.7.0","draft":true,"prerelease":false,"assets":[{draft_asset}]}},
            {{"tag_name":"not-a-version","draft":false,"prerelease":false,"assets":[]}},
            {{"tag_name":"v0.6.0","draft":false,"prerelease":false,
              "assets":[{{"name":"strata-0.6.0-bogus-arch.tar.gz","browser_download_url":"https://example.invalid/z"}}]}},
            {{"tag_name":"v0.5.0","draft":false,"prerelease":false,"assets":[{valid_asset}]}}
        ]"#
    ));
    let summaries: Vec<_> = responses.iter().filter_map(to_release_summary).collect();
    assert_eq!(
        summaries.len(),
        3,
        "only the unparsable tag is dropped at conversion time"
    );

    let installed = version("0.1.0");
    let result = select_update(Channel::Preview, &installed, &summaries);
    match result {
        UpdateCheck::Available { release, .. } => assert_eq!(release.tag, "v0.5.0"),
        other => panic!("expected the newest eligible release, got {other:?}"),
    }
}

#[test]
fn preview_offers_final_release_over_an_installed_release_candidate() {
    let final_asset = matching_asset_json("0.5.0");
    let rc_asset = matching_asset_json("0.5.0-rc.2");
    let responses = release_response_list(&format!(
        r#"[
            {{"tag_name":"v0.5.0","draft":false,"prerelease":false,"assets":[{final_asset}]}},
            {{"tag_name":"v0.5.0-rc.2","draft":false,"prerelease":true,"assets":[{rc_asset}]}}
        ]"#
    ));
    let summaries: Vec<_> = responses.iter().filter_map(to_release_summary).collect();
    let installed = version("0.5.0-rc.2");
    let result = select_update(Channel::Preview, &installed, &summaries);
    match result {
        UpdateCheck::Available {
            release,
            download_url,
        } => {
            assert_eq!(release.tag, "v0.5.0");
            assert!(!download_url.is_empty());
        }
        other => panic!("expected final 0.5.0 to be offered, got {other:?}"),
    }
}

#[test]
fn selecting_stable_on_a_prerelease_offers_the_latest_final_as_the_channel_transition() {
    let stable_asset = matching_asset_json("0.4.0");
    let response = release_response(&format!(
        r#"{{"tag_name":"v0.4.0","draft":false,"prerelease":false,"assets":[{stable_asset}]}}"#
    ));
    let summary = to_release_summary(&response).expect("tag should parse");
    let installed = version("0.5.0-rc.2");

    match select_update(Channel::Stable, &installed, &[summary]) {
        UpdateCheck::Available { release, .. } => assert_eq!(release.tag, "v0.4.0"),
        other => panic!("expected the stable channel transition, got {other:?}"),
    }
}

// --- markdown rendering, untouched by this task ---------------------------

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

fn cache(channel: Channel, checked_at: u64) -> UpdateCheckCache {
    UpdateCheckCache {
        channel: channel.as_str().to_owned(),
        checked_at,
        etag: None,
        releases: Vec::new(),
        error: None,
    }
}

#[test]
fn a_check_just_under_the_interval_is_fresh() {
    let cache = cache(Channel::Stable, 1_000);
    let now = 1_000 + CHECK_INTERVAL.as_secs() - 1;
    assert!(cache_is_fresh(&cache, Channel::Stable, false, now));
}

#[test]
fn a_check_at_or_past_the_interval_is_not_fresh() {
    let cache = cache(Channel::Stable, 1_000);
    let now = 1_000 + CHECK_INTERVAL.as_secs();
    assert!(!cache_is_fresh(&cache, Channel::Stable, false, now));
}

#[test]
fn a_forced_check_is_never_fresh() {
    let cache = cache(Channel::Stable, 1_000);
    assert!(!cache_is_fresh(&cache, Channel::Stable, true, 1_000));
}

#[test]
fn a_channel_mismatch_is_never_fresh() {
    let cache = cache(Channel::Stable, 1_000);
    assert!(!cache_is_fresh(&cache, Channel::Preview, false, 1_000));
}

#[test]
fn a_cache_from_the_future_is_still_fresh() {
    // Clock rollback must not make the cache stale through underflow.
    let cache = cache(Channel::Stable, 2_000);
    assert!(cache_is_fresh(&cache, Channel::Stable, false, 1_000));
}

fn release_summary(tag: &str) -> ReleaseSummary {
    ReleaseSummary {
        tag: tag.to_owned(),
        version: version(tag),
        draft: false,
        prerelease: false,
        download_url: Some("https://example.invalid/asset".to_owned()),
        published_at: Some("2026-01-01T00:00:00Z".to_owned()),
        notes: "Notes".to_owned(),
    }
}

#[test]
fn a_cached_release_round_trips_every_field() {
    let original = release_summary("v0.8.0");
    let restored = from_cached_release(&to_cached_release(&original))
        .expect("a freshly cached release should still parse");
    assert_eq!(restored.tag, original.tag);
    assert_eq!(restored.version, original.version);
    assert_eq!(restored.draft, original.draft);
    assert_eq!(restored.prerelease, original.prerelease);
    assert_eq!(restored.download_url, original.download_url);
    assert_eq!(restored.published_at, original.published_at);
    assert_eq!(restored.notes, original.notes);
}

#[test]
fn a_cached_release_with_an_unparseable_tag_is_dropped() {
    let mut cached = to_cached_release(&release_summary("v0.8.0"));
    cached.tag = "not-a-version".to_owned();
    assert!(from_cached_release(&cached).is_none());
}

#[test]
fn a_cached_update_reuses_its_resolved_commit() {
    let releases = vec![release_summary("v0.8.0")];
    let mut check = select_update(Channel::Stable, &version("v0.7.0"), &releases);
    if let UpdateCheck::Available { release, .. } = &mut check {
        release.commit = Some("0123456789abcdef".to_owned());
    }
    let cached = cached_releases(&releases, &check);

    let restored = select_cached_update(Channel::Stable, &version("v0.7.0"), &cached);

    assert!(matches!(
        restored,
        UpdateCheck::Available { release, .. }
            if release.commit.as_deref() == Some("0123456789abcdef")
    ));
}

#[test]
fn a_cached_failure_remains_a_failure() {
    let mut cache = cache(Channel::Stable, 1_000);
    cache.error = Some("Network request failed".to_owned());

    assert_eq!(
        check_from_cache(&cache, Channel::Stable, &version("v0.8.0")),
        UpdateCheck::Failed("Network request failed".to_owned())
    );
}

#[test]
fn a_not_modified_response_reuses_the_cached_representation() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let address = listener
        .local_addr()
        .expect("test server should have an address");
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("test server should accept");
        let mut request = [0_u8; 1024];
        let _read = stream
            .read(&mut request)
            .expect("request should be readable");
        stream
            .write_all(
                b"HTTP/1.1 304 Not Modified\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .expect("response should be writable");
    });

    let result = request_json_conditional::<serde_json::Value>(
        &format!("http://{address}"),
        Some("\"cached-etag\""),
    )
    .expect("304 should not be treated as an error");

    assert!(result.is_none());
    server.join().expect("test server should stop");
}
