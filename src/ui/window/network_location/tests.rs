// SPDX-License-Identifier: MIT

use super::*;
use crate::adapters::bookmarks::{parse_pinned_places, serialize_pinned_places};

/// Unpin matches the row reloaded from the bookmarks file, so a pin must come
/// back from that file unchanged.
#[test]
fn pinned_network_locations_survive_the_bookmark_round_trip() {
    for uri in [
        "smb://host/share",
        "smb://host/share/",
        "sftp://host/home/me",
        "davs://host/files",
    ] {
        let pinned = normalized_location(Location::uri(uri));
        let place = (pinned.clone(), pin_label(&pinned));

        let reloaded =
            parse_pinned_places(serialize_pinned_places(std::slice::from_ref(&place)).as_bytes());

        assert_eq!(reloaded, vec![place], "{uri}");
    }
}
