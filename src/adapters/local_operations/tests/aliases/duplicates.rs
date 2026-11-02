// SPDX-License-Identifier: MIT

//! Same-named items pasted into a folder that holds the original of one of them.

use super::super::super::transfer_targets::{Collision, TransferTarget, transfer_targets};
use super::*;

const MEETS: Option<Collision> = Some(Collision {
    mergeable: false,
    replaceable: true,
});

fn targets_in(
    folder: &Path,
    sources: &[Location],
    moving: bool,
) -> Result<Vec<TransferTarget>, Box<dyn Error>> {
    Ok(glib::MainContext::default().block_on(transfer_targets(
        &Location::local(folder),
        sources,
        moving,
        &gio::Cancellable::new(),
    ))?)
}

/// An item from two/ pasted into one/, which holds `original`.
struct Meeting {
    quirks: Quirks,
    original: &'static str,
    two_first: bool,
    moving: bool,
    /// What the prompt shows for one/'s and two/'s items.
    shown: [(&'static str, Option<Collision>); 2],
    /// one/ once two/'s item is kept beside it.
    listed: Vec<(&'static str, &'static str)>,
}

fn meetings() -> [Meeting; 4] {
    let notes_numbered_twice = vec![
        ("notes (1).txt", "one"),
        ("notes (2).txt", "two"),
        ("notes.txt", "one"),
    ];
    [
        Meeting {
            quirks: Quirks::ext4(),
            original: "notes.txt",
            two_first: false,
            moving: false,
            shown: [("notes.txt", None), ("notes.txt", MEETS)],
            listed: notes_numbered_twice.clone(),
        },
        Meeting {
            quirks: Quirks::ext4(),
            original: "notes.txt",
            two_first: true,
            moving: false,
            shown: [("notes.txt", None), ("notes.txt", MEETS)],
            listed: notes_numbered_twice,
        },
        Meeting {
            quirks: Quirks::vfat(),
            original: "NOTES.TXT",
            two_first: false,
            moving: false,
            shown: [("NOTES.TXT", None), ("notes.txt", MEETS)],
            listed: vec![
                ("NOTES (1).TXT", "one"),
                ("NOTES.TXT", "one"),
                ("notes (2).txt", "two"),
            ],
        },
        // A moved item stays where it is and keeps its name.
        Meeting {
            quirks: Quirks::ext4(),
            original: "notes.txt",
            two_first: false,
            moving: true,
            shown: [("notes.txt", None), ("notes (1).txt", None)],
            listed: vec![("notes (1).txt", "two"), ("notes.txt", "one")],
        },
    ]
}

/// Pastes each item under the name its target shows, keeping both where it collides.
fn paste_targets(
    folder: &Path,
    sources: Vec<Location>,
    targets: Vec<TransferTarget>,
    moving: bool,
) -> Vec<OperationEvent> {
    run_paste(PasteRequest {
        id: OperationRequestId(1530),
        destination: Location::local(folder),
        items: sources
            .into_iter()
            .zip(targets)
            .map(|(source, target)| PasteItem {
                source,
                conflict: if target.collision.is_some() {
                    TransferConflict::KeepBoth
                } else {
                    TransferConflict::FailIfExists
                },
                target_name: target.name,
            })
            .collect(),
        move_sources: moving,
    })
}

fn shown_targets(targets: &[TransferTarget]) -> Vec<(Option<OsString>, Option<Collision>)> {
    targets
        .iter()
        .map(|target| {
            let name = target.name.as_ref().map(|shown| shown.name.clone());
            (name, target.collision)
        })
        .collect()
}

fn check_meeting(meeting: Meeting) -> Result<(), Box<dyn Error>> {
    let Meeting {
        quirks,
        original,
        two_first,
        moving,
        shown,
        listed,
    } = meeting;
    let context = format!("{original}, two first: {two_first}, moving: {moving}");
    let root = tempfile::tempdir()?;
    let (one, two) = (root.path().join("one"), root.path().join("two"));
    fs::create_dir(&one)?;
    fs::create_dir(&two)?;
    fs::write(one.join(original), "one")?;
    fs::write(two.join("notes.txt"), "two")?;
    let _scope = FsQuirks::install(root.path(), quirks);
    let mut sources = vec![
        Location::local(one.join(original)),
        Location::local(two.join("notes.txt")),
    ];
    let mut expected = shown
        .map(|(name, collision)| (Some(OsString::from(name)), collision))
        .to_vec();
    if two_first {
        sources.reverse();
        expected.reverse();
    }

    let targets = targets_in(&one, &sources, moving)?;
    assert_eq!(shown_targets(&targets), expected, "{context}");
    let events = paste_targets(&one, sources, targets, moving);

    assert_eq!(held_contents(&one)?, owned_pairs(&listed), "{context}");
    let numbered = moving.then(|| {
        (
            "notes.txt".to_owned(),
            "notes (1).txt".to_owned(),
            RenameCause::Taken,
        )
    });
    assert_eq!(
        renamed_beneath(&one, &followed_report(&events)),
        Vec::from_iter(numbered),
        "{context}"
    );
    assert_eq!(names(&two)?.is_empty(), moving, "{context}");
    Ok(())
}

#[test]
fn an_item_from_another_folder_meets_the_original_of_a_duplicate() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for meeting in meetings() {
        check_meeting(meeting)?;
    }
    Ok(())
}

/// The notes.txt of three/, one/ and two/ numbered past one/'s own.
const THREE_ONE_TWO: &[(&str, &str)] = &[
    ("notes (1).txt", "three"),
    ("notes (2).txt", "one"),
    ("notes (3).txt", "two"),
    ("notes.txt", "one"),
];

const THREE_TWO: &[(&str, &str)] = &[
    ("notes (1).txt", "three"),
    ("notes (2).txt", "two"),
    ("notes.txt", "one"),
];

/// A paste of the notes.txt of the `selected` folders into one/.
struct ThreeFolders {
    selected: &'static [&'static str],
    /// The choice for the item that meets one/notes.txt; `None` skips it.
    choice: Option<TransferConflict>,
    /// Whether the request lists the items as selected instead of as the browser does, the
    /// ones asked about last.
    as_selected: bool,
    /// one/ after the paste.
    listed: &'static [(&'static str, &'static str)],
}

