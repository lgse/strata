// SPDX-License-Identifier: MIT

use super::*;

fn operation_activity() -> file_activity::Activity {
    file_activity::begin(file_activity::OperationScope::default())
}

#[test]
fn background_folder_listings_skip_a_folder_an_operation_fences() {
    let root = tempfile::tempdir().expect("fixture directory");
    let folder = root.path().join("folder");
    fs::create_dir(&folder).expect("watched folder");
    fs::write(folder.join("#1234"), b"").expect("numbered name");
    let identity = folder_identity(&folder).expect("folder identity");
    let listings: [(&str, &dyn Fn() -> bool); 3] = [
        ("numbered names", &|| {
            numbered_names_in(&folder)
                .is_ok_and(|names| names.contains(std::ffi::OsStr::new("#1234")))
        }),
        ("departure probe", &|| {
            native_entry_with_identity(
                root.path(),
                identity,
                &MountTable::parse(b""),
                &Progress::unlaned(),
            )
            .as_ref()
                == Some(&folder)
        }),
        ("background re-list", &|| {
            let events = run_enumerate(background_request(&folder));
            batched_entry_count(&events) == 1 && finished_truncated(&events) == Some(false)
        }),
    ];
    let activity = operation_activity();

    for (listing, lists) in &listings {
        assert!(lists(), "{listing}");
    }
    let _fence = activity.fence([root.path().to_owned()]);
    for (listing, lists) in &listings {
        assert!(!lists(), "{listing} under a fence");
    }
}

#[test]
fn a_background_relist_stops_for_an_operation_that_changes_its_folder() -> Result<(), Box<dyn Error>>
{
    use crate::adapters::{FsOp, FsQuirks, Quirks};

    let _serial = serial();
    let root = tempfile::tempdir()?;
    let folder = root.path().join("folder");
    fs::create_dir(&folder)?;
    fs::write(folder.join("file"), b"content")?;
    let gate = Gate::default();
    let _scope = FsQuirks::install(
        &folder,
        Quirks {
            before: vec![(FsOp::ListChunks, "*", 0, gate.racer())],
            ..Quirks::default()
        },
    );
    let events = Events::default();
    let _load = enumerate_into(&events, background_request(&folder));

    drain_until(|| gate.reached());
    let activity = operation_activity();
    let fence = activity.fence([folder.clone()]);
    fence.stop_reads();
    gate.release();
    drain_until(|| outcome_of(&events, 1).is_some());

    assert_eq!(
        outcome_of(&events, 1),
        Some(Err(file_activity::HELD_FOR_OPERATION.to_owned()))
    );
    Ok(())
}
