// SPDX-License-Identifier: MIT

//! A map rule's `rune_adds`, held to one copy of an element for each time the field's rules
//! name it (#2614's review).

use serde_json::{Value, json};

use super::{
    compare::compare,
    compare_tests::observed,
    observe_tests::stream,
    replay::Stream,
    rules::{self, Ignored, exceptions},
};

const VOLUME: &[&str] = &[r#"app.preview deviation lgse/strata#2606 {"rune_adds":["Volume"]}"#];
const ART: &[&str] =
    &[r#"app.preview oracle lgse/strata#2612 {"rune_adds":["Album art","Audio spectrum"]}"#];

/// Two streams, GTK's and Rune's, whose one observation shows the preview's names.
fn preview_streams(gtk: &Value, rune: &Value) -> (Stream, Stream) {
    let (mut left, mut right) = (stream(&["archive"]), stream(&["archive"]));
    right.header.target = "rune".to_owned();
    left.observations[0].value = observed(json!({ "preview": gtk }));
    right.observations[0].value = observed(json!({ "preview": rune }));
    (left, right)
}

/// A node Rune names twice is a duplicated node, which still diverges, beside GTK's own copy
/// or not, while one added copy beside GTK's own is what the ruling allows.
pub(crate) fn a_rune_adds_rule_takes_out_one_copy_for_each_time_it_names_an_element() {
    let cases = [
        (
            VOLUME,
            json!(["alpha"]),
            json!(["Volume", "alpha", "Volume"]),
            true,
        ),
        (
            VOLUME,
            json!(["Volume", "alpha"]),
            json!(["Volume", "alpha", "Volume", "Volume"]),
            true,
        ),
        (
            VOLUME,
            json!(["Volume", "alpha"]),
            json!(["Volume", "alpha", "Volume"]),
            false,
        ),
        (
            ART,
            json!(["tone"]),
            json!(["Album art", "tone", "Album art"]),
            true,
        ),
        (
            ART,
            json!(["tone"]),
            json!(["Album art", "tone", "Audio spectrum"]),
            false,
        ),
    ];
    for (lines, gtk, rune, diverges) in cases {
        let ruled = Ignored {
            rules: rules::parse(&lines.join("\n"), &exceptions()).expect("the rules should parse"),
            masks: Vec::new(),
        };
        let (left, right) = preview_streams(&gtk, &rune);
        let found = compare(&left, &right, &ruled).is_some();
        assert_eq!(found, diverges, "GTK {gtk} against Rune {rune}");
    }
}
