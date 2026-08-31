// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs,
    io::{self, Write},
    os::unix::fs::{MetadataExt, symlink},
    path::PathBuf,
    sync::atomic::Ordering,
};

use super::{NEXT_TEMP_FILE, atomic_write, atomic_write_with};

#[test]
fn write_failure_preserves_destination_and_removes_temporary_file() -> io::Result<()> {
    let directory = test_directory()?;
    let destination = directory.join("settings.toml");
    fs::write(&destination, b"valid")?;

    let result = atomic_write_with(&destination, |file| {
        file.write_all(b"partial")?;
        Err(io::Error::other("injected write failure"))
    });

    assert!(result.is_err());
    assert_eq!(fs::read(&destination)?, b"valid");
    assert_eq!(fs::read_dir(&directory)?.count(), 1);
    fs::remove_dir_all(directory)
}

#[test]
fn symlink_destination_is_rejected_without_touching_target() -> io::Result<()> {
    let directory = test_directory()?;
    let target = directory.join("target");
    let destination = directory.join("settings.toml");
    fs::write(&target, b"valid")?;
    symlink(&target, &destination)?;

    let result = atomic_write(&destination, b"replacement");

    assert!(result.is_err());
    assert_eq!(fs::read(&target)?, b"valid");
    assert!(fs::symlink_metadata(&destination)?.file_type().is_symlink());
    fs::remove_dir_all(directory)
}

#[test]
fn long_destination_name_writes_with_private_permissions() -> io::Result<()> {
    let directory = test_directory()?;
    let destination = directory.join(format!("{}.toml", "a".repeat(240)));

    atomic_write(&destination, b"valid")?;

    assert_eq!(fs::metadata(&destination)?.mode() & 0o777, 0o600);
    fs::remove_dir_all(directory)
}

#[test]
fn non_regular_destination_is_rejected() -> io::Result<()> {
    let directory = test_directory()?;
    let destination = directory.join("settings.toml");
    fs::create_dir(&destination)?;

    assert!(atomic_write(&destination, b"replacement").is_err());
    assert!(destination.is_dir());
    fs::remove_dir_all(directory)
}

fn test_directory() -> io::Result<PathBuf> {
    loop {
        let path = std::env::temp_dir().join(format!(
            "strata-storage-{}-{}",
            std::process::id(),
            NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
}
