//! Sorting, as Strata's `TableState` does it (`src/ui/table_view.rs`, lgse/strata#1946): a
//! stable sort of a row permutation, by every sort column most recent first, ties in source
//! order. Keys are built per column on its first sort.

use std::cmp::Ordering;

use rune_kit::SortDir;

/// One cell's sort key: a finite number, else its text lowercased, else empty. Ascending puts
/// numbers first, then text, then empty cells.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SortKey {
    Number(f64),
    Text(String),
    Empty,
}

impl SortKey {
    pub(crate) fn new(text: &str) -> SortKey {
        let text = text.trim();
        if text.is_empty() {
            return SortKey::Empty;
        }
        match text.parse::<f64>() {
            Ok(n) if n.is_finite() => SortKey::Number(n),
            _ => SortKey::Text(text.to_lowercase()),
        }
    }

    fn compare(&self, other: &SortKey) -> Ordering {
        match (self, other) {
            (SortKey::Number(a), SortKey::Number(b)) => a.total_cmp(b),
            (SortKey::Text(a), SortKey::Text(b)) => a.cmp(b),
            (SortKey::Empty, SortKey::Empty) => Ordering::Equal,
            (SortKey::Number(_), _) | (SortKey::Text(_), SortKey::Empty) => Ordering::Less,
            _ => Ordering::Greater,
        }
    }
}

const EMPTY: SortKey = SortKey::Empty;

/// One column's keys. A column with a cell in at least half the rows keeps a key per row;
/// a sparse one (a few long rows in a ragged sheet) keeps only the rows it has, so 256
/// columns over 25,000 mostly short rows never cost 6.4 million keys.
#[derive(Debug)]
pub(crate) enum ColumnKeys {
    Dense(Vec<SortKey>),
    Sparse(Vec<(u32, SortKey)>),
}

impl ColumnKeys {
    pub(crate) fn new(body: &[Vec<String>], column: usize) -> ColumnKeys {
        let present = body.iter().filter(|r| r.len() > column).count();
        if present >= body.len().div_ceil(2) {
            ColumnKeys::Dense(
                body.iter()
                    .map(|r| r.get(column).map_or(SortKey::Empty, |t| SortKey::new(t)))
                    .collect(),
            )
        } else {
            ColumnKeys::Sparse(
                body.iter()
                    .enumerate()
                    .filter_map(|(i, r)| Some((i as u32, SortKey::new(r.get(column)?))))
                    .collect(),
            )
        }
    }

    fn key(&self, row: u32) -> &SortKey {
        match self {
            ColumnKeys::Dense(keys) => &keys[row as usize],
            ColumnKeys::Sparse(keys) => keys
                .binary_search_by_key(&row, |(r, _)| *r)
                .map_or(&EMPTY, |i| &keys[i].1),
        }
    }

    fn compare(&self, a: u32, b: u32) -> Ordering {
        self.key(a).compare(self.key(b))
    }
}

/// What a click on column `column`'s title makes of the sort `spec` (most recent first): the
/// primary column flips; any other column becomes the primary, ascending, and the others keep
/// their order and direction after it, as GTK's `ColumnViewSorter` does.
pub(crate) fn clicked(spec: &[(usize, SortDir)], column: usize) -> Vec<(usize, SortDir)> {
    match spec.first() {
        Some(&(c, d)) if c == column => {
            let mut out = spec.to_vec();
            out[0].1 = d.flip();
            out
        }
        _ => std::iter::once((column, SortDir::Ascending))
            .chain(spec.iter().copied().filter(|(c, _)| *c != column))
            .collect(),
    }
}

/// The body rows in `spec`'s order, ties in source order.
pub(crate) fn order(rows: usize, spec: &[(&ColumnKeys, SortDir)]) -> Vec<u32> {
    let mut order: Vec<u32> = (0..rows as u32).collect();
    if spec.is_empty() {
        return order;
    }
    // `sort_by` is stable: rows equal on every key keep source order.
    order.sort_by(|&a, &b| {
        for &(keys, dir) in spec {
            let o = keys.compare(a, b);
            if o != Ordering::Equal {
                return dir.apply(o);
            }
        }
        Ordering::Equal
    });
    order
}

/// `order`, sorted by a spec that differs from the one it was sorted by only in the primary
/// column's direction: the primary key's groups reversed, each group's own order kept, in
/// linear time. It gives what [`order`] would.
pub(crate) fn flipped(order: &[u32], primary: &ColumnKeys) -> Vec<u32> {
    let mut out: Vec<u32> = order.iter().rev().copied().collect();
    for group in out.chunk_by_mut(|&a, &b| primary.compare(a, b) == Ordering::Equal) {
        group.reverse();
    }
    out
}
