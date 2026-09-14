// SPDX-License-Identifier: MIT

use super::{Channel, ReleaseSummary, Version, best_update, is_eligible, rollback_target};

fn parse(tag: &str) -> Version {
    Version::parse(tag).unwrap_or_else(|| panic!("expected {tag} to parse"))
}

/// Builds a [`ReleaseSummary`] for a final, non-draft release with an
/// installable asset, so each test only needs to override what it cares
/// about.
fn release(tag: &str) -> ReleaseSummary {
    ReleaseSummary {
        tag: tag.to_string(),
        version: parse(tag),
        draft: false,
        prerelease: false,
        download_url: Some(format!("https://example.invalid/{tag}")),
        published_at: None,
        notes: String::new(),
    }
}

#[test]
fn staged_prereleases_order_within_same_core() {
    assert!(parse("0.5.0-alpha.2") > parse("0.5.0-alpha.1"));
    assert!(parse("0.5.0-beta.2") > parse("0.5.0-beta.1"));
    assert!(parse("0.5.0-rc.2") > parse("0.5.0-rc.1"));
    assert!(parse("0.5.0-alpha.9") > parse("0.5.0-nightly.20260901"));
    assert!(parse("0.5.0-beta.1") > parse("0.5.0-alpha.9"));
    assert!(parse("0.5.0-rc.1") > parse("0.5.0-beta.9"));
}

#[test]
fn final_release_outranks_its_own_prerelease() {
    assert!(parse("0.5.0") > parse("0.5.0-rc.2"));
}

#[test]
fn patch_bump_outranks_prior_release() {
    assert!(parse("0.5.0") > parse("0.4.9"));
}

#[test]
fn nightly_ordinal_orders_by_date_within_same_core() {
    assert!(parse("0.5.0-nightly.20260901") > parse("0.5.0-nightly.20260831"));
}

#[test]
fn nightly_same_date_orders_by_suffix() {
    assert!(parse("0.5.0-nightly.20260901.2") > parse("0.5.0-nightly.20260901.1"));
    assert!(parse("0.5.0-nightly.20260901.1") > parse("0.5.0-nightly.20260901"));
}

#[test]
fn equal_versions_compare_equal() {
    assert_eq!(parse("0.5.0-rc.1"), parse("0.5.0-rc.1"));
    assert_eq!(parse("0.5.0"), parse("0.5.0"));
    assert_eq!(
        parse("0.5.0-nightly.20260901.2"),
        parse("0.5.0-nightly.20260901.2")
    );
}

#[test]
fn rc_outranks_nightly_for_the_same_core() {
    // Pinned by D5: within an equal core, a release candidate compares
    // greater than a nightly build. This matches semver §11's alphanumeric
    // comparison of prerelease identifiers ("nightly" sorts before "rc"),
    // and reflects that once an RC is cut for a core version, that line
    // has stabilized -- a same-core nightly should never outrank it.
    assert!(parse("0.5.0-rc.1") > parse("0.5.0-nightly.20260901"));
}

#[test]
fn nightly_suffix_does_not_collide_with_the_next_days_date() {
    // Regression test: the ordinal must not be a packed `date * 1000 + n`
    // integer, since the grammar places no bound on `N`. A large suffix
    // must never spill into the date component.
    assert!(parse("0.5.0-nightly.20260901.1000") != parse("0.5.0-nightly.20260902"));
    assert!(parse("0.5.0-nightly.20260901.1000") < parse("0.5.0-nightly.20260902"));
}

#[test]
fn accepts_canonical_forms() {
    assert!(Version::parse("v0.5.0").is_some());
    assert!(Version::parse("0.5.0").is_some());
    assert!(Version::parse("v0.5.0-alpha.1").is_some());
    assert!(Version::parse("v0.5.0-beta.1").is_some());
    assert!(Version::parse("v0.5.0-rc.1").is_some());
    assert!(Version::parse("v0.5.0-nightly.20260901").is_some());
    assert!(Version::parse("v0.5.0-nightly.20260901.2").is_some());
}

/// Missing or malformed segments must be rejected outright. The old
/// `parse_version` silently zero-filled them, which was the bug behind
/// issue #61: a truncated tag could compare as a real version.
#[test]
fn rejects_malformed_tags() {
    assert!(Version::parse("0.5").is_none());
    assert!(Version::parse("0.5.x").is_none());
    assert!(Version::parse("v0.5.0-preview.1").is_none());
    assert!(Version::parse("v0.5.0-alpha.0").is_none());
    assert!(Version::parse("v0.5.0-beta.0").is_none());
    assert!(Version::parse("v0.5.0-rc.0").is_none());
    assert!(Version::parse("v0.5.0-rc").is_none());
    assert!(Version::parse("v0.5.0-rc.x").is_none());
    assert!(Version::parse("").is_none());
    assert!(Version::parse("vv0.5.0").is_none());
    assert!(Version::parse("0.5.0-rc.1-extra").is_none());
}

