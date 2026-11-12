// SPDX-License-Identifier: MIT

use super::super::super::{RemoteFileStageCommit, RemoteFileStageCopy};
use super::*;

/// Copies the source into the stage with `fs::copy`.
fn copy_into_stage() -> RemoteFileStageCopy {
    Rc::new(|source, stage, _| {
        Box::pin(async move {
            fs::copy(
                source.path().expect("native source"),
                stage.path().expect("native stage"),
            )
            .map(drop)
            .map_err(super::super::io_error)
        })
    })
}

/// Commits the stage by renaming it to the target.
fn rename_stage_to_target() -> RemoteFileStageCommit {
    Rc::new(|stage, target, _| {
        Box::pin(async move {
            fs::rename(
                stage.path().expect("native stage"),
                target.path().expect("native target"),
            )
            .map_err(super::super::io_error)
        })
    })
}

/// Another program writes the target just before the commit, which then
/// fails as the name is taken.
fn race_the_commit_at(name: &'static str) -> RemoteFileStageCommit {
    Rc::new(move |stage, target, _| {
        Box::pin(async move {
            let target = target.path().expect("native target");
            if target.ends_with(name) {
                fs::write(&target, b"racing contents").map_err(super::super::io_error)?;
                return Err(glib::Error::new(
                    gio::IOErrorEnum::Exists,
                    "injected destination race",
                ));
            }
            fs::rename(stage.path().expect("native stage"), target).map_err(super::super::io_error)
        })
    })
}

fn copy_remote_file(
    (source, target): (&Path, &Path),
    taken: Taken,
    copy_to_stage: RemoteFileStageCopy,
    commit_stage: RemoteFileStageCommit,
) -> Result<gio::File, glib::Error> {
    glib::MainContext::default().block_on(copy_new_remote_file_with(
        gio::File::for_path(source),
        gio::File::for_path(target),
        taken,
        gio::Cancellable::new(),
        copy_to_stage,
        commit_stage,
    ))
}

/// A `source.bin` beneath `root` and the `target.bin` it is copied to.
fn source_and_target(root: &Path) -> io::Result<(PathBuf, PathBuf)> {
    let source = root.join("source.bin");
    fs::write(&source, b"source contents")?;
    Ok((source, root.join("target.bin")))
}

#[test]
fn cancelling_staged_remote_file_copy_removes_only_the_incomplete_stage()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let (source, target) = source_and_target(root.path())?;

    let result = copy_remote_file(
        (&source, &target),
        Taken::Fail,
        Rc::new(|_, stage, _| {
            Box::pin(async move {
                fs::write(stage.path().expect("native stage"), b"partial")
                    .map_err(super::super::io_error)?;
                Err(glib::Error::new(
                    gio::IOErrorEnum::Cancelled,
                    "injected cancellation",
                ))
            })
        }),
        Rc::new(|_, _, _| Box::pin(async { panic!("cancelled copy must not commit") })),
    );

    assert!(result.is_err_and(|error| error.matches(gio::IOErrorEnum::Cancelled)));
    assert!(!target.exists());
    let left = (fs::read(&source)?, fs::read_dir(root.path())?.count());
    assert_eq!(left, (b"source contents".to_vec(), 1));
    Ok(())
}

#[test]
fn staged_remote_file_copy_preserves_a_racing_destination() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let (source, target) = source_and_target(root.path())?;

    let result = copy_remote_file(
        (&source, &target),
        Taken::Fail,
        copy_into_stage(),
        race_the_commit_at("target.bin"),
    );

    assert!(result.is_err_and(|error| error.matches(gio::IOErrorEnum::Exists)));
    assert_files(
        root.path(),
        &[
            ("target.bin", "racing contents"),
            ("source.bin", "source contents"),
        ],
    )?;
    assert_eq!(fs::read_dir(root.path())?.count(), 2);
    Ok(())
}

#[test]
fn a_remote_keep_both_copy_takes_the_next_name_when_one_is_taken_meanwhile()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let (source, target) = source_and_target(root.path())?;
    fs::write(&target, b"existing")?;

    let copied = copy_remote_file(
        (&source, &target),
        Taken::Number,
        copy_into_stage(),
        race_the_commit_at("target (1).bin"),
    )?;

    assert_eq!(copied.path(), Some(root.path().join("target (2).bin")));
    assert_files(
        root.path(),
        &[
            ("target (2).bin", "source contents"),
            ("target (1).bin", "racing contents"),
            ("target.bin", "existing"),
        ],
    )?;
    assert_eq!(
        names(root.path())?,
        [
            "source.bin",
            "target (1).bin",
            "target (2).bin",
            "target.bin"
        ]
    );
    Ok(())
}

#[test]
fn a_remote_keep_both_copy_numbers_a_long_name_within_the_limit() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.bin");
    let target = root.path().join(format!("{}.bin", "t".repeat(251)));
    let numbered = root.path().join(format!("{} (1).bin", "t".repeat(247)));
    fs::write(&source, b"source contents")?;
    fs::write(&target, b"existing")?;

    let copied = copy_remote_file(
        (&source, &target),
        Taken::Number,
        copy_into_stage(),
        rename_stage_to_target(),
    )?;

    assert_eq!(copied.path(), Some(numbered.clone()));
    assert_eq!(fs::read(numbered)?, b"source contents");
    assert_eq!(fs::read(&target)?, b"existing");
    Ok(())
}

#[test]
fn failed_incomplete_copy_cleanup_is_reported_as_a_failure() {
    let error = copy_failure_after_cleanup(
        glib::Error::new(gio::IOErrorEnum::Cancelled, "injected cancellation"),
        Err(glib::Error::new(
            gio::IOErrorEnum::Failed,
            "injected cleanup failure",
        )),
    );

    assert!(!error.matches(gio::IOErrorEnum::Cancelled));
    assert!(
        error
            .to_string()
            .contains("incomplete copy could not be removed")
    );
    assert!(error.to_string().contains("injected cleanup failure"));
}
