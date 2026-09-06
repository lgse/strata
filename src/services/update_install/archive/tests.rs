// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use super::{MAX_ENTRIES, extract_release_archive};

const PACKAGE: &str = "strata-0.11.2-x86_64-unknown-linux-gnu";

fn scratch_dir(label: &str, line: u32) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "strata-update-archive-test-{label}-{}-{line}",
        std::process::id()
    ));
    let _removed = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

struct ArchiveBuilder {
    builder: Option<tar::Builder<flate2::write::GzEncoder<fs::File>>>,
    path: PathBuf,
}

impl ArchiveBuilder {
    fn new(dir: &Path) -> Self {
        let path = dir.join("update.tar.gz");
        let file = fs::File::create(&path).expect("create archive");
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
        Self {
            builder: Some(tar::Builder::new(encoder)),
            path,
        }
    }

    fn builder(&mut self) -> &mut tar::Builder<flate2::write::GzEncoder<fs::File>> {
        self.builder.as_mut().expect("archive is still open")
    }

    fn file(&mut self, name: &str, contents: &[u8]) -> &mut Self {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        self.builder()
            .append_data(&mut header, name, contents)
            .expect("append file");
        self
    }

    fn required(&mut self) -> &mut Self {
        self.file(&format!("{PACKAGE}/strata"), b"binary")
            .file(&format!("{PACKAGE}/SOURCE_COMMIT"), b"abc123\n")
    }

    fn special(&mut self, name: &str, kind: tar::EntryType, link: &str) -> &mut Self {
        let mut header = tar::Header::new_gnu();
        header.set_size(0);
        header.set_mode(0o777);
        header.set_entry_type(kind);
        if !link.is_empty() {
            header
                .set_link_name(link)
                .expect("set link name for special entry");
        }
        header.set_cksum();
        self.builder()
            .append_data(&mut header, name, std::io::empty())
            .expect("append special entry");
        self
    }

    /// Writes the entry name straight into the header, bypassing the checks
    /// `set_path` applies. A hostile archive is built by something that does
    /// not use this crate, so the test fixtures cannot go through them either.
    fn raw_named(&mut self, name: &str, contents: &[u8]) -> &mut Self {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        let gnu = header.as_gnu_mut().expect("gnu header");
        let bytes = name.as_bytes();
        gnu.name[..bytes.len()].copy_from_slice(bytes);
        header.set_cksum();
        self.builder()
            .append(&header, contents)
            .expect("append raw entry");
        self
    }

    /// Closes the tar stream *and* the gzip stream: a `GzEncoder` left
    /// unfinished writes a truncated member that no reader can decode.
    fn finish(&mut self) -> PathBuf {
        let builder = self.builder.take().expect("archive is still open");
        let encoder = builder.into_inner().expect("finish tar stream");
        encoder.finish().expect("finish gzip stream");
        self.path.clone()
    }
}

fn extract_into(dir: &Path, archive: &Path) -> Result<PathBuf, String> {
    let destination = dir.join("extracted");
    fs::create_dir_all(&destination).expect("create destination");
    extract_release_archive(archive, &destination)
}

#[test]
fn extraction_writes_the_packaged_files_and_returns_the_package_directory() {
    let dir = scratch_dir("extract", line!());
    let mut archive = ArchiveBuilder::new(&dir);
    let archive = archive
        .required()
        .file(&format!("{PACKAGE}/README.md"), b"readme")
        .file(&format!("{PACKAGE}/portal/strata.portal"), b"portal")
        .finish();

    let package = extract_into(&dir, &archive).expect("extract archive");

    assert_eq!(package.file_name().expect("package name"), PACKAGE);
    assert_eq!(
        fs::read_to_string(package.join("SOURCE_COMMIT")).expect("read commit"),
        "abc123\n"
    );
    assert!(package.join("portal/strata.portal").is_file());
}

#[test]
fn extraction_rejects_a_symlink_entry() {
    let dir = scratch_dir("symlink", line!());
    let mut archive = ArchiveBuilder::new(&dir);
    let archive = archive
        .required()
        .special(
            &format!("{PACKAGE}/escape"),
            tar::EntryType::Symlink,
            "/etc/passwd",
        )
        .finish();

    let error = extract_into(&dir, &archive).expect_err("symlink entry must be rejected");

    assert!(error.contains("symbolic link"), "unexpected error: {error}");
}

#[test]
fn extraction_rejects_a_hard_link_entry() {
    let dir = scratch_dir("hardlink", line!());
    let mut archive = ArchiveBuilder::new(&dir);
    let archive = archive
        .required()
        .special(
            &format!("{PACKAGE}/linked"),
            tar::EntryType::Link,
            &format!("{PACKAGE}/strata"),
        )
        .finish();

    let error = extract_into(&dir, &archive).expect_err("hard link entry must be rejected");

    assert!(error.contains("hard link"), "unexpected error: {error}");
}

