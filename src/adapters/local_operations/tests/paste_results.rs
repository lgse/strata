// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn a_copy_reports_the_destination_it_created() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("photo.jpg");
    let destination = root.path().join("album");
    fs::write(&source, b"original-content")?;
    fs::create_dir(&destination)?;

    let created = run_paste_collecting_created(PasteRequest {
        id: OperationRequestId(70),
        destination: Location::local(&destination),
        items: vec![PasteItem {
            source: Location::local(&source),
            conflict: TransferConflict::FailIfExists,
        }],
        move_sources: false,
    })?;

    assert_eq!(
        created.into_iter().flatten().collect::<Vec<_>>(),
        vec![Location::local(destination.join("photo.jpg"))]
    );
    Ok(())
}

#[test]
fn duplicating_a_file_preserves_contents_and_reports_the_generated_name()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let destination = root.path().to_path_buf();
    let source = destination.join("photo.jpg");
    fs::write(&source, b"original-content")?;

    let created = run_paste_collecting_created(PasteRequest {
        id: OperationRequestId(71),
        destination: Location::local(&destination),
        items: vec![PasteItem {
            source: Location::local(&source),
            conflict: TransferConflict::FailIfExists,
        }],
        move_sources: false,
    })?;

    assert_eq!(
        created.into_iter().flatten().collect::<Vec<_>>(),
        vec![Location::local(destination.join("photo (1).jpg"))]
    );
    assert_eq!(fs::read(&source)?, b"original-content");
    assert_eq!(
        fs::read(destination.join("photo (1).jpg"))?,
        b"original-content"
    );
    Ok(())
}

#[test]
fn a_copy_that_replaces_an_existing_item_reports_its_destination() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("photo.jpg");
    let destination = root.path().join("album");
    fs::write(&source, b"new-content")?;
    fs::create_dir(&destination)?;
    fs::write(destination.join("photo.jpg"), b"old-content")?;

    let created = run_paste_collecting_created(PasteRequest {
        id: OperationRequestId(72),
        destination: Location::local(&destination),
        items: vec![PasteItem {
            source: Location::local(&source),
            conflict: TransferConflict::ReplaceExisting,
        }],
        move_sources: false,
    })?;

    assert_eq!(
        created.into_iter().flatten().collect::<Vec<_>>(),
        vec![Location::local(destination.join("photo.jpg"))]
    );
    Ok(())
}

#[test]
fn a_move_reports_no_created_destination() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("photo.jpg");
    let destination = root.path().join("album");
    fs::write(&source, b"original-content")?;
    fs::create_dir(&destination)?;

    let created = run_paste_collecting_created(PasteRequest {
        id: OperationRequestId(73),
        destination: Location::local(&destination),
        items: vec![PasteItem {
            source: Location::local(&source),
            conflict: TransferConflict::FailIfExists,
        }],
        move_sources: true,
    })?;

    assert!(created.into_iter().flatten().next().is_none());
    Ok(())
}
