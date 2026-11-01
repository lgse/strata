// SPDX-License-Identifier: MIT

use std::{
    fs,
    os::unix::{ffi::OsStringExt, fs::symlink},
};

use super::*;

#[test]
fn foreign_folder_validation_preserves_symlinks_and_native_names_and_rejects_mixed_or_missing_sources()
 {
    let directory = tempfile::tempdir().expect("fixture");
    let folder = directory
        .path()
        .join(std::ffi::OsString::from_vec(b"folder-\xff".to_vec()));
    let link = directory.path().join("alias");
    let file = directory.path().join("file");
    fs::create_dir(&folder).expect("folder");
    symlink(&folder, &link).expect("directory symlink");
    fs::write(&file, b"file").expect("file");
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            context.block_on(async {
                let folder = Location::local(folder);
                let link = Location::local(link);
                let file = Location::local(file);
                let alias = folder_names(vec![link.clone()]).await;
                assert_eq!(alias, Some(vec![(link.clone(), "alias".to_owned())]));
                for (locations, accepted) in [
                    (vec![], false),
                    (vec![folder.clone(), link.clone()], true),
                    (vec![file.clone()], false),
                    (vec![folder, file], false),
                    (
                        vec![Location::local(directory.path().join("missing"))],
                        false,
                    ),
                ] {
                    let folders = folder_names(locations.clone()).await;
                    assert_eq!(
                        folders.map(|folders| folders
                            .into_iter()
                            .map(|(location, _)| location)
                            .collect()),
                        accepted.then_some(locations.clone()),
                        "{locations:?}"
                    );
                }
                fs::remove_file(directory.path().join("alias")).expect("remove alias");
                symlink(
                    directory.path().join("missing"),
                    directory.path().join("alias"),
                )
                .expect("broken link");
                assert!(folder_names(vec![link]).await.is_none());
            })
        })
        .expect("private main context");
}
