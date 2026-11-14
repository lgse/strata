// SPDX-License-Identifier: MIT

use crate::model::{FolderSort, Location, NameComparison, SortDirection, SortKey, SortOptions};

use super::{
    DefaultSort, SortKept, folder_sort, forget_folder_sort, keep_folder_sort,
    set_default_sort_options,
};
use crate::settings::{
    Preferences,
    folder_views::{FolderViews, key_for_location},
    sort_preferences,
};

fn settings(remember: bool) -> Preferences {
    Preferences {
        remember_folder_views: remember,
        ..Preferences::default()
    }
}

fn local() -> Location {
    Location::local("/home/someone/pictures")
}

fn remote() -> Location {
    Location::uri("sftp://host/pictures")
}

fn sort(key: SortKey, direction: SortDirection) -> SortOptions {
    (key, direction, NameComparison::default())
}

/// A remembered folder resolves to its own sort, else to the default; opening it counts as
/// using it, which wants writing only when the folder has values to keep.
#[test]
fn a_folder_resolves_to_its_own_sort_and_opening_it_counts_as_use() {
    let key = key_for_location(&local()).expect("a key");
    let size_down = sort(SortKey::Size, SortDirection::Descending);
    let mut views = FolderViews::default();

    assert_eq!(
        folder_sort(&mut views, &key, true),
        (FolderSort::Default, false)
    );

    views.update(&key, |view| view.sort = Some(size_down));
    let saved = FolderSort::Saved(SortKey::Size, SortDirection::Descending, Default::default());
    assert_eq!(folder_sort(&mut views, &key, false), (saved, false));
    let edits = views.edits();
    assert_eq!(folder_sort(&mut views, &key, true), (saved, true));
    assert!(views.edits() > edits, "the use is an edit to write");
}

/// A chosen sort is kept only for a remembered folder and only when it can be stored, and as
/// none when it equals the default, so the folder follows later defaults.
#[test]
fn a_chosen_sort_is_kept_where_gtk_keeps_it() {
    let key = key_for_location(&local()).expect("a key");
    let default = sort_preferences(&settings(true)).sort_options();
    let size_down = sort(SortKey::Size, SortDirection::Descending);
    for (remember, location, chosen, expected) in [
        (
            true,
            local(),
            size_down,
            Some(SortKept {
                key: key.clone(),
                sort: Some(size_down),
            }),
        ),
        (
            true,
            local(),
            default,
            Some(SortKept {
                key: key.clone(),
                sort: None,
            }),
        ),
        (
            true,
            local(),
            sort(SortKey::DeviceOrder, SortDirection::Ascending),
            None,
        ),
        (
            true,
            local(),
            sort(SortKey::Recency, SortDirection::Descending),
            None,
        ),
        (false, local(), size_down, None),
        (true, remote(), size_down, None),
    ] {
        let kept = keep_folder_sort(&settings(remember), &location, chosen);
        assert_eq!(kept, expected, "{remember} {location:?} {chosen:?}");
    }
}

/// Resetting a remembered folder drops its own sort; a folder that is not remembered has none
/// to drop.
#[test]
fn a_reset_drops_a_remembered_folders_own_sort() {
    let key = key_for_location(&local()).expect("a key");
    let mut views = FolderViews::default();
    views.update(&key, |view| {
        view.sort = Some(sort(SortKey::Size, SortDirection::Descending));
    });

    let forgotten = forget_folder_sort(&settings(true), &local()).expect("a remembered folder");
    assert!(forgotten.apply(&mut views));
    assert_eq!(views.view(&key).sort, None);
    assert_eq!(forget_folder_sort(&settings(false), &local()), None);
    assert_eq!(forget_folder_sort(&settings(true), &remote()), None);
}

/// A default that cannot be stored changes nothing; one that can is stored, and while folders'
/// views are remembered it prunes the folders' sorts that now equal it.
#[test]
fn a_new_default_sort_prunes_while_folders_are_remembered() {
    let size_down = sort(SortKey::Size, SortDirection::Descending);

    let mut unchanged = settings(true);
    let before = unchanged.clone();
    assert_eq!(
        set_default_sort_options(
            &mut unchanged,
            sort(SortKey::DeviceOrder, SortDirection::Ascending)
        ),
        DefaultSort::Unstorable
    );
    assert_eq!(unchanged, before);

    let mut off = settings(false);
    assert_eq!(
        set_default_sort_options(&mut off, size_down),
        DefaultSort::Stored(None)
    );
    assert_eq!(sort_preferences(&off).sort_options(), size_down);

    let key = key_for_location(&local()).expect("a key");
    let other = key_for_location(&Location::local("/home/someone/music")).expect("a key");
    let mut views = FolderViews::default();
    views.update(&key, |view| view.sort = Some(size_down));
    views.update(&other, |view| {
        view.sort = Some(sort(SortKey::Type, SortDirection::Ascending));
    });
    let mut on = settings(true);
    let DefaultSort::Stored(Some(prune)) = set_default_sort_options(&mut on, size_down) else {
        panic!("a stored default prunes while folders are remembered");
    };
    assert!(prune.apply(&mut views));
    assert_eq!(views.view(&key).sort, None);
    assert!(views.view(&other).sort.is_some());
}
