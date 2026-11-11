//! Find over the text layers the view holds: literal, case-insensitive (C11's folding, so
//! "éclair" finds "ÉCLAIR"), matches that don't overlap, each within one page. As layers
//! arrive (find's text-only requests, or pages scrolled to), their matches join in page
//! order, as Strata's find re-runs over a source that grows while it loads.

use super::model::Shared;

/// A match: a page and a character range of its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hit {
    pub page: usize,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Default)]
pub(crate) struct Find {
    pub query: String,
    /// In page order, then text order.
    pub hits: Vec<Hit>,
    pub current: Option<usize>,
    /// Counts the runs: a page's text joins the count once per run.
    pub generation: u64,
}

impl Find {
    pub fn active(&self) -> bool {
        !self.query.is_empty()
    }

    /// Runs `query` over every layer: the first match becomes current.
    pub fn run<'a>(&mut self, query: &str, layers: impl Iterator<Item = (usize, &'a Shared)>) {
        self.query = query.to_owned();
        self.hits.clear();
        self.current = None;
        self.generation += 1;
        if query.is_empty() {
            return;
        }
        for (page, layer) in layers {
            self.hits.extend(page_hits(query, page, layer));
        }
        self.hits.sort();
        self.current = (!self.hits.is_empty()).then_some(0);
    }

    /// A layer for `page` arrived or changed: its matches replace the page's, and the current
    /// match stays the same match. True when the current match was none and now is the first.
    pub fn add(&mut self, page: usize, layer: Option<&Shared>) -> bool {
        if !self.active() {
            return false;
        }
        let current = self.current.map(|c| self.hits[c]);
        self.hits.retain(|h| h.page != page);
        if let Some(layer) = layer {
            self.hits.extend(page_hits(&self.query, page, layer));
        }
        self.hits.sort();
        match current {
            Some(c) => {
                self.current = self.hits.iter().position(|h| *h == c).or_else(|| {
                    // The current page's text changed under it: the next match after it.
                    (!self.hits.is_empty()).then(|| {
                        self.hits
                            .partition_point(|h| *h < c)
                            .min(self.hits.len() - 1)
                    })
                });
                false
            }
            None if !self.hits.is_empty() => {
                self.current = Some(0);
                true
            }
            None => false,
        }
    }

    /// Steps to the next or previous match, wrapping. The new current match, if any.
    pub fn step(&mut self, forward: bool) -> Option<Hit> {
        let n = self.hits.len();
        if n == 0 {
            return None;
        }
        let c = match self.current {
            Some(c) if forward => (c + 1) % n,
            Some(c) => (c + n - 1) % n,
            None => 0,
        };
        self.current = Some(c);
        Some(self.hits[c])
    }

    pub fn current_hit(&self) -> Option<Hit> {
        self.current.map(|c| self.hits[c])
    }

    pub fn clear(&mut self) {
        let generation = self.generation;
        *self = Find::default();
        self.generation = generation;
    }

    /// The matches on `page`.
    pub fn on_page(&self, page: usize) -> &[Hit] {
        let a = self.hits.partition_point(|h| h.page < page);
        let b = self.hits.partition_point(|h| h.page <= page);
        &self.hits[a..b]
    }
}

fn page_hits<'a>(query: &'a str, page: usize, layer: &'a Shared) -> impl Iterator<Item = Hit> + 'a {
    layer
        .find(query)
        .into_iter()
        .map(move |(start, end)| Hit { page, start, end })
}
