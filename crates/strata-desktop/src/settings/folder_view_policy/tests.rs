// SPDX-License-Identifier: MIT

use crate::model::{Location, SortDirection, SortKey};

use super::{
    IconsSizeKept, Prune, icons_size_for, keep_icons_size, remembered_key, set_default_icons_size,
    set_remember_folder_views,
};
use crate::settings::{
    Preferences,
    folder_views::{FolderViews, key_for_location},
    sort_preferences,
};

fn settings(remember: bool, default_size: i32) -> Preferences {
    Preferences {
        remember_folder_views: remember,
        icons_thumbnail_size: default_size,
        ..Preferences::default()
    }
}

fn local() -> Location {
    Location::local("/home/someone/pictures")
}

fn remote() -> Location {
    Location::uri("sftp://host/pictures")
}

/// A folder is remembered only while folders' views are, and only where it has a key; a
/// folder's own size is read only then.
#[test]
fn a_folders_size_is_its_own_only_while_it_is_remembered() {
    for (remember, location, own, expected) in [
        (true, Some(local()), Some(128), 128),
        (true, Some(local()), None, 96),
        (false, Some(local()), Some(128), 96),
        (true, Some(remote()), Some(128), 96),
        (true, None, Some(128), 96),
    ] {
        let settings = settings(remember, 96);
        let mut read = false;
        let size = icons_size_for(&settings, location.as_ref(), |_| {
            read = true;
            own
        });
        let remembered = location
            .as_ref()
            .and_then(|location| remembered_key(&settings, location));
        assert_eq!(size, expected, "{remember} {location:?}");
        assert_eq!(
            read,
            remembered.is_some(),
            "the views read for {location:?}"
        );
    }
}

/// A changed size is the default while folders' views are off, the folder's own while they
/// are on (none where it equals the default), and only shown for a folder with no key; each
/// held between 32 and 256.
#[test]
fn a_changed_size_is_kept_where_gtk_keeps_it() {
    let key = key_for_location(&local()).expect("a key");
    for (remember, location, size, expected) in [
        (false, Some(local()), 128, IconsSizeKept::Default(128)),
        (false, Some(local()), 999, IconsSizeKept::Default(256)),
        (
            true,
            Some(local()),
            128,
            IconsSizeKept::Folder(key.clone(), Some(128)),
        ),
        (
            true,
            Some(local()),
            96,
            IconsSizeKept::Folder(key.clone(), None),
        ),
        (
            true,
            Some(local()),
            8,
            IconsSizeKept::Folder(key.clone(), Some(32)),
        ),
        (true, Some(remote()), 128, IconsSizeKept::Shown),
        (true, None, 128, IconsSizeKept::Shown),
    ] {
        let kept = keep_icons_size(&settings(remember, 96), location.as_ref(), size);
        assert_eq!(kept, expected, "{remember} {location:?} {size}");
    }
}

/// A new default is set, held in range, and prunes the folders' sizes only while folders'
/// views are remembered; remembering them again prunes the sorts and sizes that equal the
/// defaults.
#[test]
fn a_new_default_prunes_while_folders_are_remembered() {
    for (remember, expected) in [(true, Some(Prune::IconsSize(256))), (false, None)] {
        let mut settings = settings(remember, 96);
        assert_eq!(set_default_icons_size(&mut settings, 300), expected);
        assert_eq!(settings.icons_thumbnail_size, 256);
    }

    let mut on = settings(false, 112);
    let defaults = Prune::Defaults {
        sort: sort_preferences(&on).sort_options(),
        icons_size: 112,
    };
    assert_eq!(set_remember_folder_views(&mut on, true), Some(defaults));
    assert!(on.remember_folder_views);
    assert_eq!(set_remember_folder_views(&mut on, false), None);
    assert!(!on.remember_folder_views);
}

/// Pruning drops the values that equal the default and keeps the rest, and only a return to
/// remembering reaches the sorts.
#[test]
fn pruning_drops_what_equals_the_default() {
    let key = key_for_location(&local()).expect("a key");
    let other = key_for_location(&Location::local("/home/someone/music")).expect("a key");
    let sort = (SortKey::Size, SortDirection::Descending, Default::default());
    let mut views = FolderViews::default();
    views.update(&key, |view| view.icons_size = Some(128));
    views.update(&other, |view| {
        view.icons_size = Some(160);
        view.sort = Some(sort);
    });

    assert!(Prune::IconsSize(128).apply(&mut views));
    assert_eq!(views.view(&key).icons_size, None);
    assert_eq!(views.view(&other).icons_size, Some(160));
    assert!(
        !Prune::IconsSize(128).apply(&mut views),
        "nothing left to drop"
    );

    let defaults = Prune::Defaults {
        sort,
        icons_size: 160,
    };
    assert!(defaults.apply(&mut views));
    assert_eq!(views.view(&other).sort, None);
    assert_eq!(views.view(&other).icons_size, None);
    assert!(defaults.reaches_sorts());
    assert!(!Prune::IconsSize(1).reaches_sorts());
}
