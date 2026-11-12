// SPDX-License-Identifier: MIT

use strata_components::{
    contract::{EntryId, EntryKind, ListEntry, ListSource, ThumbKey},
    thumbnails::{Outcome, Settings, Step, ThumbnailStore, Viewport},
};
use strata_preview::sandbox::Cancellation;

use super::{Job, Kind, Pool, Queued, READERS, Start, Viewer, decode, made};
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

/// A failed render leaves the tile its icon: the store holds no image for the key and does
/// not ask for it again.
#[test]
fn a_failed_render_leaves_the_icon_and_is_not_asked_again() {
    let inbox = Inbox::new();
    let mut viewer = Viewer::new(inbox.later());
    let mut store = ThumbnailStore::new(Settings::default());
    let _ = viewer.take_requests(&mut store, 0.0);
    let id = viewer.id().expect("a viewer");
    store.sync(id, &photo(), Viewport::new(0..1, Step::new(64)));
    let ticket = viewer.take_requests(&mut store, 0.0)[0].ticket;
    viewer.pending.insert(ticket, Cancellation::default());

    let outcome = made(Err("Browser input changed while rendering".into()), 64);
    assert_eq!(outcome, Outcome::Failed);
    assert!(viewer.finish(&mut store, (ticket, outcome), 0.0));

    let stats = store.stats();
    assert_eq!((stats.ready, stats.failed), (0, 1));
    assert!(viewer.take_requests(&mut store, 1.0).is_empty());
}

fn queued(kind: Kind) -> Queued {
    Queued {
        job: Job {
            path: "file".into(),
            edge: 64,
            cancellation: Cancellation::default(),
            done: Box::new(|_| {}),
        },
        kind,
        modified: None,
    }
}

/// What the pool starts next.
#[derive(Debug, PartialEq)]
enum Started {
    Read,
    Render(Kind),
}

struct Case {
    name: &'static str,
    limit: usize,
    /// Renders running.
    running: Vec<Kind>,
    /// Cache reads running, and waiting.
    reading: usize,
    reads: usize,
    /// Renders waiting, in order.
    renders: Vec<Kind>,
    room: usize,
    starts: Option<Started>,
}

impl Case {
    /// A pool with nothing running or waiting.
    fn idle(name: &'static str, limit: usize, room: usize, starts: Option<Started>) -> Self {
        Self {
            name,
            limit,
            running: vec![],
            reading: 0,
            reads: 0,
            renders: vec![],
            room,
            starts,
        }
    }
}

fn pool_cases() -> Vec<Case> {
    use Kind::{Image, Pdf, RenderedModel};
    let render = |kind| Some(Started::Render(kind));
    vec![
        Case {
            running: vec![Image, Image],
            renders: vec![Image],
            ..Case::idle("every render slot busy", 2, READERS, None)
        },
        Case {
            running: vec![Image],
            renders: vec![Image],
            ..Case::idle("a free render slot", 2, READERS, render(Image))
        },
        Case {
            running: vec![Pdf, Pdf],
            renders: vec![Pdf, Image],
            ..Case::idle(
                "a slot kept from the heavy kinds",
                3,
                READERS,
                render(Image),
            )
        },
        Case {
            renders: vec![Pdf],
            ..Case::idle("one worker takes a heavy kind", 1, READERS, render(Pdf))
        },
        Case {
            running: vec![RenderedModel],
            renders: vec![RenderedModel, Image],
            ..Case::idle("one STL at a time", 4, READERS, render(Image))
        },
        Case {
            running: vec![Image],
            reads: 1,
            renders: vec![Image],
            ..Case::idle(
                "a cache read, every render slot busy",
                1,
                READERS - 1,
                Some(Started::Read),
            )
        },
        Case {
            reads: 1,
            renders: vec![Image],
            ..Case::idle(
                "a cache read before a render",
                2,
                READERS - 1,
                Some(Started::Read),
            )
        },
        Case {
            reading: READERS,
            reads: 1,
            ..Case::idle("every reader busy", 2, 0, None)
        },
        Case {
            running: vec![Image],
            renders: vec![Image; 64],
            ..Case::idle("the queue full", 1, 0, None)
        },
    ]
}

/// The pool's limits, as GTK's `jobs.rs` sets them.
#[test]
fn the_pool_starts_what_its_limits_allow() {
    for case in pool_cases() {
        let mut pool = Pool::default();
        for kind in &case.running {
            pool.count(*kind, true);
        }
        pool.reading = case.reading;
        pool.reads
            .extend((0..case.reads).map(|_| queued(Kind::Image)));
        pool.renders
            .extend(case.renders.iter().map(|kind| queued(*kind)));

        assert_eq!(pool.room(case.limit), case.room, "{}: room", case.name);
        let started = pool.next(case.limit).map(|start| match start {
            Start::Read(_) => Started::Read,
            Start::Render(queued) => Started::Render(queued.kind),
        });
        assert_eq!(started, case.starts, "{}", case.name);
    }
}
