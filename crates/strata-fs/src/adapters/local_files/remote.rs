// SPDX-License-Identifier: MIT
//! Listing GIO (non-native) folders through their remote mount lane.

use super::*;

/// One remote directory load in its admitted lane.
struct RemoteLoad<'a> {
    request: &'a DirectoryRequest,
    location: &'a Location,
    started: Instant,
    deadline: Instant,
    emit: &'a Rc<dyn Fn(DirectoryEvent)>,
    admitted: &'a mount_lanes::Admitted,
    directory: gio::File,
    can_trash: Option<bool>,
    can_delete: Option<bool>,
    total_entries: usize,
    names_digest: u64,
    first_batch: bool,
}

impl RemoteLoad<'_> {
    fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    fn finish_truncated(&self, reason: &'static str) {
        let request_id = self.request.id;
        tracing::warn!(
            request_id = request_id.0,
            entries = self.total_entries,
            elapsed_ms = self.elapsed_ms(),
            reason,
            "directory load truncated"
        );
        (self.emit)(DirectoryEvent::Finished {
            request_id,
            truncated: true,
            can_trash: self.can_trash,
            can_delete: self.can_delete,
        });
    }

    fn report_failure(&self, error: &glib::Error, message: &'static str) {
        tracing::warn!(
            request_id = self.request.id.0,
            error_domain = ?error.domain(),
            error_code = error.code(),
            "{message}"
        );
        self.admitted
            .progress()
            .answered(&Err::<(), _>(error.clone()));
        (self.emit)(remote_directory_failure(self.request.id, error));
    }

    /// Emits the folder's change stamp; `Break` once the time budget ran out.
    async fn stamp(&self) -> ControlFlow<()> {
        if !stamped_remotely(self.location) {
            return ControlFlow::Continue(());
        }
        let read_at = SystemTime::now();
        let queried = self
            .admitted
            .call(glib::future_with_timeout(
                self.remaining(),
                self.directory.query_info_future(
                    STAMP_ATTRIBUTES,
                    gio::FileQueryInfoFlags::NONE,
                    glib::Priority::LOW,
                ),
            ))
            .await;
        match queried {
            Ok(Ok(info)) => {
                self.admitted.progress().beat();
                (self.emit)(DirectoryEvent::Stamp {
                    request_id: self.request.id,
                    stamp: Box::new(remote_stamp(
                        self.location.uri_value().unwrap_or_default(),
                        &info,
                        read_at,
                    )),
                });
            }
            Ok(Err(error)) => self.admitted.progress().answered(&Err::<(), _>(error)),
            Err(_) => return ControlFlow::Break(()),
        }
        ControlFlow::Continue(())
    }

    /// Opens the enumeration; `None` once the load has ended.
    async fn open(&self) -> Option<gio::FileEnumerator> {
        let attributes = if self.request.include_metadata {
            FULL_ATTRIBUTES
        } else {
            LIST_ATTRIBUTES
        };
        let opened = self
            .admitted
            .call(glib::future_with_timeout(
                self.remaining(),
                self.directory.enumerate_children_future(
                    attributes,
                    gio::FileQueryInfoFlags::NONE,
                    glib::Priority::DEFAULT,
                ),
            ))
            .await;
        match opened {
            Ok(Ok(enumerator)) => {
                self.admitted.progress().beat();
                Some(enumerator)
            }
            Ok(Err(error)) => {
                self.report_failure(&error, "directory load failed");
                None
            }
            Err(_) => {
                self.finish_truncated("time budget");
                None
            }
        }
    }

    fn finish(&self) {
        let request_id = self.request.id;
        tracing::info!(
            request_id = request_id.0,
            entries = self.total_entries,
            elapsed_ms = self.elapsed_ms(),
            "directory load finished"
        );
        (self.emit)(DirectoryEvent::ListingDigest {
            request_id,
            names_digest: self.names_digest,
        });
        (self.emit)(DirectoryEvent::Finished {
            request_id,
            truncated: false,
            can_trash: self.can_trash,
            can_delete: self.can_delete,
        });
    }

    fn learn_capabilities(&mut self, files: &[gio::FileInfo]) {
        if self.can_trash.is_none() {
            self.can_trash = files.iter().find_map(info_can_trash);
        }
        if self.can_delete.is_none() {
            self.can_delete = files.iter().find_map(info_can_delete);
        }
    }

    /// Emits one batch; `Break` once the entry budget is exhausted.
    fn batch(&mut self, files: Vec<gio::FileInfo>) -> ControlFlow<()> {
        self.admitted.progress().beat();
        self.learn_capabilities(&files);
        let request_id = self.request.id;
        let mut entries: Vec<_> = files
            .into_iter()
            .filter_map(|info| listed_entry(request_id, self.location, &self.directory, info))
            .collect();
        let remaining_capacity = self.request.max_entries.saturating_sub(self.total_entries);
        let entry_budget_exhausted = entries.len() > remaining_capacity;
        entries.truncate(remaining_capacity);
        self.total_entries += entries.len();
        self.names_digest = entries.iter().fold(self.names_digest, |digest, entry| {
            digest.wrapping_add(name_hash(&entry.native_name))
        });
        if self.first_batch {
            tracing::info!(
                request_id = request_id.0,
                entries = entries.len(),
                elapsed_ms = self.elapsed_ms(),
                "first directory batch ready"
            );
            self.first_batch = false;
        }
        (self.emit)(DirectoryEvent::Batch {
            request_id,
            entries,
        });
        if entry_budget_exhausted {
            self.finish_truncated("entry budget");
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    }

    async fn next_batch(&mut self, enumerator: &gio::FileEnumerator) -> ControlFlow<()> {
        let remaining = self.remaining();
        if remaining.is_zero() {
            self.finish_truncated("time budget");
            return ControlFlow::Break(());
        }
        let next = self
            .admitted
            .call(glib::future_with_timeout(
                remaining,
                enumerator
                    .next_files_future(self.request.batch_size as i32, glib::Priority::DEFAULT),
            ))
            .await;
        match next {
            Ok(Ok(files)) if files.is_empty() => {
                self.admitted.progress().beat();
                self.finish();
                ControlFlow::Break(())
            }
            Ok(Ok(files)) => self.batch(files),
            Ok(Err(error)) => {
                self.report_failure(&error, "directory load interrupted");
                ControlFlow::Break(())
            }
            Err(_) => {
                self.finish_truncated("time budget");
                ControlFlow::Break(())
            }
        }
    }

    async fn run(&mut self) {
        if self.remaining().is_zero() {
            self.finish_truncated("time budget");
            return;
        }
        if self.stamp().await.is_break() {
            self.finish_truncated("time budget");
            return;
        }
        let Some(enumerator) = self.open().await else {
            return;
        };
        while self.next_batch(&enumerator).await.is_continue() {}
    }
}

