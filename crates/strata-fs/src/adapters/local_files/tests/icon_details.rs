// SPDX-License-Identifier: MIT

use std::{process::Command, sync::mpsc};

use super::*;

const COMPLETE: Option<MetadataOutcome> = Some(MetadataOutcome::Complete);

const ONE_PIXEL_PNG: [u8; 69] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
    0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x08, 0xd7, 0x63, 0xf8, 0xff, 0xff, 0x3f,
    0x00, 0x05, 0xfe, 0x02, 0xfe, 0xdc, 0xcc, 0x59, 0xe7, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e,
    0x44, 0xae, 0x42, 0x60, 0x82,
];

/// Writes a `seconds`-long lossless test clip to `path`, replacing it when `overwrite` is set.
fn encode_clip(path: &Path, seconds: u32, overwrite: bool) {
    let mut ffmpeg = Command::new("ffmpeg");
    ffmpeg.args(["-nostdin", "-v", "error"]);
    if overwrite {
        ffmpeg.arg("-y");
    }
    let status = ffmpeg
        .args(["-f", "lavfi", "-i"])
        .arg(format!("testsrc2=size=16x16:rate=1:duration={seconds}"))
        .args(["-c:v", "ffv1", "-threads", "1"])
        .arg(path)
        .status()
        .expect("ffmpeg should write the media fixture");
    assert!(status.success());
}

fn filled_duration(events: &[DirectoryEvent]) -> Option<MetadataValue<u64>> {
    last_filled(events, |update| update.duration_seconds.clone())
}

fn revisited_durations(root: &FixtureDir) -> Vec<MetadataValue<u64>> {
    batched_entries(&run_enumerate(DirectoryRequest {
        id: RequestId(2),
        ..directory_request(root.location())
    }))
    .into_iter()
    .map(|entry| entry.duration_seconds)
    .collect()
}

/// Counts the cheap and detailed updates of a fill whose detail probes wait for permission.
struct DetailProbeRelease {
    cheap: Cell<usize>,
    details: Cell<usize>,
    release: mpsc::Sender<()>,
}

impl DetailProbeRelease {
    fn observe(&self, update: &MetadataUpdate) {
        assert_eq!(update.size, MetadataValue::Known(7));
        assert!(matches!(
            update.modified_unix_seconds,
            MetadataValue::Known(_)
        ));
        assert!(matches!(update.mode, MetadataValue::Known(_)));
        if update.image_dimensions == MetadataValue::Unknown {
            self.saw_cheap();
        } else {
            self.saw_details(update);
        }
    }

    fn saw_cheap(&self) {
        self.cheap.set(self.cheap.get() + 1);
        if self.cheap.get() == 2 {
            self.release.send(()).expect("release first probe");
        }
    }

    fn saw_details(&self, update: &MetadataUpdate) {
        assert_eq!(self.cheap.get(), 2);
        assert_eq!(update.image_dimensions, MetadataValue::Known((20, 30)));
        self.details.set(self.details.get() + 1);
        if self.details.get() == 1 {
            self.release.send(()).expect("release second probe");
        }
    }
}

#[test]
fn icon_details_cache_bounds_revisit_history_and_preserves_lru() {
    let mut cache = IconDetailsCache::default();
    let fingerprint = IconDetailsFingerprint {
        size: 1,
        modified_seconds: 1,
        modified_nanoseconds: 0,
        changed_seconds: 1,
        changed_nanoseconds: 0,
    };
    let details = IconDetails {
        image_dimensions: MetadataValue::Known((1, 1)),
        child_count: MetadataValue::Unavailable,
        duration_seconds: MetadataValue::Unavailable,
    };
    for index in 0..MAX_ICON_DETAILS_CACHE_ENTRIES {
        cache.insert(
            PathBuf::from(index.to_string()),
            fingerprint,
            details.clone(),
        );
    }
    let revisited = Path::new("0");
    for _ in 0..MAX_ICON_DETAILS_CACHE_ENTRIES * 5 {
        assert!(cache.get(revisited, fingerprint).is_some());
    }
    assert!(cache.recent.len() <= MAX_ICON_DETAILS_CACHE_ENTRIES * 4);
    cache.insert(PathBuf::from("new"), fingerprint, details);
    assert!(cache.get(revisited, fingerprint).is_some());
    assert!(cache.get(Path::new("1"), fingerprint).is_none());
    assert_eq!(cache.entries.len(), MAX_ICON_DETAILS_CACHE_ENTRIES);
}

#[test]
fn fill_image_file_extracts_dimensions() {
    let root = FixtureDir::new("fill-image-dimensions");
    let path = root.write("pixel.png", ONE_PIXEL_PNG);

    let events = run_fill(icon_details_request(1, &path));

    assert_eq!(fill_outcome(&events), COMPLETE);
    assert_eq!(
        last_filled(&events, |update| update.image_dimensions.clone()),
        Some(MetadataValue::Known((1, 1)))
    );
    root.remove();
}

#[test]
fn fill_skips_media_probes_for_text_named_like_media() {
    let root = FixtureDir::new("fill-text-named-like-media");
    let path = root.write("clip.mp4", "export const clip = 1;\n");

    let events = run_fill(icon_details_request(1, &path));

    assert_eq!(
        (fill_outcome(&events), filled_duration(&events)),
        (COMPLETE, Some(MetadataValue::Unavailable))
    );
    assert_eq!(media_metadata_probe_count(&path), 0);
    root.remove();
}

