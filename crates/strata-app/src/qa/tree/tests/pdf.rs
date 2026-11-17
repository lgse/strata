// SPDX-License-Identifier: MIT

//! The preview's PDF pages across a zoom (`segment_of`).

use serde_json::json;

use super::super::{Drawn, Snapshot};

/// The node of a remembered `image` "PDF page text" in the snapshot of `tree`, as `recall`
/// finds it again in the snapshot of `later`.
fn page_text_across(tree: &serde_json::Value, later: &serde_json::Value) -> Option<String> {
    let page = |snapshot: &Snapshot| {
        (0..)
            .map_while(|index| snapshot.at(index))
            .find(|seen| seen.name == "PDF page text")
            .and_then(|seen| seen.node.clone())
    };
    let before = Snapshot::read(tree, &Drawn::default());
    let kept = before.keep(&page(&before)?);
    let after = Snapshot::read(later, &Drawn::default());
    after.recall(&kept).map(|node| node.0.to_string())
}

/// A page row keyed `{key}-z{zoom}-p{page}` under `list`.
fn pdf_tree(list: &str, row: &str) -> serde_json::Value {
    json!({ "role": "window", "children": [
        { "role": "group", "key": list, "children": [
            { "role": "group", "key": row, "children": [
                { "role": "group", "key": "page", "name": "Page 1 of 2", "children": [
                    { "role": "image", "key": "text-area", "name": "PDF page text" } ] } ] } ] } ] })
}

/// A PDF page remembered before a zoom is the same page after it, though C12 keys its row by
/// the zoom; a key of the same shape outside the preview's PDF is a key like any other.
#[test]
fn a_remembered_pdf_page_survives_a_zoom_and_only_the_pdfs_keys_are_read_so() {
    let pdf = |row: &str| pdf_tree("preview-pdf-pages", row);
    let found = page_text_across(&pdf("preview-pdf-z3-p0"), &pdf("preview-pdf-z4-p0"));
    assert!(found.is_some_and(|node| node.contains("/preview-pdf-p0/")));
    let other = |row: &str| pdf_tree("gallery-pages", row);
    assert_eq!(
        page_text_across(&other("gallery-z3-p0"), &other("gallery-z4-p0")),
        None
    );
}
