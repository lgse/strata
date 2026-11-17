// SPDX-License-Identifier: MIT

//! A page row's identity across zooms ([`page_row_identity`]).

use super::super::model::PdfDocument;
use super::super::page_row_identity;
use super::PdfState;

// A harness follows one page across zooms: its row's key without the zoom generation.
#[test]
fn a_page_row_keeps_one_identity_across_zooms() {
    let mut st = PdfState::new("preview-pdf");
    st.open(PdfDocument::new(3, [600.0, 800.0]));
    let before = st.row_key(2);
    st.zoom_gen = st.zoom_gen.wrapping_add(5);
    let after = st.row_key(2);
    assert_ne!(before, after, "the row is keyed by its zoom");
    let identity = page_row_identity(&after);
    assert_eq!(page_row_identity(&before), identity);
    assert_eq!(identity.as_deref(), Some("preview-pdf-p2"));
    for other in [
        "preview-pdf-pages",
        "preview-pdf-z-p1",
        "preview-pdf-z2-p",
        "entry-12",
    ] {
        assert_eq!(page_row_identity(other), None, "{other}");
    }
}
