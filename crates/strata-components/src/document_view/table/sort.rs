// SPDX-License-Identifier: MIT

use std::cmp::Ordering;

use rune_kit::SortDir;

use super::super::model::Cell;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SortKey {
    Number(f64),
    Text(String),
    Empty,
}

impl SortKey {
    pub(crate) fn new(text: &str) -> Self {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Self::Empty;
        }
        match trimmed.parse::<f64>() {
            Ok(n) if n.is_finite() => Self::Number(n),
            _ => Self::Text(trimmed.to_lowercase()),
        }
    }

    pub(crate) fn compare(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Number(a), Self::Number(b)) => a.total_cmp(b),
            (Self::Text(a), Self::Text(b)) => a.cmp(b),
            (Self::Empty, Self::Empty) => Ordering::Equal,
            (Self::Number(_), _) | (Self::Text(_), Self::Empty) => Ordering::Less,
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
    pub(crate) fn new(body: &[Vec<Cell>], column: usize) -> Self {
        let present = body.iter().filter(|r| r.len() > column).count();
        if present >= body.len().div_ceil(2) {
            Self::Dense(
                body.iter()
                    .map(|r| {
                        r.get(column)
                            .map_or(SortKey::Empty, |c| SortKey::new(&c.text))
                    })
                    .collect(),
            )
        } else {
            Self::Sparse(
                body.iter()
                    .enumerate()
                    .filter_map(|(i, r)| Some((i as u32, SortKey::new(&r.get(column)?.text))))
                    .collect(),
            )
        }
    }

    fn key(&self, row: u32) -> &SortKey {
        match self {
            Self::Dense(keys) => &keys[row as usize],
            Self::Sparse(keys) => keys
                .binary_search_by_key(&row, |(r, _)| *r)
                .map_or(&EMPTY, |i| &keys[i].1),
        }
    }

    pub(crate) fn compare(&self, a: u32, b: u32) -> Ordering {
        self.key(a).compare(self.key(b))
    }
}

pub(crate) fn clicked(spec: &[(usize, SortDir)], column: usize) -> Vec<(usize, SortDir)> {
    match spec.first() {
        Some(&(c, SortDir::Ascending)) if c == column => {
            let mut out = spec.to_vec();
            out[0].1 = SortDir::Descending;
            out
        }
        Some(&(c, SortDir::Descending)) if c == column => Vec::new(),
        _ => std::iter::once((column, SortDir::Ascending))
            .chain(spec.iter().copied().filter(|(c, _)| *c != column))
            .collect(),
    }
}

pub(crate) fn shift_clicked(spec: &[(usize, SortDir)], column: usize) -> Vec<(usize, SortDir)> {
    let mut out = spec.to_vec();
    if let Some(pos) = out.iter().position(|&(c, _)| c == column) {
        if out[pos].1 == SortDir::Ascending {
            out[pos].1 = SortDir::Descending;
        } else {
            out.remove(pos);
        }
    } else {
        out.push((column, SortDir::Ascending));
    }
    out
}

/// Body row indices in `spec`'s order, ties kept in source document order.
pub(crate) fn order(rows: usize, spec: &[(&ColumnKeys, SortDir)]) -> Vec<u32> {
    let mut out: Vec<u32> = (0..rows as u32).collect();
    if spec.is_empty() {
        return out;
    }
    out.sort_by(|&a, &b| compare_spec(a, b, spec));
    out
}

fn compare_spec(a: u32, b: u32, spec: &[(&ColumnKeys, SortDir)]) -> Ordering {
    for &(keys, dir) in spec {
        let ord = keys.compare(a, b);
        if ord != Ordering::Equal {
            return dir.apply(ord);
        }
    }
    Ordering::Equal
}

/// Reverses the primary key's groups in linear time without disturbing secondary tie-breakers.
pub(crate) fn flipped(order: &[u32], primary: &ColumnKeys) -> Vec<u32> {
    let mut out: Vec<u32> = order.iter().rev().copied().collect();
    for group in out.chunk_by_mut(|&a, &b| primary.compare(a, b) == Ordering::Equal) {
        group.reverse();
    }
    out
}