#[test]
fn extraction_rejects_a_device_entry() {
    let dir = scratch_dir("device", line!());
    let mut archive = ArchiveBuilder::new(&dir);
    let archive = archive
        .required()
        .special(&format!("{PACKAGE}/node"), tar::EntryType::Char, "")
        .finish();

    let error = extract_into(&dir, &archive).expect_err("device entry must be rejected");

    assert!(error.contains("device"), "unexpected error: {error}");
}

#[test]
fn extraction_rejects_a_parent_directory_escape() {
    let dir = scratch_dir("escape", line!());
    let mut archive = ArchiveBuilder::new(&dir);
    let archive = archive
        .required()
        .raw_named(&format!("{PACKAGE}/../escaped"), b"escaped")
        .finish();

    let error = extract_into(&dir, &archive).expect_err("parent escape must be rejected");

    assert!(
        error.contains("outside its directory"),
        "unexpected error: {error}"
    );
    assert!(!dir.join("escaped").exists());
}

#[test]
fn extraction_rejects_an_absolute_path() {
    let dir = scratch_dir("absolute", line!());
    let mut archive = ArchiveBuilder::new(&dir);
    let archive = archive
        .required()
        .raw_named("/etc/strata-owned", b"evil")
        .finish();

    let error = extract_into(&dir, &archive).expect_err("absolute path must be rejected");

    assert!(
        error.contains("outside its directory"),
        "unexpected error: {error}"
    );
    assert!(!Path::new("/etc/strata-owned").exists());
}

#[test]
fn extraction_rejects_more_than_one_package_directory() {
    let dir = scratch_dir("two-packages", line!());
    let mut archive = ArchiveBuilder::new(&dir);
    let archive = archive
        .required()
        .file("strata-other/strata", b"binary")
        .finish();

    let error = extract_into(&dir, &archive).expect_err("second package must be rejected");

    assert!(
        error.contains("more than one package directory"),
        "unexpected error: {error}"
    );
}

#[test]
fn extraction_rejects_an_archive_missing_the_binary() {
    let dir = scratch_dir("no-binary", line!());
    let mut archive = ArchiveBuilder::new(&dir);
    let archive = archive
        .file(&format!("{PACKAGE}/SOURCE_COMMIT"), b"abc123\n")
        .finish();

    let error = extract_into(&dir, &archive).expect_err("missing binary must be rejected");

    assert!(error.contains("no strata"), "unexpected error: {error}");
}

#[test]
fn extraction_rejects_an_archive_missing_the_source_commit() {
    let dir = scratch_dir("no-commit", line!());
    let mut archive = ArchiveBuilder::new(&dir);
    let archive = archive
        .file(&format!("{PACKAGE}/strata"), b"binary")
        .finish();

    let error = extract_into(&dir, &archive).expect_err("missing commit must be rejected");

    assert!(
        error.contains("no SOURCE_COMMIT"),
        "unexpected error: {error}"
    );
}

#[test]
fn extraction_rejects_too_many_entries() {
    let dir = scratch_dir("entries", line!());
    let mut archive = ArchiveBuilder::new(&dir);
    archive.required();
    for index in 0..=MAX_ENTRIES {
        archive.file(&format!("{PACKAGE}/file-{index}"), b"x");
    }
    let archive = archive.finish();

    let error = extract_into(&dir, &archive).expect_err("entry flood must be rejected");

    assert!(error.contains("more than"), "unexpected error: {error}");
}

#[test]
fn extraction_rejects_an_archive_that_expands_past_the_ceiling() {
    let dir = scratch_dir("bomb", line!());
    let path = dir.join("bomb.tar.gz");
    let file = fs::File::create(&path).expect("create archive");
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::best());
    let mut builder = tar::Builder::new(encoder);
    let mut header = tar::Header::new_gnu();
    // A header promising far more than the ceiling, so the entry is refused
    // before its bytes are written.
    header.set_size(super::MAX_TOTAL_BYTES + 1);
    header.set_mode(0o644);
    header.set_entry_type(tar::EntryType::Regular);
    header
        .set_path(format!("{PACKAGE}/bomb"))
        .expect("set path");
    header.set_cksum();
    builder
        .append(&header, std::io::empty())
        .expect("append bomb header");
    let mut inner = builder.into_inner().expect("finish builder");
    inner.flush().expect("flush encoder");
    drop(inner);

    let error = extract_into(&dir, &path).expect_err("archive bomb must be rejected");

    assert!(error.contains("oversized"), "unexpected error: {error}");
}
