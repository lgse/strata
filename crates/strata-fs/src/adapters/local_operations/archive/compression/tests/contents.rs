// SPDX-License-Identifier: MIT

//! What each format stores for a compressed tree: folders, files and links.

use super::*;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum CompressedEntry {
    Directory,
    File(Vec<u8>),
    Symlink(PathBuf),
}

type Entries = BTreeMap<PathBuf, CompressedEntry>;

pub(super) fn insert_once<V>(entries: &mut BTreeMap<PathBuf, V>, name: PathBuf, entry: V) {
    assert!(entries.insert(name, entry).is_none());
}

fn read_zip_entries(file: fs::File, password: Option<&str>) -> Result<Entries, Box<dyn Error>> {
    let mut entries = BTreeMap::new();
    let mut archive = zip::ZipArchive::new(file)?;
    for index in 0..archive.len() {
        let options = zip::read::ZipReadOptions::new().password(password.map(str::as_bytes));
        let mut entry = archive.by_index_with_options(index, options)?;
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        let value = if entry.is_dir() {
            CompressedEntry::Directory
        } else if entry.is_symlink() {
            CompressedEntry::Symlink(PathBuf::from(OsString::from_vec(bytes)))
        } else {
            CompressedEntry::File(bytes)
        };
        let name = PathBuf::from(&*entry.name().expect("member name"));
        insert_once(&mut entries, name, value);
    }
    Ok(entries)
}

fn tar_entry<R: Read>(entry: &mut tar::Entry<'_, R>) -> Result<CompressedEntry, Box<dyn Error>> {
    let kind = entry.header().entry_type();
    if kind.is_dir() {
        return Ok(CompressedEntry::Directory);
    }
    if kind.is_symlink() {
        let target = entry.link_name()?.ok_or("Missing link target")?;
        return Ok(CompressedEntry::Symlink(target.into_owned()));
    }
    assert!(kind.is_file());
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes)?;
    Ok(CompressedEntry::File(bytes))
}

pub(super) fn tar_reader(file: fs::File, format: ArchiveFormat) -> Box<dyn Read> {
    if format == ArchiveFormat::TarGz {
        Box::new(flate2::read::GzDecoder::new(file))
    } else {
        Box::new(file)
    }
}

fn read_tar_entries(file: fs::File, format: ArchiveFormat) -> Result<Entries, Box<dyn Error>> {
    let mut entries = BTreeMap::new();
    for entry in tar::Archive::new(tar_reader(file, format)).entries()? {
        let mut entry = entry?;
        let value = tar_entry(&mut entry)?;
        insert_once(&mut entries, entry.path()?.into_owned(), value);
    }
    Ok(entries)
}

fn read_sevenz_entries(file: fs::File, password: Option<&str>) -> Result<Entries, Box<dyn Error>> {
    let mut entries = BTreeMap::new();
    let mut archive = sevenz_rust2::ArchiveReader::new(
        file,
        password
            .map(sevenz_rust2::Password::from)
            .unwrap_or_default(),
    )?;
    archive.for_each_entries(|entry, reader| {
        let value = if entry.is_directory() {
            CompressedEntry::Directory
        } else {
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes)?;
            CompressedEntry::File(bytes)
        };
        insert_once(&mut entries, PathBuf::from(entry.name()), value);
        Ok(true)
    })?;
    Ok(entries)
}

pub(super) fn read_compressed_entries(
    path: &Path,
    format: ArchiveFormat,
    password: Option<&str>,
) -> Result<Entries, Box<dyn Error>> {
    let file = fs::File::open(path)?;
    match format {
        ArchiveFormat::Zip => read_zip_entries(file, password),
        ArchiveFormat::Tar | ArchiveFormat::TarGz => read_tar_entries(file, format),
        ArchiveFormat::SevenZ => read_sevenz_entries(file, password),
        ArchiveFormat::Rar => Err("RAR compression is not supported".into()),
    }
}

#[test]
fn compression_preserves_links_in_zip_and_tar() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    fs::create_dir_all(source.join("nested"))?;
    fs::create_dir(source.join("empty"))?;
    fs::write(source.join("file.txt"), b"file contents")?;
    fs::write(source.join("nested/child.txt"), b"child contents")?;
    let mut expected = BTreeMap::from([
        (PathBuf::from("source"), CompressedEntry::Directory),
        (PathBuf::from("source/nested"), CompressedEntry::Directory),
        (PathBuf::from("source/empty"), CompressedEntry::Directory),
        (
            PathBuf::from("source/file.txt"),
            CompressedEntry::File(b"file contents".to_vec()),
        ),
        (
            PathBuf::from("source/nested/child.txt"),
            CompressedEntry::File(b"child contents".to_vec()),
        ),
    ]);
    for (name, target) in [
        ("file-link", "file.txt"),
        ("directory-link", "nested"),
        ("broken-link", "missing.txt"),
        ("current-directory-link", "."),
    ] {
        std::os::unix::fs::symlink(target, source.join(name))?;
        expected.insert(
            PathBuf::from("source").join(name),
            CompressedEntry::Symlink(PathBuf::from(target)),
        );
    }
    let selected_link = root.path().join("selected-link");
    std::os::unix::fs::symlink("source/nested", &selected_link)?;
    expected.insert(
        PathBuf::from("selected-link"),
        CompressedEntry::Symlink(PathBuf::from("source/nested")),
    );
    let entries = [source, selected_link];
    assert_eq!(
        inspect_archive_sources(&entries, &never_cancelled())?.files,
        7
    );
    for (format, password) in [
        (ArchiveFormat::Zip, None),
        (ArchiveFormat::Zip, Some("test-password")),
        (ArchiveFormat::Tar, None),
        (ArchiveFormat::TarGz, None),
    ] {
        let archive = root.path().join("archive");
        assert_eq!(
            write_compression_fixture(&archive, &entries, format, password)?,
            7
        );
        assert_eq!(
            read_compressed_entries(&archive, format, password)?,
            expected,
            "{format:?}"
        );
    }
    Ok(())
}

