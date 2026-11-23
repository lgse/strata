// SPDX-License-Identifier: MIT

use std::cmp::Ordering;

use rune_kit::SortDir;

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

pub(crate) fn flipped(order: &[u32], primary: &ColumnKeys) -> Vec<u32> {
    let mut out: Vec<u32> = order.iter().rev().copied().collect();
    for group in out.chunk_by_mut(|&a, &b| primary.compare(a, b) == Ordering::Equal) {
        group.reverse();
    }
    out
}
