// SPDX-License-Identifier: MIT

use super::*;

pub(super) fn assert_seven_z_methods(
    path: &Path,
    password: Option<&str>,
    name: &str,
    stored: bool,
) -> Result<(), Box<dyn Error>> {
    use sevenz_rust2::{ArchiveReader, EncoderMethod, Password};
    let reader = ArchiveReader::new(
        fs::File::open(path)?,
        password.map(Password::from).unwrap_or_default(),
    )?;
    let archive = reader.archive();
    let index = archive
        .files
        .iter()
        .position(|entry| entry.name() == name)
        .ok_or("missing member")?;
    let block = archive.stream_map.file_block_index[index].ok_or("missing stream")?;
    let ids: Vec<_> = archive.blocks[block]
        .coders
        .iter()
        .map(|coder| coder.encoder_method_id())
        .collect();
    let codec = if stored {
        EncoderMethod::COPY
    } else {
        EncoderMethod::LZMA2
    };
    let expected = if password.is_some() {
        vec![EncoderMethod::AES256_SHA256.id(), codec.id()]
    } else {
        vec![codec.id()]
    };
    assert_eq!(ids, expected, "{name}");
    Ok(())
}

#[test]
fn zip_and_seven_z_store_compressed_formats_but_compress_raw_containers()
-> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let payload = vec![b'x'; 64 * 1024];
    let cases = [
        ("movie.MP4", true),
        ("nested.tar.gz", true),
        ("nested.tgz", true),
        ("picture.png", true),
        ("raw.tar", false),
        ("raw.bmp", false),
        ("raw.wav", false),
        ("raw.avi", false),
        ("raw.iso", false),
        ("unknown", false),
    ];
    let mut entries = Vec::new();
    let mut expected = BTreeMap::new();
    for (name, _) in cases {
        let path = root.path().join(name);
        fs::write(&path, &payload)?;
        entries.push(path);
        expected.insert(PathBuf::from(name), CompressedEntry::File(payload.clone()));
    }
    for format in [ArchiveFormat::Zip, ArchiveFormat::SevenZ] {
        let archive = root.path().join("output");
        write_compression_fixture(&archive, &entries, format, None)?;
        assert_eq!(read_compressed_entries(&archive, format, None)?, expected);
        for (name, stored) in cases {
            if format == ArchiveFormat::SevenZ {
                assert_seven_z_methods(&archive, None, name, stored)?;
            } else {
                let mut reader = zip::ZipArchive::new(fs::File::open(&archive)?)?;
                let entry = reader.by_name(name)?;
                assert_eq!(
                    entry.compression(),
                    if stored {
                        zip::CompressionMethod::Stored
                    } else {
                        zip::CompressionMethod::Deflated
                    }
                );
                if stored {
                    assert_eq!(entry.compressed_size(), entry.size());
                } else {
                    assert!(entry.compressed_size() < entry.size() / 4);
                }
            }
        }
    }
    Ok(())
}

#[test]
fn seven_z_copy_only_archives_keep_data_and_headers_encrypted() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("movie.mp4");
    let empty = root.path().join("empty");
    fs::create_dir(&empty)?;
    let payload = vec![b'x'; 64 * 1024];
    fs::write(&source, &payload)?;
    for entries in [
        vec![source.clone()],
        vec![source, empty.clone()],
        vec![empty],
    ] {
        let archive = root.path().join("output.7z");
        write_compression_fixture(
            &archive,
            &entries,
            ArchiveFormat::SevenZ,
            Some("test-password"),
        )?;
        for password in ["", "wrong-password"] {
            assert!(
                sevenz_rust2::ArchiveReader::new(fs::File::open(&archive)?, password.into())
                    .is_err(),
                "header must require the password"
            );
        }
        let restored =
            read_compressed_entries(&archive, ArchiveFormat::SevenZ, Some("test-password"))?;
        if entries.iter().any(|path| path.extension().is_some()) {
            assert_seven_z_methods(&archive, Some("test-password"), "movie.mp4", true)?;
            assert_eq!(
                restored.get(Path::new("movie.mp4")),
                Some(&CompressedEntry::File(payload.clone()))
            );
        }
        if entries
            .iter()
            .any(|path| path.file_name().is_some_and(|name| name == "empty"))
        {
            assert_eq!(
                restored.get(Path::new("empty")),
                Some(&CompressedEntry::Directory)
            );
        }
    }
    Ok(())
}

#[test]
fn gzip_policy_preserves_one_stream_and_compresses_mixed_payloads() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let folder = root.path().join("source");
    fs::create_dir(&folder)?;
    let payload = vec![b'x'; 64 * 1024];
    fs::write(folder.join("movie.mp4"), &payload)?;
    let entries = [folder.clone()];
    for mixed in [false, true] {
        if mixed {
            fs::write(folder.join("notes.txt"), &payload)?;
        }
        let tar = root.path().join("output.tar");
        let gzip = root.path().join("output.tar.gz");
        write_compression_fixture(&tar, &entries, ArchiveFormat::Tar, None)?;
        write_compression_fixture(&gzip, &entries, ArchiveFormat::TarGz, None)?;
        let tar_bytes = fs::read(&tar)?;
        let gzip_bytes = fs::read(&gzip)?;
        let mut decoder = flate2::bufread::GzDecoder::new(gzip_bytes.as_slice());
        let mut decoded = Vec::new();
        decoder.read_to_end(&mut decoded)?;
        assert_eq!(decoded, tar_bytes);
        assert!(
            decoder.into_inner().is_empty(),
            "one gzip member, no trailing stream"
        );
        let restored = read_compressed_entries(&gzip, ArchiveFormat::TarGz, None)?;
        assert_eq!(
            restored.get(Path::new("source/movie.mp4")),
            Some(&CompressedEntry::File(payload.clone()))
        );
        if mixed {
            assert_eq!(
                restored.get(Path::new("source/notes.txt")),
                Some(&CompressedEntry::File(payload.clone()))
            );
            assert!(gzip_bytes.len() < tar_bytes.len() / 4);
        } else {
            assert!(
                gzip_bytes.len() >= tar_bytes.len(),
                "precompressed-only inputs use stored DEFLATE blocks"
            );
        }
    }
    Ok(())
}
