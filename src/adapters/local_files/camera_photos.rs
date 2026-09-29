// SPDX-License-Identifier: MIT

use std::collections::{HashSet, VecDeque};

use super::*;

pub(super) fn enumerate(request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
    let task = glib::MainContext::default().spawn_local_with_priority(
        glib::Priority::DEFAULT_IDLE,
        async move {
            let mut library = Library::new(&request);
            let result = if request.time_budget == Duration::MAX {
                Ok(library.discover(&request, &emit).await)
            } else {
                glib::future_with_timeout(request.time_budget, library.discover(&request, &emit))
                    .await
            };
            match result {
                Ok(Err(message)) => emit(DirectoryEvent::Failed {
                    request_id: request.id,
                    message,
                }),
                result => emit(DirectoryEvent::Finished {
                    request_id: request.id,
                    truncated: !matches!(result, Ok(Ok(()))) || library.truncated,
                    can_trash: library.can_trash,
                    can_delete: library.can_delete,
                }),
            }
        },
    );
    LoadHandle::new(move || task.abort())
}

struct Library {
    pending: VecDeque<(Location, bool)>,
    seen: HashSet<Location>,
    files: usize,
    truncated: bool,
    can_trash: Option<bool>,
    can_delete: Option<bool>,
}

impl Library {
    fn new(request: &DirectoryRequest) -> Self {
        Self {
            pending: VecDeque::from([(request.location.clone(), false)]),
            seen: HashSet::from([request.location.clone()]),
            files: 0,
            truncated: false,
            can_trash: None,
            can_delete: None,
        }
    }

    async fn discover(
        &mut self,
        request: &DirectoryRequest,
        emit: &Rc<dyn Fn(DirectoryEvent)>,
    ) -> Result<(), String> {
        let attributes = if request.include_metadata {
            FULL_ATTRIBUTES
        } else {
            LIST_ATTRIBUTES
        };
        let batch_size = request.batch_size.clamp(1, 256) as i32;
        while let Some((location, hidden_parent)) = self.pending.pop_front() {
            let pending_before = self.pending.len();
            let enumerator = gio_file_for_location(&location)
                .enumerate_children_future(
                    attributes,
                    gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                    glib::Priority::DEFAULT_IDLE,
                )
                .await
                .map_err(|error| error.to_string())?;
            loop {
                let infos = enumerator
                    .next_files_future(batch_size, glib::Priority::DEFAULT_IDLE)
                    .await
                    .map_err(|error| error.to_string())?;
                if infos.is_empty() {
                    break;
                }
                let mut entries = Vec::new();
                for info in infos {
                    let Some(child) = location.child(info.name().as_os_str()) else {
                        continue;
                    };
                    if !(child.is_within(&request.location)
                        || request.location.contains_camera_photo_location(&child))
                        || info_is_symlink(&info)
                    {
                        continue;
                    }
                    match info.file_type() {
                        gio::FileType::Directory => {
                            if self.seen.insert(child.clone()) {
                                self.pending
                                    .push_back((child, hidden_parent || info_is_hidden(&info)));
                            }
                        }
                        gio::FileType::Regular => {
                            if !is_photo_media(info.name().as_os_str()) {
                                continue;
                            }
                            if self.files == request.max_entries {
                                self.truncated = true;
                                break;
                            }
                            if self.can_trash.is_none() {
                                self.can_trash = info_can_trash(&info);
                            }
                            if self.can_delete.is_none() {
                                self.can_delete = info_can_delete(&info);
                            }
                            // Each row retains its real URI, including duplicate basenames.
                            let mut entry = entry_from_info(child, info);
                            entry.is_hidden |= hidden_parent;
                            entries.push(entry);
                            self.files += 1;
                        }
                        _ => {}
                    }
                }
                if !entries.is_empty() {
                    emit(DirectoryEvent::Batch {
                        request_id: request.id,
                        entries,
                    });
                    if request.time_budget == Duration::MAX {
                        crate::services::camera_preview::yield_after_batch(&request.location).await;
                    }
                }
                if self.truncated && self.files == request.max_entries {
                    break;
                }
            }
            enumerator
                .close_future(glib::Priority::DEFAULT_IDLE)
                .await
                .map_err(|error| error.to_string())?;
            if self.truncated && self.files == request.max_entries {
                return Ok(());
            }
            if self.pending.len() > pending_before {
                // Camera date-folder names sort chronologically; visit recent branches first.
                self.pending
                    .make_contiguous()
                    .sort_by(|left, right| right.0.compare(&left.0));
            }
        }
        Ok(())
    }
}

fn is_photo_media(name: &std::ffi::OsStr) -> bool {
    Path::new(name)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "jpg"
                    | "jpeg"
                    | "heic"
                    | "heif"
                    | "mov"
                    | "mp4"
                    | "3fr"
                    | "arw"
                    | "cr2"
                    | "cr3"
                    | "dcr"
                    | "dng"
                    | "erf"
                    | "kdc"
                    | "mef"
                    | "mos"
                    | "mrw"
                    | "nef"
                    | "nrw"
                    | "orf"
                    | "pef"
                    | "raf"
                    | "raw"
                    | "rw2"
                    | "rwl"
                    | "sr2"
                    | "srf"
                    | "srw"
                    | "x3f"
            )
        })
}

#[cfg(test)]
mod tests;
