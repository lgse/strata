// SPDX-License-Identifier: MIT

//! The bounds on what an untrusted caller may send (GTK's `untrusted_request_inputs_are_bounded`).

use std::path::Path;

use super::{boolean_choice, filter};
use crate::portal::*;

/// A check of an input: what it is, whether the validation accepted it, and whether it should.
type Check = (&'static str, bool, bool);

fn accepted(result: Result<(), PortalError>) -> bool {
    result.is_ok()
}

fn request_checks(too_long: &str) -> Vec<Check> {
    let path = Path::new(too_long);
    vec![
        (
            "a long title",
            accepted(validate_common_request(too_long, None, None, &[])),
            false,
        ),
        (
            "a long label",
            accepted(validate_common_request("T", Some(too_long), None, &[])),
            false,
        ),
        (
            "a long parent",
            accepted(validate_common_request("T", None, Some(too_long), &[])),
            false,
        ),
        (
            "a NUL in the title",
            accepted(validate_common_request("T\0", None, None, &[])),
            false,
        ),
        (
            "a long name",
            accepted(validate_filename(&"x".repeat(MAX_FILENAME_BYTES + 1))),
            false,
        ),
        (
            "a long path",
            accepted(validate_path(Some(path), "path")),
            false,
        ),
        ("no path", accepted(validate_path(None, "path")), true),
    ]
}

fn choice_checks() -> Vec<Check> {
    let crowded = |count: usize| PortalChoice {
        pairs: (0..count)
            .map(|index| (format!("option-{index}"), "Option".to_owned()))
            .collect(),
        ..boolean_choice("choice")
    };
    let many = (0..=MAX_CHOICES)
        .map(|index| boolean_choice(&format!("choice-{index}")))
        .collect::<Vec<_>>();
    let spread =
        vec![crowded(MAX_CHOICE_OPTIONS); MAX_TOTAL_CHOICE_OPTIONS / MAX_CHOICE_OPTIONS + 1];
    vec![
        ("too many choices", accepted(validate_choices(&many)), false),
        (
            "too many options",
            accepted(validate_choices(&[crowded(MAX_CHOICE_OPTIONS + 1)])),
            false,
        ),
        (
            "too many options in all",
            accepted(validate_choices(&spread)),
            false,
        ),
        (
            "a full choice",
            accepted(validate_choices(&[crowded(MAX_CHOICE_OPTIONS)])),
            true,
        ),
    ]
}

fn filter_checks() -> Vec<Check> {
    let bulky = filter(
        "GitHub accepted types",
        (0..=FILTER_RULE_WARNING_THRESHOLD)
            .map(|index| FilterRule::MimeType(format!("application/x-attachment-{index}")))
            .collect(),
    );
    let glob = |pattern: &str| filter("Filter", vec![FilterRule::Pattern(pattern.to_owned())]);
    let checked = |filters: &[PortalFileFilter], current: Option<&PortalFileFilter>| {
        accepted(validate_filters(filters, current))
    };
    vec![
        ("many filter rules", checked(&[bulky], None), true),
        ("a heavy glob", checked(&[glob("*a*a*a*z")], None), false),
        (
            "two wildcard groups",
            checked(&[glob("*a**.txt")], None),
            true,
        ),
        (
            "a long glob",
            checked(&[glob(&"a".repeat(MAX_GLOB_BYTES + 1))], None),
            false,
        ),
        (
            "a heavy current filter",
            checked(&[], Some(&glob("*a*b*c"))),
            false,
        ),
    ]
}

#[test]
fn untrusted_request_inputs_are_bounded() {
    let too_long = "x".repeat(MAX_STRING_BYTES + 1);
    let checks = request_checks(&too_long)
        .into_iter()
        .chain(choice_checks())
        .chain(filter_checks());
    for (why, was_accepted, expected) in checks {
        assert_eq!(was_accepted, expected, "{why}");
    }
}
