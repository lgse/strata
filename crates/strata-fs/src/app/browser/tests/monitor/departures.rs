// SPDX-License-Identifier: MIT

use super::*;

#[derive(Debug, Clone, Copy)]
enum OutsideChange {
    RenamedInPlace,
    /// Inotify reports no rename of an ancestor, so the next change reports the removal.
    ParentRenamedInPlace,
    GrandparentRenamedInPlace,
    /// The reload a rescan or F5 starts fails, which shows the folder is gone.
    ParentRenamedInAMonitorBurst,
    MovedIntoAnotherFolder,
    Deleted,
    DeletedWithItsParent,
    /// An execute-only parent cannot be listed to find the new name.
    RenamedUnderAnUnlistableParent,
    /// A new folder elsewhere in the parent got the deleted folder's inode.
    InodeReusedByAnotherFolder,
    /// Too many changes at once: the monitor asks for a rescan instead.
    DeletedInAMonitorBurst,
    Recreated,
    /// An unreachable folder may come back; leaving would lose the user's place.
    Unreachable,
}

struct OpenFolder {
    parent: Location,
    open: Location,
    renamed: Location,
}

impl OutsideChange {
    /// Makes the change on `source` and returns where the browser should end up.
    fn apply(self, source: &DepartingSource, folder: &OpenFolder) -> Location {
        let OpenFolder {
            parent,
            open,
            renamed,
        } = folder;
        match self {
            Self::RenamedInPlace => {
                source.rename(open, renamed);
                renamed.clone()
            }
            Self::ParentRenamedInPlace | Self::ParentRenamedInAMonitorBurst => {
                source.rename_tree(parent, &Location::local("/fixture/a2"));
                Location::local("/fixture/a2/b")
            }
            Self::GrandparentRenamedInPlace => {
                source.rename_tree(&Location::local("/fixture"), &Location::local("/moved"));
                Location::local("/moved/a/b")
            }
            Self::MovedIntoAnotherFolder => {
                source.rename(open, &Location::local("/fixture/elsewhere/b"));
                parent.clone()
            }
            Self::Deleted | Self::DeletedInAMonitorBurst => {
                source.missing.borrow_mut().insert(open.clone());
                parent.clone()
            }
            Self::DeletedWithItsParent => {
                source
                    .missing
                    .borrow_mut()
                    .extend([open.clone(), parent.clone()]);
                Location::local("/fixture")
            }
            Self::RenamedUnderAnUnlistableParent => {
                source.rename(open, renamed);
                source.unlistable.borrow_mut().insert(parent.clone());
                parent.clone()
            }
            Self::InodeReusedByAnotherFolder => {
                source
                    .identities
                    .reuse_inode(open, &Location::local("/fixture/a/out"));
                source.missing.borrow_mut().insert(open.clone());
                parent.clone()
            }
            Self::Recreated => {
                source.identities.identify(open);
                open.clone()
            }
            Self::Unreachable => {
                source.unreachable.borrow_mut().insert(open.clone());
                open.clone()
            }
        }
    }

    fn report(self, open: &Location) -> DirectoryChange {
        match self {
            Self::DeletedInAMonitorBurst | Self::ParentRenamedInAMonitorBurst => {
                DirectoryChange::Rescan
            }
            _ => DirectoryChange::Remove(open.clone()),
        }
    }

    /// `ColumnsRelocated`, `Reset` and `ColumnReloaded` counts.
    fn expected_navigation(self) -> (usize, usize, usize) {
        match self {
            Self::RenamedInPlace | Self::ParentRenamedInPlace | Self::GrandparentRenamedInPlace => {
                (1, 0, 0)
            }
            Self::ParentRenamedInAMonitorBurst => (1, 0, 1),
            Self::Unreachable => (0, 0, 0),
            // The rescan's reload fails, which shows the folder is gone.
            Self::DeletedInAMonitorBurst => (0, 1, 1),
            _ => (0, 1, 0),
        }
    }
}

