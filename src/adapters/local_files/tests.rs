// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    error::Error,
    ffi::OsString,
    fs,
    io::{ErrorKind, Write},
    os::unix::ffi::{OsStrExt, OsStringExt},
    sync::{Arc, Mutex, MutexGuard},
    time::SystemTime,
};

use tracing_subscriber::fmt::MakeWriter;

use super::*;
use crate::model::Location;

#[derive(Clone, Default)]
struct LogWriter(Arc<Mutex<Vec<u8>>>);

struct LogWriterGuard<'a>(MutexGuard<'a, Vec<u8>>);

impl Write for LogWriterGuard<'_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.0.write(buffer)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

impl<'a> MakeWriter<'a> for LogWriter {
    type Writer = LogWriterGuard<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        LogWriterGuard(self.0.lock().unwrap_or_else(|error| error.into_inner()))
    }
}

impl LogWriter {
    fn output(&self) -> String {
        let output = self.0.lock().unwrap_or_else(|error| error.into_inner());
        String::from_utf8_lossy(&output).into_owned()
    }
}

fn capture_directory_start_log(level: tracing::Level, location: &Location) -> String {
    let writer = LogWriter::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(level)
        .with_writer(writer.clone())
        .finish();

    tracing::subscriber::with_default(subscriber, || {
        log_directory_load_started(RequestId(42), location);
    });
    writer.output()
}

#[test]
fn directory_logging_respects_default_and_diagnostic_privacy() {
    let native_path = "/home/alice/sentinel-private-directory";
    let native = Location::local(native_path);

    let default_log = capture_directory_start_log(tracing::Level::INFO, &native);
    assert!(default_log.contains("directory load started"));
    assert!(default_log.contains("backend=native"));
    assert!(!default_log.contains(native_path));

    let diagnostic_log = capture_directory_start_log(tracing::Level::DEBUG, &native);
    assert!(diagnostic_log.contains(native_path));

    let remote = Location::uri(
        "sftp://alice:password;key=secret@example.com/private?token=secret#private-fragment",
    );
    let remote_default_log = capture_directory_start_log(tracing::Level::INFO, &remote);
    assert!(remote_default_log.contains("backend=sftp"));
    assert!(!remote_default_log.contains("example.com"));

    let remote_log = capture_directory_start_log(tracing::Level::DEBUG, &remote);
    assert!(remote_log.contains("sftp://example.com/private"));
    for secret in [
        "alice",
        "password",
        "key=secret",
        "token=secret",
        "private-fragment",
    ] {
        assert!(!remote_log.contains(secret));
    }
}

#[test]
fn validation_accepts_readable_directories_and_rejects_files_and_missing_paths()
-> Result<(), Box<dyn Error>> {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("strata-location-test-{unique}"));
    let file = directory.join("file.txt");
    let missing = directory.join("missing");
    fs::create_dir(&directory)?;
    fs::write(&file, b"fixture")?;

    let source = LocalFileSource;
    assert_eq!(
        source.validate_location(&Location::local(&directory)),
        Ok(())
    );
    assert_eq!(
        source.validate_location(&Location::local(&file)),
        Err(LocationValidationError::NotDirectory)
    );
    assert_eq!(
        source.validate_location(&Location::local(&missing)),
        Err(LocationValidationError::Missing)
    );

    fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn invalid_utf8_names_keep_their_native_bytes() -> Result<(), Box<dyn Error>> {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("strata-native-name-test-{unique}"));
    fs::create_dir(&directory)?;
    let native_name = OsString::from_vec(b"invalid-\xff".to_vec());
    let path = directory.join(&native_name);
    fs::write(&path, b"fixture")?;

    let info = gio::File::for_path(&path).query_info(
        ATTRIBUTES,
        gio::FileQueryInfoFlags::NONE,
        None::<&gio::Cancellable>,
    )?;
    let entry = entry_from_info(Location::local(path.clone()), info);

    assert_eq!(entry.native_name.as_bytes(), native_name.as_bytes());
    assert_eq!(entry.location.native_path(), Some(path.as_path()));
    assert!(!entry.display_name.is_empty());

    fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn unmounted_network_shares_are_treated_as_directories() {
    let info = gio::FileInfo::new();
    info.set_file_type(gio::FileType::Mountable);
    info.set_is_symlink(false);
    info.set_name("share");
    info.set_display_name("share");

    let entry = entry_from_info(Location::uri("smb://host/share"), info);

    assert_eq!(entry.kind, EntryKind::Directory);
    assert!(entry.is_directory());
}

#[test]
fn symlink_targets_and_broken_links_are_distinguished() -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::symlink;

    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("strata-symlink-test-{unique}"));
    fs::create_dir(&directory)?;
    fs::create_dir(directory.join("directory"))?;
    fs::write(directory.join("file"), b"fixture")?;
    symlink("directory", directory.join("directory-link"))?;
    symlink("file", directory.join("file-link"))?;
    symlink("missing", directory.join("broken-link"))?;

    let kind = |name: &str| -> Result<EntryKind, glib::Error> {
        let path = directory.join(name);
        let info = gio::File::for_path(&path).query_info(
            ATTRIBUTES,
            gio::FileQueryInfoFlags::NONE,
            None::<&gio::Cancellable>,
        )?;
        Ok(entry_from_info(Location::local(path), info).kind)
    };

    assert_eq!(kind("directory-link")?, EntryKind::DirectorySymbolicLink);
    assert_eq!(kind("file-link")?, EntryKind::FileSymbolicLink);
    assert_eq!(kind("broken-link")?, EntryKind::SymbolicLink);

    fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn coalescing_preserves_a_move_when_metadata_follows_it() {
    let change = merge_pending_change(
        PendingMonitorChange::Move {
            from: "/fixture/old".into(),
            to: "/fixture/new".into(),
        },
        PendingMonitorChange::Upsert("/fixture/new".into()),
    );

    assert!(matches!(change, PendingMonitorChange::Move { .. }));
}

#[test]
fn conflicting_move_events_fall_back_to_a_rescan() {
    let change = merge_pending_change(
        PendingMonitorChange::Move {
            from: "/fixture/old".into(),
            to: "/fixture/new".into(),
        },
        PendingMonitorChange::Remove("/fixture/new".into()),
    );

    assert!(matches!(change, PendingMonitorChange::Rescan));
}

#[test]
fn permission_errors_are_reported_as_inaccessible() {
    let error = std::io::Error::from(ErrorKind::PermissionDenied);
    assert_eq!(
        map_validation_error(error),
        LocationValidationError::Inaccessible
    );
}