pub(super) fn enumerate_remote(
    request: DirectoryRequest,
    emit: Rc<dyn Fn(DirectoryEvent)>,
    started: Instant,
) -> LoadHandle {
    let request_id = request.id;
    let task = glib::MainContext::default().spawn_local(async move {
        let location = request.location.clone();
        let class = match request.background {
            true => Class::Background,
            false => Class::Interactive,
        };
        let admitted = mount_lanes::admit_remote(
            mount_lanes::authority(location.uri_value().unwrap_or_default()),
            class,
        );
        let Ok(admitted) = admitted else {
            emit(DirectoryEvent::Failed {
                request_id,
                message: crate::services::io_error_message(&ErrorKind::TimedOut.into()),
                refused: true,
                denied: false,
            });
            return;
        };
        let mut load = RemoteLoad {
            request: &request,
            location: &location,
            started,
            deadline: started + request.time_budget,
            emit: &emit,
            admitted: &admitted,
            directory: gio_file_for_location(&location),
            // GVfs's admin backend has no trash operation but reports `access::can-trash`.
            can_trash: location.is_administrator().then_some(false),
            can_delete: None,
            total_entries: 0,
            names_digest: 0,
            first_batch: true,
        };
        load.run().await;
    });

    LoadHandle::new(move || {
        tracing::debug!(request_id = request_id.0, "directory load cancelled");
        task.abort();
    })
}
