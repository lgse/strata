// SPDX-License-Identifier: MIT

use std::ops::Range;

use rune::{Cx, El, KeyEvent, Tx, col, row, text};
use strata_components::contract::{MatchedText, fuzzy};

use crate::{Gallery, Scene};

#[derive(Default)]
pub struct Demo {}

pub const SCENES: &[Scene] = &[
    ("w1-matched-light", |g| g.open("w1", false)),
    ("w1-matched-dark", |g| g.open("w1", true)),
];

/// Scenes for review with `--scene` only, kept out of the goldens.
pub const REVIEW: &[Scene] = &[];

/// A long name whose match lies past where the label cuts it short.
const LONG: &str = "a-very-long-file-name-that-will-not-fit-in-this-row-quarterly-report-final.pdf";

/// The ranges of `query` in `text`, matched as a substring.
fn substring(text: &str, query: &str) -> Vec<Range<usize>> {
    text.find(query)
        .map(|at| at..at + query.len())
        .into_iter()
        .collect()
}

/// The ranges `fuzzy` matches, or none.
fn fuzzy_ranges(query: &str, text: &str) -> Vec<Range<usize>> {
    fuzzy(query, text).map(|m| m.ranges).unwrap_or_default()
}

/// One row of the page.
struct Case {
    /// The suffix of the row's keys.
    id: &'static str,
    caption: &'static str,
    label: &'static str,
    ranges: Vec<Range<usize>>,
    /// Keep the label to one line in a narrow width.
    narrow: bool,
}

fn case(
    id: &'static str,
    caption: &'static str,
    label: &'static str,
    ranges: Vec<Range<usize>>,
) -> Case {
    Case {
        id,
        caption,
        label,
        ranges,
        narrow: false,
    }
}

fn cases() -> Vec<Case> {
    vec![
        case(
            "fuzzy",
            "fuzzy “stc”",
            "strata-components",
            fuzzy_ranges("stc", "strata-components"),
        ),
        case(
            "prefix",
            "prefix “Doc”",
            "Documents",
            substring("Documents", "Doc"),
        ),
        case(
            "merged",
            "touching ranges merge",
            "Downloads",
            vec![0..2, 2..4, 7..9],
        ),
        case(
            "accented",
            "fuzzy “dé”",
            "Café déjà vu",
            fuzzy_ranges("dé", "Café déjà vu"),
        ),
        case(
            "whole",
            "the whole text",
            "Music",
            substring("Music", "Music"),
        ),
        case("none", "no match", "Pictures", Vec::new()),
        Case {
            narrow: true,
            ..case(
                "truncated",
                "match past the cut",
                LONG,
                substring(LONG, "report"),
            )
        },
    ]
}

pub fn view(g: &Gallery, _cx: &mut Cx<Gallery>) -> El {
    let kit = g.kit();
    let line = |c: Case| {
        let label = MatchedText::new(c.label, c.ranges)
            .font(kit.ty.ui_compact)
            .key(format!("w1-matched-{}", c.id))
            .truncate(c.narrow)
            .build(kit);
        row()
            .gap(16.0)
            .child(
                text(c.caption)
                    .key(format!("w1-case-{}", c.id))
                    .font(kit.ty.text_small)
                    .color(kit.c.text_muted)
                    .w(180.0),
            )
            .child(
                row()
                    .w(360.0)
                    .child(label.w(if c.narrow { 260.0 } else { 360.0 })),
            )
    };
    col()
        .w_full()
        .h_full()
        .center()
        .gap(12.0)
        .child(
            text("Matched text")
                .key("w1-title")
                .font(kit.ty.ui_medium)
                .color(kit.c.text_base),
        )
        .children(cases().into_iter().map(line))
}

pub fn on_key(_g: &mut Gallery, _key: &KeyEvent, _tx: &mut Tx) -> bool {
    false
}

pub fn poll(_g: &mut Gallery, _t: f64, _tx: &mut Tx) -> bool {
    false
}