#[test]
fn fill_media_file_caches_duration_for_revisits() {
    let root = FixtureDir::new("fill-media-duration");
    let path = root.path().join("clip.mkv");
    encode_clip(&path, 2, false);

    let first = run_fill(icon_details_request(1, &path));
    assert_eq!(
        (fill_outcome(&first), filled_duration(&first)),
        (COMPLETE, Some(MetadataValue::Known(2)))
    );
    assert_eq!(media_metadata_probe_count(&path), 1);

    assert_eq!(revisited_durations(&root), [MetadataValue::Known(2)]);

    let revisit = run_fill(icon_details_request(2, &path));
    assert_eq!(
        (fill_outcome(&revisit), filled_duration(&revisit)),
        (COMPLETE, Some(MetadataValue::Known(2)))
    );
    assert_eq!(media_metadata_probe_count(&path), 1);

    encode_clip(&path, 3, true);
    let changed = run_fill(icon_details_request(3, &path));
    assert_eq!(filled_duration(&changed), Some(MetadataValue::Known(3)));
    assert_eq!(media_metadata_probe_count(&path), 2);
    root.remove();
}

#[test]
fn fill_directory_child_count_extracts_item_count() -> Result<(), Box<dyn Error>> {
    let root = FixtureDir::new("fill-dir-count");
    root.write("file-1.txt", b"1");
    root.write("file-2.txt", b"2");
    fs::create_dir_all(root.path().join("subdir")).expect("subdir");
    let alias = root.path().with_extension("alias");
    std::os::unix::fs::symlink(root.path(), &alias).expect("alias");
    let fill = |id, folder: &Path| {
        let events = run_fill(icon_details_request(id, folder));
        assert_eq!(fill_outcome(&events), COMPLETE);
        last_filled(&events, |update| update.child_count.clone())
    };
    let activity = file_activity::begin(file_activity::OperationScope::default());

    let real = root.path().canonicalize()?;

    for (row, folder) in [
        ("as named", root.path()),
        ("through a symbolic link", alias.as_path()),
    ] {
        let fence = activity.fence([real.clone()]);
        assert_eq!(
            fill(1, folder),
            Some(MetadataValue::Unknown),
            "{row}: a folder an operation fences is not listed"
        );
        drop(fence);
        assert_eq!(fill(2, folder), Some(MetadataValue::Known(3)), "{row}");
    }
    fs::remove_file(&alias).expect("the alias should be removed");
    root.remove();
    Ok(())
}

#[test]
fn cheap_metadata_is_published_while_details_are_waiting() {
    let _serial = serial();
    let root = tempfile::tempdir().expect("fixture directory");
    let path = root.path().join("photo.jpg");
    fs::write(&path, b"content").expect("first fixture");
    let second = root.path().join("second.jpg");
    fs::write(&second, b"content").expect("second fixture");
    let (release, wait) = mpsc::channel();
    let wait = Mutex::new(wait);
    glib::MainContext::default().block_on(async move {
        let (finished, done) = futures_channel::oneshot::channel();
        let finished = RefCell::new(Some(finished));
        let probes = Rc::new(DetailProbeRelease {
            cheap: Cell::new(0),
            details: Cell::new(0),
            release,
        });
        let observed = probes.clone();
        let emit = Rc::new(move |event| match event {
            DirectoryEvent::MetadataFilled { updates, .. } => {
                for update in &updates {
                    observed.observe(update);
                }
            }
            DirectoryEvent::MetadataFinished { outcome, .. } => {
                assert_eq!(outcome, MetadataOutcome::Complete);
                finished
                    .borrow_mut()
                    .take()
                    .expect("one completion")
                    .send(())
                    .expect("completion receiver");
            }
            _ => {}
        });
        let handle = fill_parallel_with_details(
            1,
            RequestId(1),
            vec![Location::local(path), Location::local(second)],
            true,
            Duration::from_secs(10),
            emit,
            move |update, _, _, _, _| {
                wait.lock()
                    .expect("probe receiver")
                    .recv_timeout(Duration::from_secs(5))
                    .expect("preceding metadata published before probe finishes");
                update.image_dimensions = MetadataValue::Known((20, 30));
                true
            },
        );
        done.await.expect("metadata completion");
        assert_eq!((probes.cheap.get(), probes.details.get()), (2, 2));
        drop(handle);
    });
}

#[test]
fn cancelled_or_withheld_media_details_are_not_cached_as_unavailable() {
    let root = tempfile::tempdir().expect("fixture directory");
    let path = root.path().join("photo.jpg");
    fs::write(&path, b"\xff\xd8\xff\xe0\0\x10JFIF\0").expect("fixture file");
    let location = Location::local(&path);
    let info = gio::File::for_path(&path)
        .query_info(
            METADATA_ATTRIBUTES,
            gio::FileQueryInfoFlags::NONE,
            None::<&gio::Cancellable>,
        )
        .expect("source metadata");
    for (row, cancels, error, filled) in [
        ("cancelled", true, "cancelled", false),
        (
            "withheld by an operation",
            false,
            file_activity::HELD_FOR_OPERATION,
            true,
        ),
    ] {
        let cancellable = gio::Cancellable::new();
        let (mut update, _) = update_from_info(&info, &location);
        let probed = fill_icon_details_with_probe(
            &mut update,
            &info,
            &location,
            &cancellable,
            Instant::now() + Duration::from_secs(10),
            |_, _, cancellation| {
                if cancels {
                    cancellation.cancel();
                }
                Err(error.to_owned())
            },
        );

        assert_eq!(
            (
                probed,
                &update.image_dimensions,
                &update.duration_seconds,
                cached_icon_details_for_revisit(&path).is_none()
            ),
            (
                filled,
                &MetadataValue::Unknown,
                &MetadataValue::Unknown,
                true
            ),
            "{row}"
        );
    }
}
