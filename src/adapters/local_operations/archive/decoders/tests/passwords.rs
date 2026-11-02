// SPDX-License-Identifier: MIT

//! Password handling across encryption schemes, and retries after a refusal.

use super::super::{MAYBE_BAD_PASSWORD, PASSWORD_REQUIRED};
use super::*;
use crate::services::INCORRECT_ARCHIVE_PASSWORD;

const ZIPCRYPTO_PASSWORD: &[u8] = b"zipsecret";

fn write_zipcrypto(
    path: &Path,
    contents: &[u8],
    compression: zip::CompressionMethod,
) -> Result<(), Box<dyn Error>> {
    use zip::unstable::write::FileOptionsExt;
    let mut writer = zip::ZipWriter::new(fs::File::create(path)?);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(compression)
        .with_deprecated_encryption(ZIPCRYPTO_PASSWORD)?;
    writer.start_file("some.txt", options)?;
    writer.write_all(contents)?;
    writer.finish()?;
    Ok(())
}

fn zipcrypto_crc_collision(archive_path: &Path) -> Result<String, Box<dyn Error>> {
    for candidate in 0..4096u32 {
        let password = candidate.to_string();
        let file = fs::File::open(archive_path)?;
        let mut archive = zip::ZipArchive::new(file)?;
        let options = zip::read::ZipReadOptions::new().password(Some(password.as_bytes()));
        let mut entry = match archive.by_index_with_options(0, options) {
            Err(zip::result::ZipError::InvalidPassword) => continue,
            Err(error) => return Err(error.into()),
            Ok(entry) => entry,
        };
        let mut buf = Vec::new();
        if entry.read_to_end(&mut buf).is_err() {
            return Ok(password);
        }
    }
    Err("no ZipCrypto CRC collision in 0..4096".into())
}

fn zipcrypto_fixture(root: &Path) -> Result<std::path::PathBuf, Box<dyn Error>> {
    let archive = root.join("password.zip");
    write_zipcrypto(
        &archive,
        b"hello from zipcrypto",
        zip::CompressionMethod::Stored,
    )?;
    Ok(archive)
}

fn zipcrypto_rejected_password(path: &Path) -> Result<String, Box<dyn Error>> {
    // ZipCrypto's one-byte header check can accept an arbitrary wrong password.
    for candidate in 0..4096 {
        let password = format!("wrong-{candidate}");
        let mut archive = zip::ZipArchive::new(fs::File::open(path)?)?;
        let options = zip::read::ZipReadOptions::new().password(Some(password.as_bytes()));
        match archive.by_index_with_options(0, options) {
            Err(zip::result::ZipError::InvalidPassword) => return Ok(password),
            Err(error) => return Err(error.into()),
            Ok(_) => {}
        }
    }
    Err("no header-rejected ZipCrypto password found".into())
}

fn write_partly_encrypted_zip(path: &Path, password: &str) -> Result<(), Box<dyn Error>> {
    let mut writer = zip::ZipWriter::new(fs::File::create(path)?);
    writer.start_file("README", zip::write::SimpleFileOptions::default())?;
    writer.write_all(b"plain")?;
    writer.start_file(
        "secret.txt",
        zip::write::SimpleFileOptions::default()
            .with_aes_encryption(zip::AesMode::Aes256, password),
    )?;
    writer.write_all(b"secret")?;
    writer.finish()?;
    Ok(())
}

/// Decoding the ZIP with `password` fails with `expected` into a fresh, still empty folder.
fn assert_zip_refuses(
    archive: &Path,
    password: Option<&str>,
    expected: ArchiveError,
    context: &str,
) -> Result<(), Box<dyn Error>> {
    let destination = tempfile::tempdir()?;
    match Decoder::with_password(ArchiveFormat::Zip, password).run(archive, destination.path()) {
        Err(error) => assert_eq!(error, expected, "{context}"),
        Ok(outcome) => panic!("{context}: {outcome:?}"),
    }
    assert_empty(destination.path(), context)?;
    Ok(())
}

