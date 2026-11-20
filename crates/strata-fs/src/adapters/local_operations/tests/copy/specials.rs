// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn special_files_inside_a_copied_folder_are_recreated_or_left_out_and_reported()
-> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    fs::create_dir_all(&source)?;
    fs::create_dir(&destination)?;
    make_fifo(&source.join("00-pipe"), 0o600)?;
    fs::set_permissions(source.join("00-pipe"), fs::Permissions::from_mode(0o640))?;
    let _socket = std::os::unix::net::UnixListener::bind(source.join("socket"))?;
    for index in 1..4 {
        fs::write(
            source.join(format!("{index:02}-file.txt")),
            index.to_string(),
        )?;
    }
    let modified = rustix::fs::Timespec {
        tv_sec: 1_000_000_000,
        tv_nsec: 0,
    };
    rustix::fs::utimensat(
        rustix::fs::CWD,
        source.join("00-pipe"),
        &rustix::fs::Timestamps {
            last_access: modified,
            last_modification: modified,
        },
        rustix::fs::AtFlags::empty(),
    )?;
    let _sources = FsQuirks::install(&source, Quirks::default());
    let _destination = FsQuirks::install(&destination, Quirks::default());
    let target = destination.join("copy");

    let report = copy_reporting(&source, &target, None)?;

    let pipe = fs::symlink_metadata(target.join("00-pipe"))?;
    assert!(pipe.file_type().is_fifo());
    assert_eq!(pipe.mode() & 0o7777, 0o640);
    assert_eq!(pipe.mtime(), 1_000_000_000);
    assert_eq!(
        report
            .skipped
            .into_iter()
            .map(|item| (item.location, item.kind))
            .collect::<Vec<_>>(),
        [(
            Location::local(target.join("socket")),
            crate::services::SpecialKind::Socket,
        )]
    );
    assert!(!target.join("socket").exists());
    for index in 1..4 {
        assert_eq!(
            fs::read_to_string(target.join(format!("{index:02}-file.txt")))?,
            index.to_string()
        );
    }
    Ok(())
}

#[test]
fn a_special_file_left_out_of_fat_is_reported_under_its_own_name() -> Result<(), Box<dyn Error>> {
    use crate::services::SpecialKind;
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let stick = root.path().join("stick");
    fs::create_dir_all(&source)?;
    fs::create_dir(&stick)?;
    make_fifo(&source.join("p:ipe"), 0o600)?;
    let _socket = std::os::unix::net::UnixListener::bind(source.join("s:ock"))?;
    let _stick = FsQuirks::install(&stick, Quirks::vfat());
    let target = stick.join("copy");

    let (copied, report) = reporting(
        &source,
        super::super::super::copy_engine::Destination::New(target.clone(), Taken::Fail),
        super::super::super::copy_engine::Options {
            names: fixing(),
            ..super::super::super::copy_engine::Options::default()
        },
    );

    copied?;
    let mut skipped = report
        .skipped
        .into_iter()
        .map(|item| (item.location, item.kind))
        .collect::<Vec<_>>();
    skipped.sort_by(|left, right| left.0.compare(&right.0));
    assert_eq!(
        skipped,
        [
            (Location::local(target.join("p:ipe")), SpecialKind::Pipe),
            (Location::local(target.join("s:ock")), SpecialKind::Socket),
        ]
    );
    assert_eq!(report.renamed, []);
    assert_eq!(names(&target)?, Vec::<String>::new());
    Ok(())
}

