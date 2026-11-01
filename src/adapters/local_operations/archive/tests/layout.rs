// SPDX-License-Identifier: MIT

//! Where an extraction puts an archive's roots: as they are, or in a folder of its own.

use super::*;

fn expect_listings(expected: &[(PathBuf, &[&str])], context: &str) -> Result<(), Box<dyn Error>> {
    for (folder, names) in expected {
        assert_eq!(
            entry_names(folder)?,
            *names,
            "{context}: {}",
            folder.display()
        );
    }
    Ok(())
}

/// `bundle` archives in every format, each holding `a.txt` and `dir/b.txt`.
fn write_bundles(root: &Path) -> Result<[PathBuf; 4], Box<dyn Error>> {
    let archives = [
        root.join("bundle.zip"),
        root.join("bundle.tar"),
        root.join("bundle.tar.gz"),
        root.join("bundle.7z"),
    ];
    write_7z_entries(&archives[3], &[("a.txt", b"a"), ("dir/b.txt", b"b")])?;
    write_zip_stored(&archives[0], &[("a.txt", b"a"), ("dir/b.txt", b"b")])?;
    for (archive, gzip) in [(&archives[1], false), (&archives[2], true)] {
        write_tar_entries(
            archive,
            &[
                (tar::EntryType::Regular, "a.txt", b"a"),
                (tar::EntryType::Regular, "dir/b.txt", b"b"),
            ],
            gzip,
        )?;
    }
    Ok(archives)
}

#[test]
fn spilled_members_bundle_under_the_archive_stem() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    for archive in &write_bundles(root.path())? {
        let context = format!("{archive:?}");
        let destination = tempfile::tempdir()?;
        let destination = destination.path();
        fs::write(destination.join("a.txt"), b"EXISTING")?;
        fs::create_dir(destination.join("dir"))?;
        fs::write(destination.join("dir/keep.txt"), b"keep")?;

        let events = run_extraction(extract_request(1, archive, destination));

        expect_extracted_as(&events, "bundle");
        expect_contents(
            &[
                (destination.join("bundle/a.txt"), b"a"),
                (destination.join("bundle/dir/b.txt"), b"b"),
                (destination.join("a.txt"), b"EXISTING"),
            ],
            &context,
        )?;
        expect_listings(
            &[
                (destination.join("bundle"), &["a.txt", "dir"]),
                (destination.join("dir"), &["keep.txt"]),
                (destination.to_path_buf(), &["a.txt", "bundle", "dir"]),
            ],
            &context,
        )?;
    }
    Ok(())
}

#[test]
fn single_root_extraction_lands_verbatim() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;

    let archive = root.path().join("note.zip");
    write_zip_stored(&archive, &[("readme.txt", b"hi")])?;
    let events = run_extraction(extract_request(2, &archive, &destination));
    expect_extracted_as(&events, "readme.txt");
    assert_eq!(fs::read(destination.join("readme.txt"))?, b"hi");
    assert!(!destination.join("note").exists());

    let archive = root.path().join("foldered.zip");
    write_zip_stored(&archive, &[("folder/a.txt", b"a"), ("folder/b.txt", b"b")])?;
    let events = run_extraction(extract_request(3, &archive, &destination));
    expect_extracted_as(&events, "folder");
    assert_eq!(fs::read(destination.join("folder/a.txt"))?, b"a");
    assert!(!destination.join("foldered").exists());

    let archive = root.path().join("note.zip");
    let events = run_extraction(extract_request(4, &archive, &destination));
    expect_extracted_as(&events, "readme (2).txt");
    expect_contents(
        &[
            (destination.join("readme.txt"), b"hi"),
            (destination.join("readme (2).txt"), b"hi"),
        ],
        "extracted again",
    )?;
    assert!(!destination.join("note").exists());
    assert_eq!(destination.read_dir()?.count(), 3);
    Ok(())
}

#[test]
fn extraction_reports_published_locations_for_undo() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;
    fs::write(destination.join("readme.txt"), b"original")?;

    let archive = root.path().join("note.zip");
    write_zip_stored(&archive, &[("readme.txt", b"extracted")])?;
    let events = run_extraction(extract_request(7, &archive, &destination));

    let Some(OperationEvent::Extracted { created, .. }) = events.last() else {
        panic!("expected extraction, got {events:?}");
    };
    assert_eq!(
        created.as_slice(),
        &[Location::local(destination.join("readme (2).txt"))],
        "undo must target the published rename, not the pre-existing file"
    );
    assert_eq!(fs::read(destination.join("readme.txt"))?, b"original");
    Ok(())
}

#[test]
fn a_bundle_may_share_its_name_with_an_extracted_root() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;
    let archive = root.path().join("photos.zip");
    write_zip_stored(
        &archive,
        &[("photos/inner.txt", b"inner"), ("extra.txt", b"extra")],
    )?;

    let events = run_extraction(extract_request(4, &archive, &destination));

    expect_extracted_as(&events, "photos");
    expect_contents(
        &[
            (destination.join("photos/photos/inner.txt"), b"inner"),
            (destination.join("photos/extra.txt"), b"extra"),
        ],
        "photos",
    )?;
    assert_eq!(destination.read_dir()?.count(), 1);
    Ok(())
}

#[test]
fn bundled_extraction_reserves_a_fresh_name_against_existing_entries() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let destination = root.path().join("destination");
    fs::create_dir(&destination)?;
    fs::write(destination.join("bundle"), b"existing file")?;
    std::os::unix::fs::symlink("missing", destination.join("bundle (1)"))?;
    let archive = root.path().join("bundle.zip");
    write_zip_stored(&archive, &[("a.txt", b"a"), ("b.txt", b"b")])?;

    let events = run_extraction(extract_request(5, &archive, &destination));

    expect_extracted_as(&events, "bundle (2)");
    expect_contents(
        &[
            (destination.join("bundle"), b"existing file"),
            (destination.join("bundle (2)/a.txt"), b"a"),
        ],
        "bundle",
    )?;
    assert_eq!(
        fs::read_link(destination.join("bundle (1)"))?,
        Path::new("missing")
    );
    Ok(())
}
