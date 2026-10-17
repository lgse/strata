// SPDX-License-Identifier: MIT

use rustix::io::Errno;

use super::super::fs_ops::{Entry, fs};
use super::fs_quirks::{Op, Support};
use super::*;
use crate::model::hidden_names::{Leftover, classify};

#[test]
fn a_copy_cut_off_by_an_unplug_leaves_only_its_stage() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.bin");
    fs::write(&source, b"copied")?;
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;
    let target = destination.join("copy.bin");
    let _source = FsQuirks::install(root.path(), Quirks::ext4());
    let scope = FsQuirks::install(
        &destination,
        Quirks {
            tmpfile: Support::Rejected(Errno::OPNOTSUPP),
            short_io: Some(2),
            unplugged_after: Some((Op::Write, ".strata-copy-*", 1)),
            ..Quirks::ext4()
        },
    );
    let copy = || {
        glib::MainContext::default().block_on(copy_new_recursively(
            gio::File::for_path(&source),
            gio::File::for_path(&target),
            gio::Cancellable::new(),
        ))
    };
    let stat = |dir: &Path, name: &str| -> Result<Result<(), Errno>, Box<dyn Error>> {
        let dir = open_local_parent_directory(dir)?;
        Ok(fs()
            .stat(Entry::new(&dir, OsStr::new(name)), false)
            .map(drop))
    };

    assert!(copy().is_err());
    let left = names(&destination)?;
    let [stage] = left.as_slice() else {
        panic!("only the stage stays: {left:?}");
    };
    assert_eq!(classify(OsStr::new(stage)), Some(Leftover::IncompleteCopy));
    assert_eq!(fs::read(destination.join(stage))?, b"copi");
    let calls = scope.calls();
    let unplugged = calls
        .iter()
        .enumerate()
        .filter(|(_, call)| call.op == Op::Write)
        .nth(1)
        .map(|(index, _)| index)
        .ok_or("the stage's second write")?;
    assert!(
        calls[unplugged + 1..]
            .iter()
            .any(|call| call.names.iter().any(|name| name == stage.as_str())),
        "{calls:?}"
    );
    assert_eq!(stat(&destination, stage)?, Err(Errno::IO));
    assert_eq!(stat(root.path(), "source.bin")?, Ok(()));
    assert!(
        fs().trash(&destination.join(stage), &gio::Cancellable::new())
            .is_err()
    );

    scope.clear_calls();
    assert!(copy().is_err());
    assert!(!scope.calls().is_empty());
    assert_eq!(names(&destination)?, left);

    scope.replug();
    assert_eq!(stat(&destination, stage)?, Ok(()));
    copy()?;
    assert_eq!(fs::read(&target)?, b"copied");
    assert_eq!(names(&destination)?, [stage.as_str(), "copy.bin"]);
    Ok(())
}
