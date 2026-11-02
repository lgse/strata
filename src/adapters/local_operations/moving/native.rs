// SPDX-License-Identifier: MIT

use super::*;
use std::os::fd::OwnedFd;

pub(in crate::adapters::local_operations) async fn rename_native(
    source: PathBuf,
    target: PathBuf,
    cancellable: &gio::Cancellable,
) -> Result<(), glib::Error> {
    let (Some(source_parent), Some(source_name), Some(target_parent), Some(target_name)) = (
        source.parent().map(Path::to_path_buf),
        source.file_name().map(OsStr::to_os_string),
        target.parent().map(Path::to_path_buf),
        target.file_name().map(OsStr::to_os_string),
    ) else {
        return Err(translated_io_error("Invalid move destination"));
    };
    let stopped = cancellable.clone();
    run_local_fs_step(move || {
        let source_dir = open_local_parent_directory(&source_parent)?;
        let target_dir = if target_parent == source_parent {
            None
        } else {
            Some(open_local_parent_directory(&target_parent)?)
        };
        let from = Entry::new(&source_dir, &source_name);
        let to = Entry::new(target_dir.as_ref().unwrap_or(&source_dir), &target_name);
        let Ok(renamed) = deferred_close::retry_folder_rename(
            fs(),
            from,
            &Deadline::default(),
            &|| stopped.is_cancelled(),
            |failure| {
                matches!(
                    failure,
                    RenameFailure::Place(PlaceError::Io(rustix::io::Errno::ACCESS))
                )
            },
            || placement::rename_in_place(fs(), from, to),
        ) else {
            return Ok(Err(cancelled_local_operation()));
        };
        Ok(renamed.map(drop).map_err(|failure| match failure {
            RenameFailure::Place(error) => still_open_rename(from, error)
                .map_or_else(|| rename_failure_error(failure, &source, &target), io_error),
            failure => rename_failure_error(failure, &source, &target),
        }))
    })
    .await?
}

pub(in crate::adapters::local_operations) fn move_local_path(
    source_path: PathBuf,
    target_path: PathBuf,
    taken: Taken,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<Moved, glib::Error>>>> {
    Box::pin(async move {
        if cancellable.is_cancelled() {
            return Err(cancelled_local_operation());
        }
        let paths = MovePaths::of(&source_path, &target_path)?;
        let display_name = paths.source_name.to_string_lossy().into_owned();
        let stopped = cancellable.clone();
        run_local_fs_step(move || paths.place(taken, &stopped, &display_name)).await?
    })
}

struct MovePaths {
    source_parent: PathBuf,
    source_name: OsString,
    target_parent: PathBuf,
    target_name: OsString,
}

impl MovePaths {
    fn of(source_path: &Path, target_path: &Path) -> Result<Self, glib::Error> {
        let Some(source_parent) = source_path.parent().map(Path::to_path_buf) else {
            return Err(translated_io_error("Cannot move the filesystem root"));
        };
        let Some(source_name) = source_path.file_name().map(OsStr::to_os_string) else {
            return Err(translated_io_error("Invalid move source"));
        };
        let Some(target_parent) = target_path.parent().map(Path::to_path_buf) else {
            return Err(translated_io_error(
                "The move destination has no parent directory",
            ));
        };
        let Some(target_name) = target_path.file_name().map(OsStr::to_os_string) else {
            return Err(translated_io_error("Invalid move destination"));
        };
        Ok(Self {
            source_parent,
            source_name,
            target_parent,
            target_name,
        })
    }

    fn place(
        &self,
        taken: Taken,
        stopped: &gio::Cancellable,
        display_name: &str,
    ) -> Result<Result<Moved, glib::Error>, String> {
        let source_parent = open_local_parent_directory(&self.source_parent)?;
        let target_parent = open_local_parent_directory(&self.target_parent)?;
        let from = Entry::new(&source_parent, &self.source_name);
        let local = LocalMove {
            from,
            target_parent: &target_parent,
            paths: self,
            taken,
            stopped,
        };
        Ok(match local.run() {
            Ok(moved) => moved.map_err(|error| move_failure(from, error, display_name)),
            Err(WaitCancelled) => Err(cancelled_local_operation()),
        })
    }
}

fn move_failure(from: Entry<'_>, error: PlaceError, display_name: &str) -> glib::Error {
    match still_open_rename(from, error) {
        Some(reason) => io_error(could_not_move(
            display_name,
            &crate::services::error_detail(reason),
        )),
        None => placement_failure(error, display_name, could_not_move),
    }
}

/// Renames one item into the target folder, numbering it where `taken` asks to.
struct LocalMove<'a> {
    from: Entry<'a>,
    target_parent: &'a OwnedFd,
    paths: &'a MovePaths,
    taken: Taken,
    stopped: &'a gio::Cancellable,
}

impl LocalMove<'_> {
    fn run(&self) -> Result<Result<Moved, PlaceError>, WaitCancelled> {
        let is_directory = match self.is_directory() {
            Ok(is_directory) => is_directory,
            Err(error) => return Ok(Err(error)),
        };
        let limit = || mount_profile::name_limit(fs(), self.target_parent.as_fd());
        let deadline = Deadline::default();
        for name in self
            .taken
            .names(&self.paths.target_name, is_directory, limit)
        {
            let to = Entry::new(self.target_parent, &name);
            let placed = deferred_close::retry_folder_rename(
                fs(),
                self.from,
                &deadline,
                &|| self.stopped.is_cancelled(),
                |error| matches!(error, PlaceError::Io(rustix::io::Errno::ACCESS)),
                || placement::place(fs(), self.from, to, None),
            )?;
            if self.taken == Taken::Number && matches!(placed, Err(PlaceError::Exists)) {
                continue;
            }
            return Ok(match placed {
                Ok(_) => Ok(Moved::Done(gio::File::for_path(
                    self.paths.target_parent.join(&name),
                ))),
                Err(PlaceError::CrossDevice | PlaceError::Unsupported) => self.copy_instead(to),
                Err(error) => Err(error),
            });
        }
        Ok(Err(PlaceError::Exists))
    }

    fn is_directory(&self) -> Result<bool, PlaceError> {
        match self.taken {
            Taken::Fail => Ok(false),
            Taken::Number => fs()
                .stat(self.from, false)
                .map(|stat| stat.is_directory())
                .map_err(PlaceError::Io),
        }
    }

    fn copy_instead(&self, to: Entry<'_>) -> Result<Moved, PlaceError> {
        match fs().stat(self.from, false) {
            Ok(stat)
                if stat.is_directory()
                    && placement::contains(fs(), stat.identity, to.dir) != Some(false) =>
            {
                Err(PlaceError::IntoItself)
            }
            Ok(_) => Ok(Moved::Copy),
            Err(error) => Err(PlaceError::Io(error)),
        }
    }
}