/// Decoding the ZIP with `password` into a fresh folder places `name` with `contents`.
fn assert_zip_extracts(
    archive: &Path,
    password: Option<&str>,
    name: &str,
    contents: &[u8],
) -> Result<(), Box<dyn Error>> {
    let destination = tempfile::tempdir()?;
    Decoder::with_password(ArchiveFormat::Zip, password).extract(archive, destination.path())?;
    assert_eq!(fs::read(destination.path().join(name))?, contents);
    Ok(())
}

fn zipcrypto_collision_is_retryable(
    compression: zip::CompressionMethod,
    contents: &[u8],
) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let archive = root.path().join("password.zip");
    write_zipcrypto(&archive, contents, compression)?;
    let collision = zipcrypto_crc_collision(&archive)?;

    assert_zip_refuses(
        &archive,
        Some(&collision),
        ArchiveError::IncorrectPassword(MAYBE_BAD_PASSWORD.to_owned()),
        &format!("ZipCrypto {compression:?} collision {collision} extracted"),
    )?;
    assert_zip_extracts(&archive, Some("zipsecret"), "some.txt", contents)
}

#[test]
fn encrypted_decoders_fail_without_the_correct_password() -> Result<(), Box<dyn Error>> {
    for format in [ArchiveFormat::Zip, ArchiveFormat::SevenZ] {
        let root = tempfile::tempdir()?;
        let source = root.path().join("file.txt");
        fs::write(&source, b"contents")?;
        let archive = root.path().join("archive");
        write_compression_fixture(&archive, &[source], format, Some("test-password"))?;
        for password in [None, Some("wrong-password")] {
            let destination = tempfile::tempdir()?;
            let result = Decoder::with_password(format, password).run(&archive, destination.path());
            assert!(
                match password {
                    None => matches!(result, Err(ArchiveError::PasswordRequired(_))),
                    Some(_) => matches!(result, Err(ArchiveError::IncorrectPassword(_))),
                },
                "{format:?} {password:?}: {result:?}"
            );
            assert_empty(destination.path(), "")?;
        }
    }
    Ok(())
}

#[test]
fn zipcrypto_header_collision_is_retryable() -> Result<(), Box<dyn Error>> {
    zipcrypto_collision_is_retryable(zip::CompressionMethod::Stored, b"hello from zipcrypto")
}

#[test]
fn zipcrypto_deflated_header_collision_is_retryable() -> Result<(), Box<dyn Error>> {
    zipcrypto_collision_is_retryable(
        zip::CompressionMethod::Deflated,
        &b"hello from zipcrypto\n".repeat(64),
    )
}

#[test]
fn zipcrypto_wrong_password_stays_invalid_password() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let archive = zipcrypto_fixture(root.path())?;
    let password = zipcrypto_rejected_password(&archive)?;
    assert_zip_refuses(
        &archive,
        Some(&password),
        ArchiveError::IncorrectPassword(INCORRECT_ARCHIVE_PASSWORD.to_owned()),
        "ZipCrypto accepted a non-colliding wrong password",
    )
}

#[test]
fn zipcrypto_without_password_still_requires_one() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let archive = zipcrypto_fixture(root.path())?;
    assert_zip_refuses(
        &archive,
        None,
        ArchiveError::PasswordRequired(PASSWORD_REQUIRED.to_owned()),
        "ZipCrypto extracted without a password",
    )
}

#[test]
fn aes_zip_wrong_password_stays_invalid_password() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("file.txt");
    fs::write(&source, b"contents")?;
    let archive = root.path().join("archive.zip");
    write_compression_fixture(
        &archive,
        &[source],
        ArchiveFormat::Zip,
        Some("test-password"),
    )?;

    assert_zip_refuses(
        &archive,
        Some("wrong"),
        ArchiveError::IncorrectPassword(INCORRECT_ARCHIVE_PASSWORD.to_owned()),
        "AES zip accepted a wrong password",
    )?;
    assert_zip_extracts(&archive, Some("test-password"), "file.txt", b"contents")
}

