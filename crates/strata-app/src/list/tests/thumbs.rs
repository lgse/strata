// SPDX-License-Identifier: MIT

//! The rows' thumbnails, from the window's store.

use strata_components::{contract::EntrySource, thumbnails::State};

use super::{LIST, MODULE, settle};
use crate::test_support::view::{hosted_in, open};

/// An image's row has its thumbnail asked for once its folder shows, as GTK's List row asks
/// in `request_thumbnail_and_metadata`, and a folder opened in place has its own asked for.
#[test]
fn an_image_row_has_its_thumbnail_asked_for_in_each_folder_shown() {
    hosted_in(
        MODULE,
        "list",
        "thumbs::an_image_row_has_its_thumbnail_asked_for_in_each_folder_shown",
        |mut host| {
            let folder = tempfile::tempdir().expect("folder");
            std::fs::write(folder.path().join("one.png"), "one").expect("file");
            std::fs::create_dir(folder.path().join("inner")).expect("folder");
            std::fs::write(folder.path().join("inner/two.png"), "two").expect("file");
            for (shown, image) in [
                (folder.path().to_owned(), "one.png"),
                (folder.path().join("inner"), "two.png"),
            ] {
                open(&mut host, LIST, &shown);
                settle(&mut host);
                let app = host.app();
                let listing = app.list.single.pane.listing().expect("a folder shown");
                let index = (0..listing.len())
                    .find(|index| listing.entry(*index).name == image)
                    .expect("the image's row");
                let key = listing.entry(index).thumbnail.expect("a thumbnail key");
                let state = crate::shell::thumbnails(app).state(key);
                assert_ne!(state, State::Empty, "{image} was never asked for");
            }
        },
    );
}
