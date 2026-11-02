// SPDX-License-Identifier: MIT

//! Whether local files named as audio or video hold media. A name alone decides nothing:
//! the preview reads a file's leading bytes before choosing a player, so a media sequence
//! that trusted names would count, and land on, files that preview as text.

use std::{cell::RefCell, collections::HashMap, path::PathBuf};

use gtk::{gio, glib};

use super::MediaFamily;
use crate::model::{FileEntry, MetadataValue};

/// Verdicts outlive folders so revisits stay instant; the cache restarts past this size.
const MAX_REMEMBERED: usize = 8192;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Fingerprint {
    size: MetadataValue<u64>,
    modified: MetadataValue<i64>,
}

impl Fingerprint {
    fn of(entry: &FileEntry) -> Self {
        Self {
            size: entry.size.clone(),
            modified: entry.modified_unix_seconds.clone(),
        }
    }

    /// Listings learn sizes and dates late, so only two known values that differ
    /// mean the file changed.
    fn same_file(&self, other: &Self) -> bool {
        fn agree<T: PartialEq>(left: &MetadataValue<T>, right: &MetadataValue<T>) -> bool {
            match (left, right) {
                (MetadataValue::Known(left), MetadataValue::Known(right)) => left == right,
                _ => true,
            }
        }
        agree(&self.size, &other.size) && agree(&self.modified, &other.modified)
    }

    fn learn(&mut self, other: &Self) {
        if matches!(other.size, MetadataValue::Known(_)) {
            self.size = other.size.clone();
        }
        if matches!(other.modified, MetadataValue::Known(_)) {
            self.modified = other.modified.clone();
        }
    }
}

struct Check {
    fingerprint: Fingerprint,
    /// The last answer, kept while a changed file is read again.
    media: Option<bool>,
    reading: bool,
}

thread_local! {
    static CHECKS: RefCell<HashMap<PathBuf, Check>> = RefCell::new(HashMap::new());
}

/// Local files whose bytes a media sequence still has to read.
#[derive(Default)]
pub(super) struct PendingReads(Vec<(PathBuf, Fingerprint)>);

/// Whether `entry`, already named as `family`, belongs in its media sequence. Unread
/// local files stay out and are queued on `pending` until their bytes answer.
pub(super) fn holds_media(
    entry: &FileEntry,
    family: MediaFamily,
    pending: &mut PendingReads,
) -> bool {
    if family == MediaFamily::Route || probed_media(entry, family) {
        return true;
    }
    let Some(path) = entry.location.native_path() else {
        return true;
    };
    let fingerprint = Fingerprint::of(entry);
    CHECKS.with_borrow_mut(|checks| {
        if let Some(check) = checks.get_mut(path) {
            if check.fingerprint.same_file(&fingerprint) {
                check.fingerprint.learn(&fingerprint);
            } else if !check.reading {
                check.fingerprint = fingerprint.clone();
                check.reading = true;
                pending.0.push((path.to_path_buf(), fingerprint));
            }
            return check.media == Some(true);
        }
        if checks.len() >= MAX_REMEMBERED {
            checks.clear();
        }
        checks.insert(
            path.to_path_buf(),
            Check {
                fingerprint: fingerprint.clone(),
                media: None,
                reading: true,
            },
        );
        pending.0.push((path.to_path_buf(), fingerprint));
        false
    })
}

/// A metadata probe only runs on bytes that already back the media name.
fn probed_media(entry: &FileEntry, family: MediaFamily) -> bool {
    matches!(entry.duration_seconds, MetadataValue::Known(_))
        || (family == MediaFamily::Video
            && matches!(entry.image_dimensions, MetadataValue::Known(_)))
}

impl PendingReads {
    /// Reads the queued files off the GTK thread, then calls `done` on it.
    pub(super) fn start(self, done: impl FnOnce() + 'static) {
        if self.0.is_empty() {
            return;
        }
        glib::MainContext::default().spawn_local(async move {
            let paths: Vec<PathBuf> = self.0.iter().map(|(path, _)| path.clone()).collect();
            let answers = gio::spawn_blocking(move || {
                paths
                    .iter()
                    .map(|path| crate::services::bytes_support_named_family(path))
                    .collect::<Vec<_>>()
            })
            .await
            .unwrap_or_default();
            CHECKS.with_borrow_mut(|checks| {
                for (index, (path, fingerprint)) in self.0.iter().enumerate() {
                    let Some(check) = checks
                        .get_mut(path)
                        .filter(|check| check.reading && check.fingerprint.same_file(fingerprint))
                    else {
                        continue;
                    };
                    check.reading = false;
                    // Like an unreadable file, a failed job leaves the name to decide.
                    check.media = Some(answers.get(index).copied().unwrap_or(true));
                }
            });
            done();
        });
    }
}