#[test]
fn named_pipes_are_left_out_only_where_the_destination_refuses_them() -> Result<(), Box<dyn Error>>
{
    use rustix::io::Errno;
    let _serial = serial()?;
    // How vfat, exfat-fuse, SMB and others refuse a pipe, and errors that
    // stop the copy instead.
    let rows = [
        (Errno::PERM, true),
        (Errno::IO, true),
        (Errno::OPNOTSUPP, true),
        (Errno::NOSYS, true),
        (Errno::INVAL, true),
        (Errno::NOSPC, false),
        (Errno::ACCESS, false),
    ];
    for (errno, refused) in rows {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let destination = root.path().join("destination");
        fs::create_dir_all(&source)?;
        fs::create_dir(&destination)?;
        make_fifo(&source.join("pipe"), 0o600)?;
        fs::write(source.join("file.txt"), b"file")?;
        let _destination = FsQuirks::install(
            &destination,
            Quirks {
                fifos: fs_quirks::Support::Rejected(errno),
                ..Quirks::default()
            },
        );
        let target = destination.join("copy");

        let copied = copy_reporting(&source, &target, None);

        if refused {
            let report = copied.map_err(|error| format!("{errno:?}: {error}"))?;
            let skipped = report
                .skipped
                .into_iter()
                .map(|item| (item.location, item.kind))
                .collect::<Vec<_>>();
            assert_eq!(
                skipped,
                [(
                    Location::local(target.join("pipe")),
                    crate::services::SpecialKind::Pipe
                )],
                "{errno:?}"
            );
            assert_eq!(names(&target)?, ["file.txt"], "{errno:?}");
        } else {
            assert!(copied.is_err(), "{errno:?}");
            assert_eq!(names(&destination)?, Vec::<String>::new(), "{errno:?}");
        }
    }
    Ok(())
}

#[test]
fn a_named_pipe_whose_copy_fails_is_not_left_behind() -> Result<(), Box<dyn Error>> {
    use crate::adapters::volume::learn_mount_capability;
    use rustix::io::Errno;
    let _serial = serial()?;
    for selected in [true, false] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("source");
        let destination = root.path().join("destination");
        fs::create_dir_all(&source)?;
        fs::create_dir(&destination)?;
        make_fifo(&source.join("pipe"), 0o640)?;
        fs::write(source.join("a.txt"), b"a")?;
        let scope = FsQuirks::install(
            &destination,
            Quirks {
                fail: vec![(
                    Op::SetTimes,
                    if selected { ".strata-copy-*" } else { "pipe" },
                    0,
                    Errno::IO,
                )],
                ..Quirks::without_folder_placement()
            },
        );
        learn_mount_capability(scope.key(), |capabilities| {
            capabilities.no_replace = Some(false);
            capabilities.hard_links = Some(true);
            capabilities.rename_over_empty_directory = Some(false);
        });
        let (from, to) = if selected {
            (source.join("pipe"), destination.join("pipe"))
        } else {
            (source.clone(), destination.join("copy"))
        };

        let copied = glib::MainContext::default().block_on(copy_new_recursively(
            gio::File::for_path(from),
            gio::File::for_path(to),
            gio::Cancellable::new(),
        ));

        assert!(copied.is_err(), "selected: {selected}");
        assert_eq!(
            names(&destination)?,
            Vec::<String>::new(),
            "selected: {selected}"
        );
    }
    Ok(())
}

#[test]
fn a_selected_named_pipe_is_recreated_where_the_destination_can() -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::FileTypeExt;
    let _serial = serial()?;
    for (row, preset, pipes) in [
        ("tmpfs", Quirks::default as Preset, true),
        (
            "recreated without placement",
            Quirks::without_placement,
            true,
        ),
        ("vfat", Quirks::vfat, false),
    ] {
        let root = tempfile::tempdir()?;
        let destination = root.path().join("destination");
        fs::create_dir(&destination)?;
        let source = root.path().join("pipe");
        make_fifo(&source, 0o600)?;
        let _destination = FsQuirks::install(&destination, preset());
        let target = destination.join("pipe");

        let copied = glib::MainContext::default().block_on(copy_new_recursively(
            gio::File::for_path(&source),
            gio::File::for_path(&target),
            gio::Cancellable::new(),
        ));

        if pipes {
            copied?;
            assert!(
                fs::symlink_metadata(&target)?.file_type().is_fifo(),
                "{row}"
            );
        } else {
            let error = copied.expect_err("vfat has no named pipes");
            assert_eq!(
                error.message(),
                rust_i18n::t!(
                    "Cannot copy %{name}: it is not a regular file, directory, or symbolic link",
                    name = "pipe"
                ),
                "{row}"
            );
            assert_eq!(names(&destination)?, Vec::<String>::new(), "{row}");
        }
    }
    Ok(())
}
