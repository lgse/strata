// SPDX-License-Identifier: MIT

use super::super::super::transfer_targets::{
    Collision, LOOKUPS_WITHOUT_LISTING, TransferTarget, transfer_targets,
};
use super::*;
use crate::services::RenameCause;

/// What a source meets: its name and what has it on the mount.
type Met = (&'static str, Option<Collision>);

/// A source: its name and what it meets on vfat, then on ext4, and whether
/// names inside it change on vfat. `own.txt` and `same` are pasted from
/// the destination itself, `old.txt` from a Trash on another disk, and
/// `holder` from inside the folder of its name.
fn prompt_rows() -> [(&'static str, Met, Met, bool); 11] {
    let collision = |mergeable, replaceable| {
        Some(Collision {
            mergeable,
            replaceable,
        })
    };
    let (file, folder, file_on_folder) = (
        collision(false, true),
        collision(true, true),
        collision(false, false),
    );
    let folder_holding_it = file;
    [
        ("a:b", ("a_b", file), ("a:b", None), false),
        ("Photo.jpg", ("Photo.jpg", file), ("Photo.jpg", None), false),
        ("x:y", ("x_y", folder), ("x:y", None), true),
        ("folder", ("folder", folder), ("folder", folder), true),
        ("same", ("same", folder), ("same", folder), false),
        (
            "holder",
            ("holder", folder_holding_it),
            ("holder", folder_holding_it),
            false,
        ),
        (
            "data",
            ("data", file_on_folder),
            ("data", file_on_folder),
            false,
        ),
        ("link", ("link", None), ("link", file), false),
        ("new.txt", ("new.txt", None), ("new.txt", None), false),
        ("own.txt", ("own.txt", None), ("own.txt", None), false),
        ("old.txt", ("old.txt", file), ("old.txt", file), false),
    ]
}

fn seed_prompt_items(source: &Path, disk: &Path, pads: &[String]) -> io::Result<()> {
    for name in ["x:y", "folder"] {
        fs::create_dir_all(source.join(name))?;
    }
    for name in ["a:b", "Photo.jpg", "data", "new.txt"]
        .into_iter()
        .chain(pads.iter().map(String::as_str))
    {
        fs::write(source.join(name), name)?;
    }
    std::os::unix::fs::symlink("a:b", source.join("link"))?;
    for name in [
        "x_y",
        "folder",
        "data",
        "same",
        "inner/same",
        "holder/holder",
    ] {
        fs::create_dir_all(disk.join(name))?;
    }
    for name in ["a_b", "photo.JPG", "link", "own.txt", "old.txt"] {
        fs::write(disk.join(name), name)?;
    }
    Ok(())
}

fn prompt_sources(source: &Path, disk: &Path, pads: &[String]) -> Vec<Location> {
    prompt_rows()
        .iter()
        .map(|(name, ..)| match *name {
            "own.txt" => Location::local(disk.join(name)),
            "same" => Location::local(disk.join("inner").join(name)),
            "holder" => Location::local(disk.join("holder").join(name)),
            "old.txt" => Location::uri(format!(
                "trash:///%5Cmnt%5CData%5C.Trash-1000%5Cfiles%5C{name}"
            )),
            _ => Location::local(source.join(name)),
        })
        .chain(pads.iter().map(|pad| Location::local(source.join(pad))))
        .collect()
}

fn expected_targets(on_vfat: bool, pads: &[String]) -> Vec<TransferTarget> {
    prompt_rows()
        .iter()
        .map(|(item, vfat, ext4, fixes_inside)| {
            let (name, collision) = if on_vfat { *vfat } else { *ext4 };
            TransferTarget {
                name: Some(ShownName {
                    name: OsString::from(name),
                    cause: (name != *item).then_some(RenameCause::Invalid),
                }),
                collision,
                fixes_inside: on_vfat && *fixes_inside,
            }
        })
        .chain(pads.iter().map(|pad| TransferTarget {
            name: Some(ShownName {
                name: OsString::from(pad),
                cause: None,
            }),
            collision: None,
            fixes_inside: false,
        }))
        .collect()
}

#[test]
fn the_prompt_sees_the_name_a_paste_gives_each_item_and_what_has_it() -> Result<(), Box<dyn Error>>
{
    let _serial = serial()?;
    let (vfat, ext4): (Preset, Preset) = (Quirks::vfat, Quirks::ext4);
    // Enough more items make the lookup read the destination's listing.
    for ((preset, on_vfat), padding) in [(vfat, true), (ext4, false)]
        .into_iter()
        .flat_map(|mount| [(mount, 0), (mount, LOOKUPS_WITHOUT_LISTING)])
    {
        let pads = (0..padding)
            .map(|index| format!("pad-{index:02}"))
            .collect::<Vec<_>>();
        let root = tempfile::tempdir()?;
        let (source, disk) = (root.path().join("source"), root.path().join("disk"));
        seed_prompt_items(&source, &disk, &pads)?;
        let disk_quirks = FsQuirks::install(&disk, preset());
        let sources = prompt_sources(&source, &disk, &pads);

        let targets = glib::MainContext::default().block_on(transfer_targets(
            &Location::local(&disk),
            &sources,
            false,
            &gio::Cancellable::new(),
        ))?;

        let context = format!("vfat {on_vfat}, {padding} more items");
        assert_eq!(targets, expected_targets(on_vfat, &pads), "{context}");
        assert_eq!(
            disk_quirks.count(fs_quirks::Op::Names),
            usize::from(padding > 0),
            "{context}"
        );
    }
    Ok(())
}

/// An item pasted under the name its prompt showed: how it collided, the
/// name shown, whether it is moved, where it lands and why.
struct Shown {
    item: &'static str,
    conflict: TransferConflict,
    shown: &'static str,
    moving: bool,
    landed: &'static str,
    cause: RenameCause,
}

/// Where a paste left the item: what landed, how many entries the disk
/// holds, whether the source stayed, and the renames it reported.
#[derive(Debug, PartialEq)]
struct Landing {
    contents: Vec<u8>,
    entries: usize,
    source_kept: bool,
    renamed: Vec<(String, String, RenameCause)>,
}

impl Shown {
    fn context(&self) -> String {
        let Self {
            item,
            conflict,
            shown,
            moving,
            ..
        } = self;
        format!("{item} as {shown}, {conflict:?}, moving {moving}")
    }

    fn merging(&self) -> bool {
        self.conflict == TransferConflict::Merge
    }

    /// The file an item is checked through: itself, or one inside a merged folder.
    fn file(&self, item: &str) -> String {
        if self.merging() {
            format!("{item}/same.txt")
        } else {
            item.to_owned()
        }
    }

    fn seed(&self, source: &Path, disk: &Path) -> io::Result<()> {
        fs::create_dir_all(source)?;
        fs::create_dir_all(disk)?;
        if self.merging() {
            fs::create_dir(source.join(self.item))?;
            fs::create_dir(disk.join(self.shown))?;
        }
        fs::write(source.join(self.file(self.item)), "incoming")?;
        fs::write(disk.join(self.file(&self.shown.replace(" (1)", ""))), "old")
    }

    fn expected_landing(&self) -> Landing {
        Landing {
            contents: b"incoming".to_vec(),
            entries: 1 + usize::from(
                self.landed != self.shown || self.conflict == TransferConflict::FailIfExists,
            ),
            source_kept: !self.moving,
            renamed: vec![(self.item.to_owned(), self.landed.to_owned(), self.cause)],
        }
    }

    fn landing(
        &self,
        source: &Path,
        disk: &Path,
        events: &[OperationEvent],
    ) -> Result<Landing, Box<dyn Error>> {
        Ok(Landing {
            contents: fs::read(disk.join(self.file(self.landed)))?,
            entries: names(disk)?.len(),
            source_kept: source.join(self.item).exists(),
            renamed: renamed_beneath(disk, &followed_report(events)),
        })
    }
}

#[test]
fn a_paste_gives_an_item_the_name_its_prompt_showed() -> Result<(), Box<dyn Error>> {
    use crate::services::RenameCause::{Alias, Invalid};
    let _serial = serial()?;
    let (replace, keep_both, merge, fail) = (
        TransferConflict::ReplaceExisting,
        TransferConflict::KeepBoth,
        TransferConflict::Merge,
        TransferConflict::FailIfExists,
    );
    let cases = [
        ("a:b", replace, "a_b", false, "a_b", Invalid),
        ("a:b", keep_both, "a_b", false, "a_b (1)", Invalid),
        ("a:b", keep_both, "a_b", true, "a_b (1)", Invalid),
        ("a:b", fail, "a_b (1)", false, "a_b (1)", Invalid),
        ("b.jpg", fail, "b (1).jpg", false, "b (1).jpg", Alias),
        ("x:y", merge, "x_y", false, "x_y", Invalid),
    ]
    .map(|(item, conflict, shown, moving, landed, cause)| Shown {
        item,
        conflict,
        shown,
        moving,
        landed,
        cause,
    });
    for case in cases {
        let root = tempfile::tempdir()?;
        let (source, disk) = (root.path().join("source"), root.path().join("disk"));
        case.seed(&source, &disk)?;
        let _disk = FsQuirks::install(&disk, Quirks::vfat());

        let events = run_paste(PasteRequest {
            id: OperationRequestId(1615),
            destination: Location::local(&disk),
            items: vec![PasteItem {
                source: Location::local(source.join(case.item)),
                conflict: case.conflict,
                target_name: Some(ShownName {
                    name: OsString::from(case.shown),
                    cause: None,
                }),
            }],
            move_sources: case.moving,
        });

        assert_eq!(
            case.landing(&source, &disk, &events)?,
            case.expected_landing(),
            "{}",
            case.context()
        );
    }
    Ok(())
}

/// The paste completed or failed, either way.
fn ended(event: &OperationEvent) -> bool {
    matches!(
        event,
        OperationEvent::Pasted { .. }
            | OperationEvent::TransferFailed { .. }
            | OperationEvent::Failed { .. }
    )
}

#[test]
fn a_paste_fails_an_item_whose_shown_name_is_not_one_plain_name() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for shown in ["../escaped", "sub/x", "a\0b"] {
        let root = tempfile::tempdir()?;
        let (source, disk) = (root.path().join("source"), root.path().join("disk"));
        fs::create_dir_all(&source)?;
        fs::create_dir_all(disk.join("sub"))?;
        fs::write(source.join("item"), "incoming")?;

        let events = paste_until(
            PasteRequest {
                id: OperationRequestId(1615),
                destination: Location::local(&disk),
                items: vec![PasteItem {
                    source: Location::local(source.join("item")),
                    conflict: TransferConflict::FailIfExists,
                    target_name: Some(ShownName {
                        name: OsString::from(shown),
                        cause: None,
                    }),
                }],
                move_sources: false,
            },
            ended,
        );

        assert!(
            matches!(events.last(), Some(OperationEvent::Failed { .. })),
            "{shown:?}"
        );
        expect_entries(&[
            (root.path(), &["disk", "source"]),
            (&disk, &["sub"]),
            (&disk.join("sub"), &[]),
        ])?;
    }
    Ok(())
}