#[test]
fn display_renders_canonical_tags() {
    for (input, expected) in [
        ("v0.5.0-alpha.1", "0.5.0-alpha.1"),
        ("v0.5.0-beta.2", "0.5.0-beta.2"),
        ("v0.5.0-rc.3", "0.5.0-rc.3"),
        ("v0.5.0", "0.5.0"),
        ("v0.5.0-nightly.20260901.2", "0.5.0-nightly.20260901.2"),
        ("v0.5.0-nightly.20260901", "0.5.0-nightly.20260901"),
        (
            "v0.5.0-nightly.20260901.1000",
            "0.5.0-nightly.20260901.1000",
        ),
    ] {
        assert_eq!(parse(input).to_string(), expected);
    }
}

#[test]
fn channel_round_trips() {
    assert_eq!(Channel::parse("stable"), Channel::Stable);
    assert_eq!(Channel::parse("preview"), Channel::Preview);
    assert_eq!(Channel::parse("nightly"), Channel::Nightly);
    assert_eq!(Channel::parse(""), Channel::Stable);
}

#[test]
fn draft_rejected_on_stable_and_preview() {
    let draft = ReleaseSummary {
        draft: true,
        ..release("v0.5.0")
    };
    assert!(!is_eligible(Channel::Stable, &draft));
    assert!(!is_eligible(Channel::Preview, &draft));
}

#[test]
fn assetless_release_rejected_on_both_channels() {
    let assetless = ReleaseSummary {
        download_url: None,
        ..release("v0.5.0")
    };
    assert!(!is_eligible(Channel::Stable, &assetless));
    assert!(!is_eligible(Channel::Preview, &assetless));
}

#[test]
fn stable_rejects_prerelease_tag_even_when_flag_is_false() {
    let mislabelled = ReleaseSummary {
        prerelease: false,
        ..release("v0.5.0-rc.1")
    };
    assert!(!is_eligible(Channel::Stable, &mislabelled));
}

#[test]
fn stable_rejects_prerelease_flag_even_when_tag_parses_as_final() {
    let mislabelled = ReleaseSummary {
        prerelease: true,
        ..release("v0.5.0")
    };
    assert!(!is_eligible(Channel::Stable, &mislabelled));
}

#[test]
fn preview_excludes_nightly_while_nightly_accepts_every_build_kind() {
    let final_release = release("v0.5.0");
    let rc = ReleaseSummary {
        prerelease: true,
        ..release("v0.5.0-rc.1")
    };
    let nightly = ReleaseSummary {
        prerelease: true,
        ..release("v0.5.0-nightly.20260901")
    };
    assert!(is_eligible(Channel::Preview, &final_release));
    assert!(is_eligible(Channel::Preview, &rc));
    assert!(!is_eligible(Channel::Preview, &nightly));
    assert!(is_eligible(Channel::Nightly, &final_release));
    assert!(is_eligible(Channel::Nightly, &rc));
    assert!(is_eligible(Channel::Nightly, &nightly));
}

#[test]
fn best_update_on_stable_with_only_prereleases_is_none() {
    let installed = parse("0.4.0");
    let releases = [
        ReleaseSummary {
            prerelease: true,
            ..release("v0.5.0-rc.1")
        },
        ReleaseSummary {
            prerelease: true,
            ..release("v0.5.0-rc.2")
        },
    ];
    assert!(best_update(Channel::Stable, &installed, &releases).is_none());
}

#[test]
fn best_update_on_preview_prefers_final_over_installed_rc() {
    let installed = parse("0.5.0-rc.2");
    let releases = [
        release("v0.5.0"),
        ReleaseSummary {
            prerelease: true,
            ..release("v0.5.0-rc.2")
        },
    ];
    let result = best_update(Channel::Preview, &installed, &releases);
    assert_eq!(result.map(|r| r.tag.as_str()), Some("v0.5.0"));
}

#[test]
fn best_update_never_offers_a_downgrade() {
    for (channel, installed, older) in [
        (
            Channel::Preview,
            "0.5.0-rc.2",
            ReleaseSummary {
                prerelease: true,
                ..release("v0.5.0-rc.1")
            },
        ),
        (Channel::Stable, "0.5.0", release("v0.4.0")),
    ] {
        assert!(
            best_update(channel, &parse(installed), &[older]).is_none(),
            "{channel:?}"
        );
    }
}

#[test]
fn best_update_when_installed_equals_newest_is_none() {
    let installed = parse("0.5.0");
    let releases = [release("v0.5.0")];
    assert!(best_update(Channel::Stable, &installed, &releases).is_none());
}

#[test]
fn rollback_target_ignores_installed_version() {
    let releases = [
        ReleaseSummary {
            prerelease: true,
            ..release("v0.5.0-rc.2")
        },
        release("v0.4.0"),
    ];
    let result = rollback_target(&releases);
    assert_eq!(result.map(|r| r.tag.as_str()), Some("v0.4.0"));
}

#[test]
fn rollback_target_with_no_final_releases_is_none() {
    let releases = [ReleaseSummary {
        prerelease: true,
        ..release("v0.5.0-rc.2")
    }];
    assert!(rollback_target(&releases).is_none());
}
