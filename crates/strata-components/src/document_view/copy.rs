// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::ops::Range;

use rune::{SelectedText, TextSelection};

use super::model::UnitKind;
use super::state::Content;

/// Which text of a unit a selected text is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Part {
    /// The unit's text element, keyed `t`.
    Text,
    /// A table cell, keyed `r{row}c{column}`.
    Cell(usize, usize),
}

/// The unit and part of a selected text, from its path (`…/{key}-u{unit}/…/t`).
pub(crate) fn locate(key: &str, path: &str) -> Option<(usize, Part)> {
    let mut unit = None;
    let mut last = "";
    for seg in path.split('/') {
        if let Some(n) = seg
            .strip_prefix(key)
            .and_then(|rest| rest.strip_prefix("-u"))
            .and_then(|n| n.parse().ok())
        {
            unit = Some(n);
        }
        last = seg;
    }
    let part = if last == "t" {
        Part::Text
    } else {
        let (r, c) = last.strip_prefix('r')?.split_once('c')?;
        Part::Cell(r.parse().ok()?, c.parse().ok()?)
    };
    Some((unit?, part))
}

struct Piece<'a> {
    part: Part,
    range: Range<usize>,
    whole: bool,
    text: &'a SelectedText,
}

pub(crate) fn copy(state: &Content, sel: &TextSelection) -> String {
    let key = state.key.as_str();
    let units = &state.doc.units;
    let mut by_unit: BTreeMap<usize, Vec<Piece<'_>>> = BTreeMap::new();
    for t in sel.texts().iter().filter(|t| t.is_selected()) {
        if let Some((unit, part)) = locate(key, t.path())
            && unit < units.len()
        {
            by_unit.entry(unit).or_default().push(Piece {
                part,
                range: t.range(),
                whole: t.is_whole(),
                text: t,
            });
        }
    }
    let (Some(&first), Some(&last)) = (by_unit.keys().next(), by_unit.keys().next_back()) else {
        // Nothing of the document's: what Rune would copy.
        return sel.text().to_owned();
    };
    let mut out = String::new();
    for (i, unit) in units.iter().enumerate().take(last + 1).skip(first) {
        let Some(pieces) = by_unit.get(&i).filter(|_| i == first || i == last) else {
            out.push_str(&state.unit_copy_text(i));
            continue;
        };
        match &unit.kind {
            UnitKind::Table { rows, .. } => {
                let body = state.table_body_rows(i);
                let selectable_cells: usize = body
                    .iter()
                    .filter_map(|&r| rows.get(r))
                    .flat_map(|row| row.iter())
                    .filter(|c| !c.text.is_empty())
                    .count();
                if i > first
                    && selectable_cells > 0
                    && pieces.len() == selectable_cells
                    && pieces.iter().all(|p| p.whole)
                {
                    out.push_str(&state.unit_copy_text(i));
                } else {
                    table_part(&mut out, pieces);
                    if i < last {
                        out.push('\n');
                    }
                }
            }
            _ => {
                let Some(p) = pieces.iter().find(|p| p.part == Part::Text) else {
                    continue;
                };
                let len = state.shown(i).0.len();
                if p.range.start == 0 && p.range.end >= len {
                    out.push_str(&unit.copy_text);
                    continue;
                }
                match state.shown_of(i) {
                    Some(shown) => out.push_str(&shown.copy(p.range.clone())),
                    None => out.push_str(&unit.text[p.range.clone()]),
                }
                if p.range.end >= len && i < last && unit.copy_text.ends_with('\n') {
                    out.push('\n');
                }
            }
        }
    }
    out
}

/// The selected cells of a table, a tab between cells of a row and a newline between rows.
fn table_part(out: &mut String, pieces: &[Piece<'_>]) {
    let mut row = None;
    for p in pieces {
        let Part::Cell(r, _) = p.part else { continue };
        match row {
            Some(prev) if prev == r => out.push('\t'),
            Some(_) => out.push('\n'),
            None => {}
        }
        row = Some(r);
        out.push_str(p.text.text());
    }
}

const RUNE_ROWS: usize = 10_000;
const RUNE_BYTES: usize = 4 << 20;

/// Whether the document has any selectable prose or table text.
pub(crate) fn has_selectable_text(state: &Content) -> bool {
    ends(state).is_some()
}

/// Whether `sel` selects any text belonging to this document.
pub(crate) fn has_selection_in(state: &Content, sel: &TextSelection) -> bool {
    let key = state.key.as_str();
    let len = state.doc.units.len();
    sel.texts()
        .iter()
        .filter(|t| t.is_selected())
        .any(|t| locate(key, t.path()).is_some_and(|(unit, _)| unit < len))
}

fn ends(state: &Content) -> Option<((usize, Part), (usize, Part))> {
    let units = &state.doc.units;
    let part = |i: usize, last: bool| -> Option<(usize, Part)> {
        let u = &units[i];
        match &u.kind {
            UnitKind::Table { rows, .. } => {
                let body = state.table_body_rows(i);
                let mut cells = body.into_iter().enumerate().flat_map(|(dr, src)| {
                    rows.get(src).into_iter().flat_map(move |row| {
                        row.iter()
                            .enumerate()
                            .filter(|(_, c)| !c.text.is_empty())
                            .map(move |(c, _)| Part::Cell(dr, c))
                    })
                });
                let p = if last { cells.last() } else { cells.next() };
                p.map(|p| (i, p))
            }
            UnitKind::Media { .. } => None,
            kind => (super::state::searchable(kind) && !state.shown(i).0.is_empty())
                .then_some((i, Part::Text)),
        }
    };
    let first = (0..units.len()).find_map(|i| part(i, false))?;
    let last = (0..units.len()).rev().find_map(|i| part(i, true))?;
    Some((first, last))
}

pub(crate) fn reads_whole(state: &Content, sel: &TextSelection) -> bool {
    let key = state.key.as_str();
    let Some((doc_first, doc_last)) = ends(state) else {
        return false;
    };
    let mut first = None;
    let mut last = None;
    let mut unit_count = 0usize;
    let mut prev_unit = None;
    let mut bytes = 0usize;
    for t in sel.texts().iter().filter(|t| t.is_selected()) {
        if !t.is_whole() {
            return false;
        }
        let loc = locate(key, t.path());
        if first.is_none() {
            first = Some(loc);
        }
        last = Some(loc);
        if let Some((u, _)) = loc
            && prev_unit != Some(u)
        {
            prev_unit = Some(u);
            unit_count += 1;
        }
        bytes += t.len();
    }
    if first != Some(Some(doc_first)) {
        return false;
    }
    let cut = unit_count >= RUNE_ROWS || bytes >= RUNE_BYTES;
    last == Some(Some(doc_last)) || cut
}
