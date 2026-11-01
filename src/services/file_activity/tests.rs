// SPDX-License-Identifier: MIT

use std::{
    io::Read,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use super::*;
use crate::{
    sandbox::{self, Cancellation, MediaPreviewBackend, ParseOperation},
    services::{ArchiveFormat, DocumentMedia, document_media, read_head_sample},
};

const LONG: Duration = Duration::from_secs(10);

fn released(fence: &Fence) -> bool {
    futures_lite::future::block_on(futures_lite::future::poll_once(fence.drained())).is_some()
}

#[test]
fn a_release_stops_and_waits_for_the_reads_under_its_roots_only() {
    let root = tempfile::tempdir().expect("root folder");
    let item = root.path().join("item");
    let sibling = root.path().join("item 2");
    let (stopped, stop_seen) = mpsc::channel();
    let below = hold([item.join("inside")], move || {
        let _ = stopped.send(());
    })
    .expect("a read below the root");
    let sibling_stops = Arc::new(AtomicUsize::new(0));
    let counted = sibling_stops.clone();
    let beside = hold([&sibling], move || {
        counted.fetch_add(1, Ordering::SeqCst);
    })
    .expect("a read beside the root");
    let ended = Arc::new(AtomicBool::new(false));
    let reader = {
        let ended = ended.clone();
        thread::spawn(move || {
            stop_seen.recv_timeout(LONG).expect("the read is stopped");
            assert_eq!(
                below.settle(Err::<(), _>("Browser request cancelled".to_owned())),
                Err(HELD_FOR_OPERATION.to_owned()),
                "a stopped read reports the operation, not a failure"
            );
            thread::sleep(Duration::from_millis(30));
            ended.store(true, Ordering::SeqCst);
            drop(below);
        })
    };
    let activity = begin(OperationScope::default());

    let fence = activity.fence([item.clone()]);
    fence.stop_reads();
    futures_lite::future::block_on(fence.drained());

    assert!(ended.load(Ordering::SeqCst));
    reader.join().expect("reader thread");
    assert_eq!(sibling_stops.load(Ordering::SeqCst), 0);
    let other = activity.fence([sibling.clone()]);
    assert!(!released(&other), "the read beside the root stays open");
    assert_eq!(
        beside.settle(Err::<(), _>("Unreadable".to_owned())),
        Err("Unreadable".to_owned()),
    );
    drop(beside);
}

#[test]
fn a_read_is_refused_while_any_fence_covers_it() {
    let root = tempfile::tempdir().expect("root folder");
    let elsewhere = tempfile::tempdir().expect("another folder");
    let item = root.path().join("item");
    let below = item.join("child");
    let next_to = root.path().join("other");
    let opens = |path: &Path| hold([path], || {}).is_ok();
    let activity = begin(OperationScope::default());

    let outer = activity.fence([root.path().to_owned()]);
    let inner = activity.fence([item.clone()]);
    assert!(!opens(&below));
    assert!(!opens(&next_to));
    assert!(opens(elsewhere.path()));
    assert!(
        hold([elsewhere.path(), &below], || {}).is_err(),
        "any of its names is fenced"
    );

    drop(outer);
    assert!(!opens(&below), "the fence on the item still holds");
    assert!(opens(&next_to));

    let again = activity.fence([item.clone()]);
    drop(inner);
    assert!(
        !opens(&below),
        "a second fence on the same item still holds"
    );
    drop(again);
    assert!(opens(&below));
}

#[test]
fn a_read_through_a_symbolic_link_matches_where_it_really_is() {
    let root = tempfile::tempdir().expect("root folder");
    let real = root.path().canonicalize().expect("root path").join("real");
    std::fs::create_dir(&real).expect("real folder");
    std::fs::write(real.join("song.flac"), b"audio").expect("file");
    let link = root.path().join("link");
    std::os::unix::fs::symlink(&real, &link).expect("symbolic link");
    let song = link.join("song.flac");
    let activity = begin(OperationScope::default());

    let mut file = open(&song).expect("a file read through the link");
    let mut listing = list(&link).expect("a folder listed through the link");
    let fence = activity.fence([real.clone()]);
    assert!(!released(&fence));
    fence.stop_reads();
    let read = file.read(&mut [0; 8]).expect_err("a stopped file read");
    let listed = listing
        .next()
        .and_then(Result::err)
        .expect("a stopped listing");
    assert!(listing.next().is_none(), "a stopped listing ends");
    for (reader, error) in [("file", read), ("folder listing", listed)] {
        assert!(is_refusal(&error), "{reader}: {error}");
    }
    drop(file);
    assert!(!released(&fence), "the listing has not ended");
    drop(listing);
    assert!(released(&fence));

    for (reader, refused) in [
        ("file", open(&song).err()),
        ("folder listing", list(&link).err()),
    ] {
        let refused = refused.unwrap_or_else(|| panic!("a new {reader} through the link"));
        assert_eq!(refused.to_string(), HELD_FOR_OPERATION, "{reader}");
    }
}

#[test]
fn every_read_funnel_refuses_a_file_an_operation_fences_under_either_name() {
    let root = tempfile::tempdir().expect("root folder");
    let real = root.path().canonicalize().expect("root path").join("real");
    std::fs::create_dir(&real).expect("real folder");
    std::fs::write(real.join("photo.png"), b"\x89PNG\r\n\x1a\n").expect("image");
    let link = root.path().join("link");
    std::os::unix::fs::symlink(&real, &link).expect("symbolic link");
    let photo = link.join("photo.png");
    let notes = link.join("notes.md");
    let cancellation = Cancellation::default();
    let refused = |error: Option<String>| error.as_deref() == Some(HELD_FOR_OPERATION);
    let parse = |operation| {
        sandbox::parse(
            &photo,
            operation,
            0,
            MediaPreviewBackend::Software,
            &cancellation,
        )
        .err()
    };
    let funnels: [(&str, &dyn Fn() -> bool); 5] = [
        ("content sniffing", &|| {
            read_head_sample(&photo).is_err_and(|error| is_refusal(&error))
        }),
        ("Markdown image", &|| {
            refused(
                document_media::render(
                    &DocumentMedia::Image("photo.png".into()),
                    Some(&notes),
                    &cancellation,
                )
                .err(),
            )
        }),
        ("image preview", &|| {
            refused(parse(ParseOperation::PreviewImage))
        }),
        ("archive listing", &|| {
            refused(parse(ParseOperation::ArchiveList {
                format: ArchiveFormat::Zip,
                password: None,
            }))
        }),
        ("pooled thumbnail", &|| {
            refused(sandbox::browser::metadata(&photo, true, &cancellation).err())
        }),
    ];
    assert!(read_head_sample(&photo).is_ok());
    let activity = begin(OperationScope::default());

    for fenced in [&real, &link] {
        let _fence = activity.fence([fenced.clone()]);
        for (funnel, is_refused) in &funnels {
            assert!(is_refused(), "{funnel} under {fenced:?}");
        }
    }
}

#[test]
fn running_operations_answer_for_their_roots_and_folders_until_they_end() {
    let root = tempfile::tempdir().expect("root folder");
    let folder = root.path().join("folder");
    let item = folder.join("item");
    let other = root.path().join("other");
    let set_aside = folder.join("original");

    let first = begin(OperationScope {
        roots: vec![item.clone()],
        folders: vec![folder.clone()],
    });
    assert!(is_changing(&item));
    assert!(is_changing(&item.join("below")));
    assert!(!is_changing(&folder));
    assert!(is_active_in(&folder));
    assert!(!is_active_in(&item));
    assert!(!is_active_in(root.path()));

    let second = begin(OperationScope {
        roots: Vec::new(),
        folders: vec![folder.clone()],
    });
    second.include(OperationScope {
        roots: vec![other.clone()],
        folders: Vec::new(),
    });
    let fence = second.fence([set_aside.clone()]);
    let merged = root.path().join("merged");
    let remote = Location::uri("sftp://host/share/dir");
    second.include_folder(&remote);
    second.include_trees([
        Location::local(&merged),
        Location::uri("sftp://host/share/merged"),
    ]);
    assert!(is_changing(&other));
    assert!(is_changing(&set_aside));
    let active = |folder: Location| is_active_at(&folder);
    let below_trees = [
        Location::local(&merged),
        Location::local(merged.join("sub/deep")),
        remote.clone(),
        Location::uri("sftp://host/share/merged"),
        Location::uri("sftp://host/share/merged/sub"),
    ];
    let elsewhere = [
        Location::local(root.path()),
        Location::local(root.path().join("merged-too")),
        Location::uri("sftp://host/share/dir/sub"),
        Location::uri("sftp://host/share"),
        Location::uri("sftp://other/share/dir"),
    ];
    assert!(below_trees.iter().cloned().all(active));
    assert!(!elsewhere.iter().cloned().any(active));

    drop(first);
    assert!(!is_changing(&item));
    assert!(is_active_in(&folder), "the second operation writes into it");

    drop(fence);
    assert!(!is_changing(&set_aside));
    drop(second);
    assert!(!is_active_in(&folder));
    assert!(!is_changing(&other));
    assert!(!below_trees.into_iter().any(active));
}