fn navigation_after_removal(events: &RefCell<Vec<BrowserEvent>>) -> (usize, usize, usize) {
    (
        count_events(events, |event| {
            matches!(event, BrowserEvent::ColumnsRelocated { from_depth: 0 })
        }),
        count_events(events, |event| matches!(event, BrowserEvent::Reset)),
        count_events(events, |event| {
            matches!(event, BrowserEvent::ColumnReloaded { .. })
        }),
    )
}

fn reacts_to_its_own_removal(change: OutsideChange) -> Option<String> {
    let folder = OpenFolder {
        parent: Location::local("/fixture/a"),
        open: Location::local("/fixture/a/b"),
        renamed: Location::local("/fixture/a/renamed"),
    };
    let open = &folder.open;
    let source = Rc::new(DepartingSource::default());
    source.identities.identify(open);
    let browser = Browser::new(source.clone());
    browser.navigate(open.clone());
    let could_go_back = browser.can_go_back();
    let monitors_before = source.watches.borrow().len();
    let events = recorded_events(&browser);

    let expected = change.apply(&source, &folder);
    source.directory_monitor(open)(change.report(open));

    let (relocations, resets, reloads) = navigation_after_removal(&events);
    let rearmed = source.watches.borrow().len() > monitors_before;
    let got = (
        open_path(&browser),
        (relocations, resets, reloads),
        rearmed,
        browser.can_go_back(),
    );
    let wanted = (
        vec![expected.clone()],
        change.expected_navigation(),
        !matches!(change, OutsideChange::Unreachable),
        could_go_back,
    );
    (got != wanted).then(|| {
        format!(
            "{change:?}: expected [{expected:?}], got {:?}; ColumnsRelocated x{relocations} \
             Reset x{resets} ColumnReloaded x{reloads} rearmed={rearmed} can_go_back \
             {could_go_back}->{}",
            got.0, got.3,
        )
    })
}

#[test]
fn the_open_folder_reacts_to_its_own_removal_report() {
    let changes = [
        OutsideChange::RenamedInPlace,
        OutsideChange::ParentRenamedInPlace,
        OutsideChange::GrandparentRenamedInPlace,
        OutsideChange::ParentRenamedInAMonitorBurst,
        OutsideChange::MovedIntoAnotherFolder,
        OutsideChange::Deleted,
        OutsideChange::DeletedWithItsParent,
        OutsideChange::RenamedUnderAnUnlistableParent,
        OutsideChange::InodeReusedByAnotherFolder,
        OutsideChange::DeletedInAMonitorBurst,
        OutsideChange::Recreated,
        OutsideChange::Unreachable,
    ];
    assert_rows(changes, reacts_to_its_own_removal);
}

#[test]
fn a_root_removed_with_an_open_child_column_leaves_in_either_report_order() {
    let root = Location::local("/fixture/a");
    let open_child = Location::local("/fixture/a/b");
    assert_rows([true, false], |root_reported_first| {
        let source = Rc::new(DepartingSource {
            defer_finds: true,
            ..DepartingSource::default()
        });
        source.identities.identify(&root);
        let browser = Browser::new(source.clone());
        browser.navigate(root.clone());
        browser.show_child(0, open_child.clone());
        source
            .missing
            .borrow_mut()
            .extend([root.clone(), open_child.clone()]);

        // One monitor batch, drained in either order; the parent lookup answers later.
        let reports = match root_reported_first {
            true => [root.clone(), open_child.clone()],
            false => [open_child.clone(), root.clone()],
        };
        let monitor = source.directory_monitor(&root);
        for removed in reports {
            monitor(DirectoryChange::Remove(removed));
        }
        source.answer_finds();

        (open_path(&browser) != [Location::local("/fixture")]).then(|| {
            format!(
                "root reported first: {root_reported_first}, path {:?}",
                open_path(&browser)
            )
        })
    });
}

