// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn merging_a_file_is_rejected_without_touching_either_side() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.txt");
    let target = root.path().join("target.txt");
    write_files(root.path(), &[("source.txt", "new"), ("target.txt", "old")])?;

    let result = merge_into(&source, &target, &no_plan);

    assert!(result.is_err());
    assert_files(
        root.path(),
        &[("source.txt", "new"), ("target.txt", "old")],
        "",
    )?;
    Ok(())
}

#[test]
fn merging_onto_a_file_is_rejected_without_touching_either_side() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir_all(&source)?;
    fs::write(source.join("incoming.txt"), b"new")?;
    fs::write(&target, b"old")?;

    let result = merge_into(&source, &target, &no_plan);

    assert!(result.is_err());
    assert!(source.join("incoming.txt").exists());
    assert_eq!(fs::read(&target)?, b"old");
    Ok(())
}

/// Writes a partial file into the target, then fails as cancelled.
fn partial_then_cancelled() -> CopyStep {
    Rc::new(|_, target, _, _| {
        Box::pin(async move {
            fs::write(
                target
                    .path()
                    .ok_or_else(|| super::super::super::super::io_error("missing target path"))?
                    .join("partial.txt"),
                b"partial",
            )
            .map_err(super::super::super::super::io_error)?;
            Err(glib::Error::new(
                gio::IOErrorEnum::Cancelled,
                "injected cancellation",
            ))
        })
    })
}

#[test]
fn a_failed_merge_preserves_the_destinations_existing_contents() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir_all(&source)?;
    fs::create_dir_all(&target)?;
    fs::write(source.join("incoming.txt"), b"new")?;
    fs::write(target.join("stays.txt"), b"keep me")?;

    let hooks = hooks(
        NamePolicy::default(),
        partial_then_cancelled(),
        Rc::new(trash_stage_overwrite),
        &no_plan,
    );
    let result = merge_hooked(gio::File::for_path(&source), &target, hooks);

    assert!(result.is_err());
    assert_eq!(fs::read(target.join("stays.txt"))?, b"keep me");
    assert!(source.join("incoming.txt").exists());
    Ok(())
}
