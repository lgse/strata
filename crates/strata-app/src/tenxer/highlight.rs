// SPDX-License-Identifier: MIT

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

/// Whether 10xer mode currently highlights a find query on listing rows.
pub(crate) fn has_find(app: &App) -> bool {
    find(app).is_some()
}

/// `marks` with listing row `name`'s highlight (`highlight_listing_name`).
pub(crate) fn listing_marks(app: &App, name: Option<&str>, mut marks: RowMarks) -> RowMarks {
    if let (Some(query), Some(name)) = (find(app), name) {
        marks.name_matches = within_reach(name, match_ranges(name, query));
    }
    marks
}

/// `marks` with result `id`'s highlight (`highlight_hit_name`): the find's, else its query's.
pub(crate) fn hit_marks(app: &App, id: EntryId, mut marks: RowMarks) -> RowMarks {
    let Some(name) = crate::filter::tenxer::hit_name(app, id) else {
        return marks;
    };
    let ranges = match find(app) {
        Some(query) => Some(match_ranges(name, query)),
        None => crate::filter::tenxer::hit_name_ranges(app, id),
    };
    if let Some(ranges) = ranges {
        marks.name_matches = within_reach(name, ranges);
    }
    marks
}

fn within_reach(name: &str, ranges: Vec<Range<usize>>) -> NameMatches {
    let reach = (0..=NameMatches::REACH.min(name.len()))
        .rev()
        .find(|at| name.is_char_boundary(*at))
        .unwrap_or(0);
    NameMatches::new(
        ranges
            .into_iter()
            .map(|range| range.start..range.end.min(reach)),
    )
}

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
