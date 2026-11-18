// SPDX-License-Identifier: MIT

//! The names drawn with their matches in the accent (#2570), as GTK's `refresh_name_highlights`
//! marks them (`src/ui/browser/find.rs`): a highlighted 10xer find marks its query in every
//! name, the listing's and the results'; without one, the results of a fuzzy `f` or `s` mark
//! the parts their query matched. The components only paint the ranges ([`NameMatches`]).

use std::ops::Range;

use strata_components::{
    collection::element::{NameMatches, RowMarks},
    contract::EntryId,
};

use crate::App;

#[cfg(test)]
mod tests;

/// The find query highlighted now, while 10xer mode is on (`find_highlight`).
fn find(app: &App) -> Option<&str> {
    crate::shell::preferences(app)
        .tenxer_mode
        .then(|| app.tenxer.prompt_state.find.highlight())
        .flatten()
}

/// `marks` with listing row `name`'s highlight (`highlight_listing_name`).
pub(crate) fn listing_marks(app: &App, name: Option<&str>, mut marks: RowMarks) -> RowMarks {
    if let (Some(query), Some(name)) = (find(app), name) {
        marks.name_matches = NameMatches::new(match_ranges(name, query));
    }
    marks
}

/// `marks` with result `id`'s highlight (`highlight_hit_name`): the find's, else its query's.
pub(crate) fn hit_marks(app: &App, id: EntryId, mut marks: RowMarks) -> RowMarks {
    let ranges = match find(app) {
        Some(query) => {
            crate::filter::tenxer::hit_name(app, id).map(|name| match_ranges(name, query))
        }
        None => crate::filter::tenxer::hit_name_ranges(app, id),
    };
    if let Some(ranges) = ranges {
        marks.name_matches = NameMatches::new(ranges);
    }
    marks
}

/// The byte ranges of `name` where `query` occurs, compared case-folded, each after the last
/// (`match_ranges`). A character whose lowercase is longer keeps its own byte span.
pub(crate) fn match_ranges(name: &str, query: &str) -> Vec<Range<usize>> {
    let needle: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let folded: Vec<(Range<usize>, char)> = name
        .char_indices()
        .flat_map(|(start, character)| {
            let span = start..start + character.len_utf8();
            character
                .to_lowercase()
                .map(move |lower| (span.clone(), lower))
        })
        .collect();
    let mut ranges = Vec::new();
    let mut index = 0;
    while index + needle.len() <= folded.len() {
        let window = &folded[index..index + needle.len()];
        let found = window
            .iter()
            .zip(&needle)
            .all(|((_, character), wanted)| character == wanted);
        if found {
            ranges.push(window[0].0.start..window[needle.len() - 1].0.end);
            index += needle.len();
        } else {
            index += 1;
        }
    }
    ranges
}
