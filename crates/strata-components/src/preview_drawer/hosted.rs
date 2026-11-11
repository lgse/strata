//! The contract between the drawer and the view it hosts. C10b CodeView, C11 DocumentView,
//! C12 PdfView, C13 MediaPlayer and C14b's viewers implement [`HostedPreview`]; Strata's own
//! preview enum delegates to whichever one is showing.
//!
//! Every method has a default meaning "not supported", so the trait grows by addition (work
//! README, rule 4) without breaking an implementor.

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

/// What a hosted preview supports in the drawer's header, and the search behind its find
/// bar. The drawer reads the getters on every build, so a control appears and goes as the
/// view's state changes (a document that falls back to Source hides the switch, say).
///
/// The drawer owns the find bar (the query field, the status, Previous, Next and Close find)
/// and calls the search methods; the view owns the matches, their highlight and the
/// selection.
///
/// Which views are searchable is each view's call. The feature map says PDF pages aren't
/// searched (`preview/preview-panel/find`), while C12's work package plans find over the
/// text layer: C12 resolves that, and either answer fits this trait.
pub trait HostedPreview {
    /// `Some(current)` shows the switch, a single button offering the other form ("View
    /// source" while rendered, "View rendered" while showing source). The host swaps the
    /// view on [`DrawerIntent::ShowView`](super::DrawerIntent::ShowView).
    fn document_view(&self) -> Option<DocumentView> {
        None
    }
    /// `Some(on)` shows the word-wrap toggle, checked when on. The host applies the change
    /// (a preference in Strata) on [`DrawerIntent::SetWrap`](super::DrawerIntent::SetWrap).
    fn wrap(&self) -> Option<bool> {
        None
    }
    /// Whether the view has searchable text now. True shows the Find button, and the find
    /// keys open the bar; false hides both and passes the keys on.
    fn searchable(&self) -> bool {
        false
    }
    /// Runs `query`: selects and reveals the first match. An empty query clears the matches.
    fn find(&mut self, _query: &str) {}
    /// Moves to the next match, or the previous one, wrapping at either end.
    fn find_step(&mut self, _forward: bool) {}
    /// Where find stands, read on every build, so a view that re-runs find itself (a source
    /// that grows while it loads) shows the new count.
    fn find_status(&self) -> Option<FindStatus> {
        None
    }
    /// The find bar closed: drop the highlights and keep the selected match selected.
    fn find_end(&mut self) {}
    /// A label while the view works (C14b's "Rendering N triangles…"): the header shows a
    /// spinner named with it, left of Open.
    fn busy(&self) -> Option<&str> {
        None
    }
}

/// A view with nothing to offer the header: placeholders and the empty reserved slot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoPreview;

impl HostedPreview for NoPreview {}