const fn three(
    selected: &'static [&'static str],
    choice: Option<TransferConflict>,
    as_selected: bool,
    listed: &'static [(&'static str, &'static str)],
) -> ThreeFolders {
    ThreeFolders {
        selected,
        choice,
        as_selected,
        listed,
    }
}

const KEEP_BOTH: Option<TransferConflict> = Some(TransferConflict::KeepBoth);

const THREE_FOLDER_ROWS: &[ThreeFolders] = &[
    three(&["two", "three"], KEEP_BOTH, false, THREE_TWO),
    three(
        &["two", "three"],
        None,
        false,
        &[("notes (1).txt", "three"), ("notes.txt", "one")],
    ),
    three(&["one", "two", "three"], KEEP_BOTH, false, THREE_ONE_TWO),
    three(
        &["one", "two", "three"],
        None,
        false,
        &[
            ("notes (1).txt", "three"),
            ("notes (2).txt", "one"),
            ("notes.txt", "one"),
        ],
    ),
    three(&["two", "three", "one"], KEEP_BOTH, false, THREE_ONE_TWO),
    three(
        &["one", "two", "three"],
        Some(TransferConflict::ReplaceExisting),
        false,
        &[
            ("notes (1).txt", "three"),
            ("notes (2).txt", "one"),
            ("notes.txt", "two"),
        ],
    ),
    three(&["two", "one", "three"], KEEP_BOTH, false, THREE_ONE_TWO),
    three(
        &["one", "three", "two"],
        KEEP_BOTH,
        false,
        &[
            ("notes (1).txt", "two"),
            ("notes (2).txt", "one"),
            ("notes (3).txt", "three"),
            ("notes.txt", "one"),
        ],
    ),
    three(&["two", "three"], KEEP_BOTH, true, THREE_TWO),
    three(&["one", "two", "three"], KEEP_BOTH, true, THREE_ONE_TWO),
    three(&["two", "three", "one"], KEEP_BOTH, true, THREE_ONE_TWO),
];

/// The items for `targets`, taking `choice` where one collides and leaving it out for `None`.
fn chosen_items(
    sources: Vec<Location>,
    targets: Vec<TransferTarget>,
    choice: Option<TransferConflict>,
) -> Vec<PasteItem> {
    sources
        .into_iter()
        .zip(targets)
        .filter_map(|(source, target)| {
            let conflict = match target.collision {
                Some(_) => choice?,
                None => TransferConflict::FailIfExists,
            };
            Some(PasteItem {
                source,
                conflict,
                target_name: target.name,
            })
        })
        .collect()
}

fn check_three_folders(row: &ThreeFolders) -> Result<(), Box<dyn Error>> {
    let context = format!(
        "{:?}, {:?}, as selected: {}",
        row.selected, row.choice, row.as_selected
    );
    let root = tempfile::tempdir()?;
    for folder in ["one", "two", "three"] {
        fs::create_dir(root.path().join(folder))?;
        fs::write(root.path().join(folder).join("notes.txt"), folder)?;
    }
    let one = root.path().join("one");
    let sources = row
        .selected
        .iter()
        .map(|folder| Location::local(root.path().join(folder).join("notes.txt")))
        .collect::<Vec<_>>();
    let targets = targets_in(&one, &sources, false)?;
    let mut items = chosen_items(sources, targets, row.choice);
    if !row.as_selected {
        items.sort_by_key(|item| item.conflict != TransferConflict::FailIfExists);
    }

    let events = paste_until_settled(PasteRequest {
        id: OperationRequestId(1530),
        destination: Location::local(&one),
        items,
        move_sources: false,
    });

    assert_eq!(held_contents(&one)?, owned_pairs(row.listed), "{context}");
    assert_eq!(
        renamed_beneath(&one, &followed_report(&events)),
        [(
            "notes.txt".to_owned(),
            "notes (1).txt".to_owned(),
            RenameCause::Taken
        )],
        "{context}"
    );
    Ok(())
}

#[test]
fn same_named_items_from_three_folders_all_land_whatever_their_order_and_choice()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for row in THREE_FOLDER_ROWS {
        check_three_folders(row)?;
    }
    Ok(())
}