#[test]
fn seven_z_compression_preserves_mixed_methods_and_empty_directories() -> Result<(), Box<dyn Error>>
{
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    fs::create_dir_all(source.join("empty"))?;
    fs::write(source.join("one.txt"), b"one")?;
    fs::write(source.join("two.png"), b"two")?;
    let expected = BTreeMap::from([
        (PathBuf::from("source"), CompressedEntry::Directory),
        (PathBuf::from("source/empty"), CompressedEntry::Directory),
        (
            PathBuf::from("source/one.txt"),
            CompressedEntry::File(b"one".to_vec()),
        ),
        (
            PathBuf::from("source/two.png"),
            CompressedEntry::File(b"two".to_vec()),
        ),
    ]);
    for password in [None, Some("test-password")] {
        let archive = root.path().join("archive");
        assert_eq!(
            write_compression_fixture(
                &archive,
                std::slice::from_ref(&source),
                ArchiveFormat::SevenZ,
                password
            )?,
            2
        );
        assert_eq!(
            read_compressed_entries(&archive, ArchiveFormat::SevenZ, password)?,
            expected,
        );
        assert_seven_z_methods(&archive, password, "source/one.txt", false)?;
        assert_seven_z_methods(&archive, password, "source/two.png", true)?;
        if password.is_some() {
            for wrong_password in [None, Some("wrong-password")] {
                assert!(
                    read_compressed_entries(&archive, ArchiveFormat::SevenZ, wrong_password)
                        .is_err()
                );
            }
        }
    }
    Ok(())
}

#[test]
fn compression_handles_non_utf8_link_targets_without_loss() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let link = root.path().join("link");
    let target = PathBuf::from(OsString::from_vec(b"target-\xff".to_vec()));
    std::os::unix::fs::symlink(&target, &link)?;
    for format in [ArchiveFormat::Tar, ArchiveFormat::TarGz] {
        let archive = root.path().join("archive");
        write_compression_fixture(&archive, std::slice::from_ref(&link), format, None)?;
        assert_eq!(
            read_compressed_entries(&archive, format, None)?,
            BTreeMap::from([(
                PathBuf::from("link"),
                CompressedEntry::Symlink(target.clone())
            )])
        );
    }
    let error = write_compression_fixture(
        &root.path().join("archive.zip"),
        &[link],
        ArchiveFormat::Zip,
        None,
    )
    .expect_err("ZIP must reject a link target it cannot encode");
    assert!(error.contains("non-UTF-8 link target"));
    Ok(())
}

#[test]
fn compression_accepts_a_symlink_in_the_parent_path() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let actual = root.path().join("actual");
    let alias = root.path().join("alias");
    fs::create_dir_all(actual.join("tree"))?;
    fs::write(actual.join("tree/file.txt"), b"contents")?;
    std::os::unix::fs::symlink("actual", &alias)?;
    let entries = [alias.join("tree")];
    let expected = BTreeMap::from([
        (PathBuf::from("tree"), CompressedEntry::Directory),
        (
            PathBuf::from("tree/file.txt"),
            CompressedEntry::File(b"contents".to_vec()),
        ),
    ]);

    for format in [
        ArchiveFormat::Zip,
        ArchiveFormat::Tar,
        ArchiveFormat::TarGz,
        ArchiveFormat::SevenZ,
    ] {
        let archive = root.path().join("archive");
        assert_eq!(
            inspect_archive_sources(&entries, &never_cancelled())?.files,
            1
        );
        assert_eq!(
            write_compression_fixture(&archive, &entries, format, None)?,
            1
        );
        assert_eq!(
            read_compressed_entries(&archive, format, None)?,
            expected,
            "{format:?}"
        );
    }
    Ok(())
}

#[test]
fn zip_and_seven_z_refuse_non_utf8_names_instead_of_mangling_them() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root
        .path()
        .join(OsString::from_vec(b"name-\xff.txt".to_vec()));
    fs::write(&source, b"contents")?;
    for format in [ArchiveFormat::Zip, ArchiveFormat::SevenZ] {
        let archive = root.path().join("archive.out");
        let error =
            write_compression_fixture(&archive, std::slice::from_ref(&source), format, None)
                .expect_err("a non-UTF-8 name cannot be stored losslessly");
        assert!(error.contains("non-UTF-8 name"), "{format:?}: {error}");
    }
    let archive = root.path().join("archive.tar");
    write_compression_fixture(
        &archive,
        std::slice::from_ref(&source),
        ArchiveFormat::Tar,
        None,
    )?;
    Ok(())
}

#[test]
fn encrypted_seven_z_archives_are_still_compressed() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("zeros.bin");
    fs::write(&source, vec![0u8; 4 << 20])?;
    let plain = root.path().join("plain.7z");
    let encrypted = root.path().join("encrypted.7z");
    write_compression_fixture(
        &plain,
        std::slice::from_ref(&source),
        ArchiveFormat::SevenZ,
        None,
    )?;
    write_compression_fixture(
        &encrypted,
        std::slice::from_ref(&source),
        ArchiveFormat::SevenZ,
        Some("secret"),
    )?;
    let plain_len = fs::metadata(&plain)?.len();
    let encrypted_len = fs::metadata(&encrypted)?.len();
    assert!(
        encrypted_len < 1 << 20,
        "encrypted archive should compress ({encrypted_len} bytes, plain {plain_len} bytes)"
    );
    Ok(())
}
