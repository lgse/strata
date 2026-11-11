// SPDX-License-Identifier: MIT

//! Names a destination lists under another spelling, or holds before the paste.

use super::super::fs_quirks::{Op, Racer, renumbered};
use super::*;

/// Lowers `Makefile` in the destination's copy stage to `makefile`, renumbering it when asked.
fn lowers_the_name(destination: &Path, renumber: bool) -> Racer {
    let live = destination.to_path_buf();
    Box::new(move || {
        let stage = names(&live)
            .expect("destination")
            .into_iter()
            .find(|name| name.starts_with(".strata-copy-"))
            .expect("stage");
        let lowered = live.join(stage).join("makefile");
        fs::rename(lowered.with_file_name("Makefile"), &lowered).expect("lower the name");
        if renumber {
            renumbered(&lowered);
        }
    })
}

fn reported_renames(events: &[OperationEvent]) -> Vec<RenamedItem> {
    events
        .iter()
        .filter_map(|event| match event {
            OperationEvent::ItemsReported { report, .. } => Some(report.renamed.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}

#[test]
fn two_names_are_never_taken_for_one_where_the_listing_changes_their_case()
-> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    // ciopfs lists `Makefile` as `makefile` once it is made, as Samba with
    // `preserve case = no` does; a racer renames it so. Where inode numbers
    // are not kept, the entry then reports another number. FAT and SMB plan
    // such names apart before.
    let rows = [
        ("copy", false, false),
        ("move", true, false),
        ("unstable inodes", false, true),
    ];
    for (row, moving, renumber) in rows {
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let folder = sources.join("folder");
        let destination = root.path().join("destination");
        fs::create_dir_all(&folder)?;
        fs::create_dir(&destination)?;
        fs::write(folder.join("Makefile"), b"upper")?;
        fs::write(folder.join("makefile"), b"lower")?;
        let _sources = FsQuirks::install(&sources, Quirks::default());
        let _destination = FsQuirks::install(
            &destination,
            Quirks {
                before: vec![(
                    Op::CreateNode,
                    "makefile",
                    0,
                    lowers_the_name(&destination, renumber),
                )],
                ..ciopfs()
            },
        );

        let events = run_paste(PasteRequest {
            id: OperationRequestId(1),
            ..paste_of(&sources, &destination, &["folder"], moving)
        });

        let copied = destination.join("folder");
        assert_eq!(
            reported_renames(&events),
            [RenamedItem {
                requested: Location::local(copied.join("makefile")),
                saved_as: Location::local(copied.join("makefile (1)")),
                cause: RenameCause::Alias,
            }],
            "{row}"
        );
        assert_eq!(names(&copied)?, ["makefile", "makefile (1)"], "{row}");
        expect_contents(
            &[
                (copied.join("makefile"), "upper"),
                (copied.join("makefile (1)"), "lower"),
            ],
            row,
        )?;
        assert_eq!(folder.exists(), !moving, "{row}");
    }
    Ok(())
}

/// A row of a paste into a destination where another item holds a name it would save.
struct TakenRow {
    row: &'static str,
    theirs: &'static str,
    items: Vec<String>,
    after_saved: bool,
}

fn taken_rows() -> [TakenRow; 5] {
    let padding = (0..8).map(|index| format!("item {index}"));
    let spellings = ["Makefile", "makefile"].map(String::from);
    let at_once = || spellings.iter().cloned().chain(padding.clone()).collect();
    // Past eight files a paste copies several at once.
    [
        TakenRow {
            row: "theirs at the same spelling",
            theirs: "item 3",
            items: padding
                .clone()
                .chain(["item 8", "item 9"].map(String::from))
                .collect(),
            after_saved: false,
        },
        TakenRow {
            row: "theirs at another spelling, at once",
            theirs: "MAKEFILE",
            items: at_once(),
            after_saved: false,
        },
        TakenRow {
            row: "theirs at another spelling, one at a time",
            theirs: "MAKEFILE",
            items: spellings.to_vec(),
            after_saved: false,
        },
        TakenRow {
            row: "theirs once the earlier spelling was saved",
            theirs: "MAKEFILE",
            items: spellings.to_vec(),
            after_saved: true,
        },
        TakenRow {
            row: "theirs once the earlier spelling was saved, at once",
            theirs: "MAKEFILE",
            items: at_once(),
            after_saved: true,
        },
    ]
}

/// Puts `theirs` in the destination now, or has a racer move the saved `Makefile` out to
/// `moved_out` and take its name once the paste looks up the later spelling.
fn place_theirs(
    row: &TakenRow,
    destination: &Path,
    moved_out: &Path,
) -> io::Result<Vec<(Op, &'static str, usize, Racer)>> {
    if !row.after_saved {
        fs::write(destination.join(row.theirs), b"theirs")?;
        return Ok(Vec::new());
    }
    let (saved, taken, moved_out) = (
        destination.join("Makefile"),
        destination.join(row.theirs),
        moved_out.to_path_buf(),
    );
    let racer: Racer = Box::new(move || {
        fs::rename(&saved, &moved_out).expect("move the saved item out");
        fs::write(&taken, b"theirs").expect("take its name");
    });
    Ok(vec![(Op::Stat, "makefile", 0, racer)])
}

fn check_taken_row(row: &TakenRow) -> Result<(), Box<dyn Error>> {
    let (root, source, destination) = source_and_destination()?;
    for item in &row.items {
        let path = source.join(item);
        fs::create_dir_all(path.parent().ok_or("no parent")?)?;
        fs::write(path, item)?;
    }
    let moved_out = root.path().join("moved out");
    let before = place_theirs(row, &destination, &moved_out)?;
    let _source = FsQuirks::install(&source, Quirks::ext4());
    let _destination = FsQuirks::install(
        &destination,
        Quirks {
            before,
            ..casefolded_ext4()
        },
    );
    let items = row.items.iter().map(String::as_str).collect::<Vec<_>>();

    let events = paste_until_settled(paste_of(&source, &destination, &items, false));

    let name = row.row;
    assert!(
        matches!(
            terminal_transfer(&events),
            Some(OperationEvent::TransferFailed { .. })
        ),
        "{name}: {events:?}"
    );
    assert_eq!(fs::read(destination.join(row.theirs))?, b"theirs", "{name}");
    let saved = names(&destination)?;
    assert!(
        saved.iter().all(|name| !name.ends_with(" (1)")),
        "{name}: {saved:?}"
    );
    assert_eq!(moved_out.exists(), row.after_saved, "{name}");
    Ok(())
}

#[test]
fn an_item_already_at_a_top_level_name_is_never_taken_for_one_the_paste_saved()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for row in taken_rows() {
        check_taken_row(&row)?;
    }
    Ok(())
}
