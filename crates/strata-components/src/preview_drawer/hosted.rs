// SPDX-License-Identifier: MIT

/// Which form of a document the preview shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DocumentView {
    #[default]
    Rendered,
    Source,
}

impl DocumentView {
    /// The other form: what the header's switch button offers.
    pub fn other(self) -> DocumentView {
        match self {
            DocumentView::Rendered => DocumentView::Source,
            DocumentView::Source => DocumentView::Rendered,
        }
    }
}

/// Where find stands in the hosted view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FindStatus {
    /// The selected match, from 1, or `None` with no match selected.
    pub current: Option<usize>,
    /// How many matches the query has.
    pub total: usize,
}

pub trait HostedPreview {
    fn document_view(&self) -> Option<DocumentView> {
        None
    }
    fn wrap(&self) -> Option<bool> {
        None
    }
    fn searchable(&self) -> bool {
        false
    }
    /// Runs `query`: selects and reveals the first match. An empty query clears the matches.
    fn find(&mut self, _query: &str) {}
    /// Moves to the next match, or the previous one, wrapping at either end.
    fn find_step(&mut self, _forward: bool) {}
    fn find_status(&self) -> Option<FindStatus> {
        None
    }
    /// The find bar closed: drop the highlights and keep the selected match selected.
    fn find_end(&mut self) {}
    fn busy(&self) -> Option<&str> {
        None
    }
}

/// A view with nothing to offer the header: placeholders and the empty reserved slot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoPreview;

impl HostedPreview for NoPreview {}
