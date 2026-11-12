// SPDX-License-Identifier: MIT

use strata_components::{
    contract::{EntryId, EntryKind, ListEntry, ListSource, ThumbKey},
    thumbnails::{Outcome, Settings, Step, ThumbnailStore, Viewport},
};
use strata_preview::sandbox::Cancellation;

use super::{Viewer, decode};
use crate::slots::Inbox;

fn photo() -> ListSource {
    let mut entry = ListEntry::new(EntryId::new(1), "photo.png", EntryKind::File, "");
    entry.thumbnail = Some(ThumbKey::new(7));
    ListSource::new(vec![entry])
}

fn red(width: u32) -> Outcome {
    Outcome::Ready {
        width,
        height: width,
        rgba: [255, 0, 0, 255].repeat((width * width) as usize),
    }
}

/// A result that comes back after its folder was left is dropped, and the store holds
/// nothing for it; one still wanted lands.
#[test]
fn a_result_for_a_folder_left_is_dropped_without_touching_the_store() {
    let inbox = Inbox::new();
    let mut viewer = Viewer::new(inbox.later());
    let mut store = ThumbnailStore::new(Settings::default());
    let shown = photo();
    let step = Step::new(64);
    let requests = viewer.take_requests(&mut store, 0.0);
    assert!(requests.is_empty(), "nothing is shown yet");
    let id = viewer.id().expect("a viewer");
    store.sync(id, &shown, Viewport::new(0..1, step));
    let ticket = viewer.take_requests(&mut store, 0.0)[0].ticket;
    viewer.pending.insert(ticket, Cancellation::default());

    store.sync(id, &ListSource::new(Vec::new()), Viewport::new(0..0, step));
    let _ = viewer.take_requests(&mut store, 0.0);

    assert!(!viewer.finish(&mut store, (ticket, red(64)), 0.0));
    assert_eq!(store.stats().ready, 0);

    store.sync(id, &shown, Viewport::new(0..1, step));
    let ticket = viewer.take_requests(&mut store, 1.0)[0].ticket;
    viewer.pending.insert(ticket, Cancellation::default());
    assert!(viewer.finish(&mut store, (ticket, red(64)), 1.0));
    assert_eq!(store.stats().ready, 1);
}

/// A render larger than the step is scaled down to it, keeping its shape.
#[test]
fn a_render_is_decoded_down_to_its_step() {
    let mut png = Vec::new();
    let image = image::RgbaImage::from_pixel(256, 128, image::Rgba([0, 255, 0, 255]));
    image
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("a PNG");

    let Outcome::Ready {
        width,
        height,
        rgba,
    } = decode(&png, 64)
    else {
        panic!("not decoded");
    };
    assert_eq!((width, height), (64, 32));
    assert_eq!(rgba.len(), 64 * 32 * 4);
    assert_eq!(decode(b"not a png", 64), Outcome::Failed);
}
