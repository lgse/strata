// SPDX-License-Identifier: MIT

use super::*;

fn rename_batch_request(id: u64, items: Vec<(PathBuf, &str)>) -> RenameBatchRequest {
    RenameBatchRequest {
        id: OperationRequestId(id),
        items: items
            .into_iter()
            .map(|(path, new_name)| RenameBatchItem {
                location: Location::local(path),
                new_name: new_name.to_owned(),
            })
            .collect(),
    }
}

fn run_rename_batch(request: RenameBatchRequest) -> Rc<RefCell<Vec<OperationEvent>>> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.rename_batch(
        request,
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    wait_for_operation(&events, |event| {
        matches!(
            event,
            OperationEvent::RenamedBatch { .. }
                | OperationEvent::Cancelled { .. }
                | OperationEvent::Failed { .. }
        )
    });
    events
}

fn assert_no_staging_leftovers(root: &Path) -> Result<(), Box<dyn Error>> {
    let leftovers: Vec<_> = fs::read_dir(root)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".strata-batch-")
        })
        .collect();
    assert!(leftovers.is_empty(), "staging names leaked: {leftovers:?}");
    Ok(())
}

#[test]
fn batch_rename_applies_a_chain_onto_another_items_source() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let photo = root.path().join("photo.jpg");
    let first = root.path().join("vacation 00001.jpg");
    fs::write(&photo, b"photo")?;
    fs::write(&first, b"first")?;

    let events = run_rename_batch(rename_batch_request(
        60,
        vec![
            (photo.clone(), "vacation 00001.jpg"),
            (first.clone(), "vacation 00002.jpg"),
        ],
    ));

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::RenamedBatch { renamed, errors, .. })
            if renamed.len() == 2 && errors.is_empty()
    ));
    assert!(!photo.exists());
    assert_eq!(fs::read(root.path().join("vacation 00001.jpg"))?, b"photo");
    assert_eq!(fs::read(root.path().join("vacation 00002.jpg"))?, b"first");
    assert_no_staging_leftovers(root.path())
}

#[test]
fn batch_rename_swaps_two_names() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let first = root.path().join("a.txt");
    let second = root.path().join("b.txt");
    fs::write(&first, b"aaa")?;
    fs::write(&second, b"bbb")?;

    let events = run_rename_batch(rename_batch_request(
        61,
        vec![(first.clone(), "b.txt"), (second.clone(), "a.txt")],
    ));

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::RenamedBatch { renamed, errors, .. })
            if renamed.len() == 2 && errors.is_empty()
    ));
    assert_eq!(fs::read(root.path().join("a.txt"))?, b"bbb");
    assert_eq!(fs::read(root.path().join("b.txt"))?, b"aaa");
    assert_no_staging_leftovers(root.path())
}

#[test]
fn batch_rename_rejects_two_items_claiming_one_name() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let first = root.path().join("a.txt");
    let second = root.path().join("b.txt");
    fs::write(&first, b"aaa")?;
    fs::write(&second, b"bbb")?;

    let events = run_rename_batch(rename_batch_request(
        62,
        vec![(first.clone(), "same.txt"), (second.clone(), "same.txt")],
    ));

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::RenamedBatch { renamed, errors, .. })
            if renamed.is_empty() && errors.len() == 2
    ));
    assert_eq!(fs::read(&first)?, b"aaa");
    assert_eq!(fs::read(&second)?, b"bbb");
    assert!(!root.path().join("same.txt").exists());
    Ok(())
}

#[test]
fn batch_rename_still_refuses_a_target_held_by_an_outsider() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("a.txt");
    let outsider = root.path().join("keep.txt");
    fs::write(&source, b"mine")?;
    fs::write(&outsider, b"keep")?;

    let events = run_rename_batch(rename_batch_request(63, vec![(source.clone(), "keep.txt")]));

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::RenamedBatch { renamed, errors, .. })
            if renamed.is_empty() && errors.len() == 1
    ));
    assert_eq!(fs::read(&source)?, b"mine");
    assert_eq!(fs::read(&outsider)?, b"keep");
    Ok(())
}

#[test]
fn cancelled_batch_rename_leaves_sources_in_place() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let first = root.path().join("a.txt");
    let second = root.path().join("b.txt");
    fs::write(&first, b"aaa")?;
    fs::write(&second, b"bbb")?;

    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let operation = LocalOperationProvider.rename_batch(
        rename_batch_request(
            65,
            vec![(first.clone(), "b.txt"), (second.clone(), "a.txt")],
        ),
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    drop(operation);
    wait_for_operation(&events, |event| {
        matches!(
            event,
            OperationEvent::Cancelled { .. } | OperationEvent::RenamedBatch { .. }
        )
    });

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::Cancelled { .. })
    ));
    assert_eq!(fs::read(&first)?, b"aaa");
    assert_eq!(fs::read(&second)?, b"bbb");
    assert_no_staging_leftovers(root.path())
}

#[test]
fn batch_rename_reports_an_invalid_name() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .map_err(|error| error.to_string())?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("a.txt");
    fs::write(&source, b"mine")?;

    let events = run_rename_batch(rename_batch_request(64, vec![(source.clone(), "")]));

    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::RenamedBatch { renamed, errors, .. })
            if renamed.is_empty() && errors.len() == 1
    ));
    assert_eq!(fs::read(&source)?, b"mine");
    Ok(())
}
