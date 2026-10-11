// SPDX-License-Identifier: MIT

use std::{
    ffi::OsStr,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

use super::*;
use crate::adapters::RemovableRoot;

fn local(path: &str) -> FolderKey {
    FolderKey::local(Path::new(path)).expect("valid local key")
}

fn volume(uuid: &str, relative: &str) -> FolderKey {
    FolderKey::on_volume(uuid, Path::new(relative)).expect("valid volume key")
}

fn sorted(key: SortKey, direction: SortDirection) -> FolderView {
    FolderView {
        sort: Some((key, direction)),
        icons_size: None,
    }
}

fn set(views: &mut FolderViews, key: &FolderKey, view: FolderView) {
    views.update(key, |current| *current = view);
}

#[test]
fn saved_views_round_trip_through_the_state_file() {
    let mut views = FolderViews::default();
    let downloads = local("/home/me/Downloads");
    let photos = volume("1234-ABCD", "DCIM/100CANON");
    let drive_root = volume("1234-ABCD", "");
    set(
        &mut views,
        &downloads,
        sorted(SortKey::Modified, SortDirection::Descending),
    );
    set(
        &mut views,
        &photos,
        FolderView {
            sort: Some((SortKey::Size, SortDirection::Ascending)),
            icons_size: Some(208),
        },
    );
    set(
        &mut views,
        &drive_root,
        FolderView {
            sort: None,
            icons_size: Some(64),
        },
    );

    let (loaded, repaired) =
        FolderViews::parse(&views.to_toml().expect("serializes")).expect("parses");

    assert!(!repaired);
    assert_eq!(loaded.len(), 3);
    for key in [&downloads, &photos, &drive_root] {
        assert_eq!(loaded.view(key), views.view(key), "{key:?}");
    }
}

#[test]
fn invalid_entries_and_fields_are_skipped_while_valid_ones_load() {
    let (views, repaired) = FolderViews::parse(
        r#"
        version = 1

        [[folder]]
        path = "/kept"
        sort = "size"
        direction = "descending"
        used = 3

        [[folder]]
        sort = "name"
        direction = "ascending"

        [[folder]]
        path = "relative/path"
        sort = "name"
        direction = "ascending"

        [[folder]]
        path = "/escapes/../parent"
        icons_size = 64

        [[folder]]
        volume = "1234-ABCD"
        path = "/absolute/on/drive"
        icons_size = 64

        [[folder]]
        path = "/unknown-sort-keeps-size"
        sort = "recency"
        direction = "descending"
        icons_size = 128

        [[folder]]
        path = "/clamped"
        icons_size = 9000

        [[folder]]
        path = "/textual-size-keeps-sort"
        sort = "type"
        direction = "ascending"
        icons_size = "huge"

        [[folder]]
        path = "/nothing-valid"
        sort = "type"
        icons_size = false

        [[folder]]
        path = "/kept"
        sort = "name"
        direction = "ascending"
        used = 1
        "#,
    )
    .expect("valid TOML loads");

    assert!(repaired);
    assert_eq!(views.len(), 4);
    assert_eq!(
        views.view(&local("/kept")),
        sorted(SortKey::Size, SortDirection::Descending),
        "a duplicate keeps the more recently used entry"
    );
    assert_eq!(
        views.view(&local("/unknown-sort-keeps-size")),
        FolderView {
            sort: None,
            icons_size: Some(128),
        }
    );
    assert_eq!(
        views.view(&local("/clamped")).icons_size,
        Some(MAX_ICONS_THUMBNAIL_SIZE)
    );
    assert_eq!(
        views.view(&local("/textual-size-keeps-sort")),
        sorted(SortKey::Type, SortDirection::Ascending)
    );
    assert_eq!(views.view(&local("/nothing-valid")), FolderView::default());
}

#[test]
fn unreadable_or_unknown_version_files_fail_instead_of_loading_empty() {
    for contents in [
        "version = 1\n[[folder]\npath = \"/broken\"",
        "version = 2\n[[folder]]\npath = \"/newer\"\nicons_size = 64",
        "version = \"1\"",
        "version = 0",
    ] {
        let error = FolderViews::parse(contents).expect_err(contents);
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{contents}");
    }
    let (views, repaired) =
        FolderViews::parse("[[folder]]\npath = \"/unversioned\"\nicons_size = 64")
            .expect("a file without a version is version 1");
    assert!(!repaired);
    assert_eq!(views.len(), 1);
    let (views, repaired) = FolderViews::parse("folder = \"not a list\"").expect("valid TOML");
    assert!(repaired);
    assert!(views.is_empty());
}

#[test]
fn clearing_every_value_removes_the_folder_and_unchanged_updates_report_nothing() {
    let mut views = FolderViews::default();
    let key = local("/music");
    let view = sorted(SortKey::Name, SortDirection::Descending);
    assert!(views.update(&key, |current| *current = view));
    assert!(!views.update(&key, |current| *current = view));
    assert!(views.update(&key, |current| current.sort = None));
    assert!(views.is_empty());
    assert!(!views.touch(&key));

    assert!(!views.clear());
    let saved = || {
        let mut saved = FolderViews::default();
        saved.update(&key, |current| *current = view);
        saved
    };
    assert!(
        views.merged_over(saved()).same_views(&saved()),
        "clearing nothing keeps what another process saves later"
    );
}

#[test]
fn defaults_prune_matching_values_without_touching_other_fields() {
    let mut views = FolderViews::default();
    let default = (SortKey::Modified, SortDirection::Descending);
    set(
        &mut views,
        &local("/only-sort"),
        sorted(default.0, default.1),
    );
    set(
        &mut views,
        &local("/sort-and-size"),
        FolderView {
            sort: Some(default),
            icons_size: Some(160),
        },
    );
    set(
        &mut views,
        &local("/other-sort"),
        sorted(SortKey::Size, SortDirection::Ascending),
    );

    assert!(views.prune_default_sort(default));

    assert_eq!(views.len(), 2);
    assert_eq!(
        views.view(&local("/sort-and-size")),
        FolderView {
            sort: None,
            icons_size: Some(160),
        }
    );
    assert_eq!(
        views.view(&local("/other-sort")),
        sorted(SortKey::Size, SortDirection::Ascending)
    );
    assert!(!views.prune_default_icons_size(96));
    assert!(views.prune_default_icons_size(160));
    assert_eq!(views.len(), 1);
}

#[test]
fn least_recently_used_folders_are_evicted_at_the_limit() {
    let mut views = FolderViews::default();
    let view = sorted(SortKey::Size, SortDirection::Ascending);
    for index in 0..FOLDER_VIEWS_LIMIT {
        set(&mut views, &local(&format!("/folder/{index}")), view);
    }
    assert!(views.touch(&local("/folder/0")));

    set(&mut views, &local("/folder/new"), view);

    assert_eq!(views.len(), FOLDER_VIEWS_LIMIT);
    assert_eq!(views.view(&local("/folder/0")), view, "recently used");
    assert_eq!(views.view(&local("/folder/new")), view);
    assert_eq!(
        views.view(&local("/folder/1")),
        FolderView::default(),
        "least recently used"
    );

    let mut contents = String::from("version = 1\n");
    for index in 0..=FOLDER_VIEWS_LIMIT {
        contents.push_str(&format!(
            "[[folder]]\npath = \"/loaded/{index}\"\nicons_size = 64\nused = {index}\n"
        ));
    }
    let (loaded, repaired) = FolderViews::parse(&contents).expect("valid TOML");
    assert!(repaired);
    assert_eq!(loaded.len(), FOLDER_VIEWS_LIMIT);
    assert_eq!(loaded.view(&local("/loaded/0")), FolderView::default());
    assert!(loaded.view(&local("/loaded/1")).icons_size.is_some());
}

#[test]
fn relocation_carries_descendants_and_replaces_stale_destinations() {
    let mut views = FolderViews::default();
    let first = sorted(SortKey::Size, SortDirection::Ascending);
    let second = sorted(SortKey::Type, SortDirection::Descending);
    let sibling = sorted(SortKey::Modified, SortDirection::Ascending);
    let stale = sorted(SortKey::Name, SortDirection::Descending);
    set(&mut views, &local("/photos"), first);
    set(&mut views, &local("/photos/2024"), second);
    set(&mut views, &local("/photos-old"), sibling);
    set(&mut views, &local("/archive"), stale);

    assert!(views.relocate(&[(local("/photos"), Some(local("/archive")))]));

    assert_eq!(views.view(&local("/archive")), first);
    assert_eq!(views.view(&local("/archive/2024")), second);
    assert_eq!(views.view(&local("/photos")), FolderView::default());
    assert_eq!(views.view(&local("/photos-old")), sibling);

    assert!(views.relocate(&[(local("/archive"), Some(volume("1234-ABCD", "backup")))]));
    assert_eq!(views.view(&volume("1234-ABCD", "backup")), first);
    assert_eq!(views.view(&volume("1234-ABCD", "backup/2024")), second);
    assert!(views.relocate(&[
        (volume("1234-ABCD", "backup"), Some(local("/restored"))),
        (local("/unrelated"), Some(local("/elsewhere"))),
    ]));
    assert_eq!(views.view(&local("/restored/2024")), second);
    assert!(!views.relocate(&[(local("/missing"), Some(local("/elsewhere")))]));
}

#[test]
fn forgetting_drops_the_folder_and_its_descendants_only() {
    let mut views = FolderViews::default();
    let view = sorted(SortKey::Size, SortDirection::Ascending);
    for path in ["/work", "/work/notes", "/workshop"] {
        set(&mut views, &local(path), view);
    }
    set(&mut views, &volume("1234-ABCD", "work"), view);

    assert!(views.relocate(&[(local("/work"), None)]));

    assert_eq!(views.len(), 2);
    assert_eq!(views.view(&local("/workshop")), view);
    assert_eq!(views.view(&volume("1234-ABCD", "work")), view);
    assert!(!views.relocate(&[(local("/work"), None)]));
}

#[test]
fn only_a_folder_missing_beside_other_entries_and_no_mounts_counts_as_deleted() {
    let root = tempfile::tempdir().expect("root folder");
    let parent = root.path().join("projects");
    std::fs::create_dir_all(parent.join("kept")).expect("sibling folder");
    let mount_point = root.path().join("data");
    std::fs::create_dir(&mount_point).expect("empty mount point");
    let key = |path: &Path| FolderKey::local(path).expect("valid local key");
    let no_mounts = |_: &Path| false;
    let deleted = parent.join("gone");

    assert!(folder_was_deleted(&deleted, &key(&deleted), no_mounts));
    let kept = parent.join("kept");
    assert!(!folder_was_deleted(&kept, &key(&kept), no_mounts));
    for unmounted in [mount_point.join("photos"), mount_point.join("photos/2024")] {
        assert!(
            !folder_was_deleted(&unmounted, &key(&unmounted), no_mounts),
            "{} is on a drive that is not mounted",
            unmounted.display()
        );
    }
    assert!(
        !folder_was_deleted(&deleted, &key(&deleted), |directory| directory == parent),
        "a directory of mount points loses a drive's folder when it is unmounted"
    );
    assert!(!folder_was_deleted(
        &deleted,
        &volume("1234-ABCD", ""),
        no_mounts
    ));
}

#[test]
fn removable_paths_key_by_the_innermost_volume_and_unstorable_paths_have_no_key() {
    let roots = [
        RemovableRoot {
            uuid: "OUTER".into(),
            path: PathBuf::from("/run/media/me/DRIVE"),
        },
        RemovableRoot {
            uuid: "INNER".into(),
            path: PathBuf::from("/run/media/me/DRIVE/card"),
        },
    ];

    assert_eq!(
        key_for_path(Path::new("/run/media/me/DRIVE/DCIM/"), &roots),
        Some(volume("OUTER", "DCIM"))
    );
    assert_eq!(
        key_for_path(Path::new("/run/media/me/DRIVE"), &roots),
        Some(volume("OUTER", ""))
    );
    assert_eq!(
        key_for_path(Path::new("/run/media/me/DRIVE/card/photos"), &roots),
        Some(volume("INNER", "photos"))
    );
    assert_eq!(
        key_for_path(Path::new("/run/media/me/DRIVE-2/photos"), &roots),
        Some(local("/run/media/me/DRIVE-2/photos"))
    );
    assert_eq!(
        key_for_path(Path::new("/home/me/./Music"), &roots),
        Some(local("/home/me/Music"))
    );
    for path in [
        Path::new("relative"),
        Path::new("/home/me/../other"),
        Path::new(OsStr::from_bytes(b"/home/me/\xff")),
    ] {
        assert_eq!(key_for_path(path, &roots), None, "{path:?}");
    }
}

#[test]
fn saving_merges_unsaved_changes_over_what_another_process_wrote() {
    let (mut ours, _) = FolderViews::parse(
        "[[folder]]\npath = \"/ours\"\nicons_size = 64\nused = 1\n\
         [[folder]]\npath = \"/touched\"\nicons_size = 64\nused = 2\n\
         [[folder]]\npath = \"/removed\"\nicons_size = 64\nused = 3\n",
    )
    .expect("loaded");
    let mine = sorted(SortKey::Name, SortDirection::Descending);
    set(&mut ours, &local("/ours"), mine);
    set(&mut ours, &local("/new"), mine);
    assert!(ours.touch(&local("/touched")));
    ours.update(&local("/removed"), |view| view.icons_size = None);
    let (theirs, _) = FolderViews::parse(
        "[[folder]]\npath = \"/ours\"\nicons_size = 160\nused = 1\n\
         [[folder]]\npath = \"/touched\"\nicons_size = 160\nused = 2\n\
         [[folder]]\npath = \"/removed\"\nicons_size = 64\nused = 3\n\
         [[folder]]\npath = \"/theirs\"\nicons_size = 160\nused = 4\n",
    )
    .expect("written elsewhere");

    let mut merged = ours.merged_over(theirs);

    assert_eq!(
        merged.view(&local("/ours")),
        mine,
        "a folder changed here keeps this process's values"
    );
    assert_eq!(merged.view(&local("/new")), mine);
    assert_eq!(
        merged.view(&local("/touched")).icons_size,
        Some(160),
        "opening a folder does not revert another process's change"
    );
    assert_eq!(merged.view(&local("/removed")), FolderView::default());
    assert_eq!(merged.view(&local("/theirs")).icons_size, Some(160));

    let (later, _) = FolderViews::parse("[[folder]]\npath = \"/later\"\nicons_size = 96\n")
        .expect("written again before this save succeeded");
    let unsaved = merged.merged_over(later);
    assert_eq!(unsaved.view(&local("/new")), mine, "still unsaved");
    assert_eq!(unsaved.view(&local("/theirs")), FolderView::default());
    merged.mark_saved();
    let (after_save, _) =
        FolderViews::parse("[[folder]]\npath = \"/later\"\nicons_size = 96\n").expect("parsed");
    assert!(
        merged.merged_over(after_save).same_views(
            &FolderViews::parse("[[folder]]\npath = \"/later\"\nicons_size = 96\n")
                .expect("parsed")
                .0
        )
    );

    let (mut forgetting, _) =
        FolderViews::parse("[[folder]]\npath = \"/theirs\"\nicons_size = 160\n").expect("parsed");
    assert!(forgetting.clear());
    set(&mut forgetting, &local("/kept"), mine);
    let (theirs, _) =
        FolderViews::parse("[[folder]]\npath = \"/theirs\"\nicons_size = 160\n").expect("parsed");
    let merged = forgetting.merged_over(theirs);
    assert_eq!(merged.view(&local("/theirs")), FolderView::default());
    assert_eq!(merged.view(&local("/kept")), mine);
}