#[test]
fn a_child_column_own_removal_report_is_left_to_its_parent_column() {
    let parent = Location::local("/fixture/a");
    let open = Location::local("/fixture/a/b");
    let source = Rc::new(DepartingSource::default());
    let browser = Browser::new(source.clone());
    browser.navigate(parent.clone());
    browser.show_child(0, open.clone());
    let events = recorded_events(&browser);
    source.missing.borrow_mut().insert(open.clone());

    source.directory_monitor(&open)(DirectoryChange::Remove(open.clone()));

    assert_eq!(open_path(&browser), [parent, open]);
    assert!(events.borrow().is_empty(), "{:?}", events.borrow());
}

#[test]
fn a_child_column_removal_report_follows_a_renamed_ancestor_of_the_first_column() {
    let root = Location::local("/fixture/a/b");
    let child = Location::local("/fixture/a/b/c");
    let source = Rc::new(DepartingSource::default());
    source.identities.identify(&root);
    let browser = Browser::new(source.clone());
    browser.navigate(root.clone());
    browser.show_child(0, child.clone());
    source.rename_tree(
        &Location::local("/fixture/a"),
        &Location::local("/fixture/a2"),
    );

    source.directory_monitor(&child)(DirectoryChange::Remove(child.clone()));

    assert_eq!(
        open_path(&browser),
        [
            Location::local("/fixture/a2/b"),
            Location::local("/fixture/a2/b/c")
        ]
    );
}

#[derive(Debug, Clone, Copy)]
enum BoundaryDeparture {
    /// The boundary folder itself is renamed, so its new name lies outside.
    RenamedOutside,
    /// Every folder left inside the boundary is gone too.
    DeletedWithTheBoundary,
}

impl BoundaryDeparture {
    fn open(self, boundary: &Location) -> Location {
        match self {
            Self::RenamedOutside => boundary.clone(),
            Self::DeletedWithTheBoundary => Location::local("/fixture/device/open"),
        }
    }

    fn apply(self, source: &DepartingSource, open: &Location, boundary: &Location) {
        match self {
            Self::RenamedOutside => {
                source.rename(open, &Location::local("/fixture/device-renamed"))
            }
            Self::DeletedWithTheBoundary => source
                .missing
                .borrow_mut()
                .extend([open.clone(), boundary.clone()]),
        }
    }
}

fn stays_inside_the_boundary(departure: BoundaryDeparture) -> Option<String> {
    let boundary = Location::local("/fixture/device");
    let open = departure.open(&boundary);
    let source = Rc::new(DepartingSource {
        boundary: Some(boundary.clone()),
        ..DepartingSource::default()
    });
    source.identities.identify(&open);
    let browser = Browser::new(source.clone());
    browser.navigate(open.clone());
    let events = recorded_events(&browser);
    departure.apply(&source, &open, &boundary);

    source.directory_monitor(&open)(DirectoryChange::Remove(open.clone()));

    // With nothing allowed to show instead, the folder shows its read error.
    let reloads = count_events(&events, |event| {
        matches!(event, BrowserEvent::ColumnReloaded { depth: 0 })
    });
    let navigated = count_events(&events, |event| {
        matches!(
            event,
            BrowserEvent::Reset | BrowserEvent::ColumnsRelocated { .. }
        )
    });
    let path = open_path(&browser);
    ((&path, reloads, navigated) != (&vec![open], 1, 0)).then(|| {
        format!("{departure:?}: path {path:?}, ColumnReloaded x{reloads}, navigated x{navigated}")
    })
}

#[test]
fn a_departure_never_leaves_the_navigation_boundary() {
    assert_rows(
        [
            BoundaryDeparture::RenamedOutside,
            BoundaryDeparture::DeletedWithTheBoundary,
        ],
        stays_inside_the_boundary,
    );
}