#[test]
fn wrong_password_for_content_encrypted_7z_is_retryable() -> Result<(), Box<dyn Error>> {
    // Generated with `7z a -psecret -mhe=off` using 7-Zip 26.02.
    let archive = include_bytes!("../fixtures/content-encrypted.7z");
    let wrong_destination = tempfile::tempdir()?;
    let Err(error) = extract_7z_from_reader(
        Cursor::new(archive),
        wrong_destination.path(),
        "archive",
        "wrong".into(),
        &Arc::new(AtomicUsize::new(0)),
        &never_cancelled(),
        &mut TransferReport::default(),
    ) else {
        panic!("a wrong password must fail extraction");
    };
    assert_eq!(
        error,
        ArchiveError::IncorrectPassword(MAYBE_BAD_PASSWORD.to_owned())
    );
    assert_empty(wrong_destination.path(), "")?;

    let correct_destination = tempfile::tempdir()?;
    completed_extract(extract_7z_from_reader(
        Cursor::new(archive),
        correct_destination.path(),
        "archive",
        "secret".into(),
        &Arc::new(AtomicUsize::new(0)),
        &never_cancelled(),
        &mut TransferReport::default(),
    )?)?;
    assert_eq!(
        fs::read(correct_destination.path().join("document.txt"))?,
        b"private contents"
    );
    Ok(())
}

#[test]
fn a_password_failure_discards_unencrypted_members_so_the_retry_keeps_the_name()
-> Result<(), Box<dyn Error>> {
    for (attempt, expected) in [
        (None, PASSWORD_REQUIRED),
        (Some("wrong-password"), INCORRECT_ARCHIVE_PASSWORD),
    ] {
        let root = tempfile::tempdir()?;
        let archive = root.path().join("mixed.zip");
        write_partly_encrypted_zip(&archive, "test-password")?;
        let destination = root.path().join("destination");
        fs::create_dir(&destination)?;

        let Err(error) =
            Decoder::with_password(ArchiveFormat::Zip, attempt).run(&archive, &destination)
        else {
            panic!("{attempt:?} extracted the encrypted member");
        };
        assert!(error.to_string().contains(expected), "{attempt:?}: {error}");
        assert_empty(
            &destination,
            &format!("{attempt:?} left output behind: {error}"),
        )?;

        let retried = Decoder::with_password(ArchiveFormat::Zip, Some("test-password"))
            .extract(&archive, &destination)?;
        assert_eq!(retried.as_deref(), Some("mixed"), "{attempt:?}");
        assert_contents(
            &destination,
            &[("mixed/README", b"plain"), ("mixed/secret.txt", b"secret")],
            "",
        )?;
    }
    Ok(())
}

#[test]
fn damage_in_an_unencrypted_member_is_not_a_password_failure() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let archive = root.path().join("mixed.zip");
    let plain = b"plain member contents";
    let mut writer = zip::ZipWriter::new(fs::File::create(&archive)?);
    writer.start_file(
        "secret.txt",
        zip::write::SimpleFileOptions::default()
            .with_aes_encryption(zip::AesMode::Aes256, "test-password"),
    )?;
    writer.write_all(b"secret")?;
    writer.start_file(
        "README",
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
    )?;
    writer.write_all(plain)?;
    writer.finish()?;
    corrupt_stored_payload(&archive, plain, 0xff)?;
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;

    let result = Decoder::with_password(ArchiveFormat::Zip, Some("test-password"))
        .run(&archive, &destination);

    assert_failed_keeping(&result, "mixed", "");
    assert_eq!(fs::read(destination.join("mixed/secret.txt"))?, b"secret");
    assert!(!destination.join("mixed/README").exists());
    Ok(())
}
