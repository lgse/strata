// SPDX-License-Identifier: MIT

use super::*;

/// Two items to move, each already present at the destination.
fn seed_conflicting_items(sources: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir_all(sources)?;
    fs::create_dir_all(destination)?;
    fs::write(sources.join("replace.txt"), b"new replacement")?;
    fs::write(sources.join("late.txt"), b"new late item")?;
    fs::write(destination.join("replace.txt"), b"old replacement")?;
    fs::write(destination.join("late.txt"), b"late arrival")
}

#[test]
fn each_transfer_item_keeps_its_own_conflict_decision() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-conflict-decisions-test-{unique}"));
    let sources = root.join("sources");
    let destination = root.join("destination");
    seed_conflicting_items(&sources, &destination)?;

    let events = paste_until(
        paste_request(
            1,
            &destination,
            vec![
                item(
                    sources.join("replace.txt"),
                    TransferConflict::ReplaceExisting,
                ),
                item(sources.join("late.txt"), TransferConflict::FailIfExists),
            ],
            true,
        ),
        settled,
    );

    // Items without a conflict go first, so the late arrival stops the paste before Replace.
    assert!(matches!(
        events.last(),
        Some(OperationEvent::TransferFailed {
            completed_locations,
            ..
        }) if completed_locations.is_empty()
    ));
    assert!(item_moves(&events).is_empty());
    assert!(trashed_items(&events).is_empty());
    expect_contents(&[
        (destination.join("replace.txt"), b"old replacement"),
        (destination.join("late.txt"), b"late arrival"),
    ])?;
    expect_present(&[
        (sources.join("replace.txt"), true),
        (sources.join("late.txt"), true),
    ]);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn mixed_conflict_choices_apply_independently_across_a_multi_item_paste()
-> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new()?;
    let sources = scratch.path().join("sources");
    let destination = scratch.path().join("destination");
    fs::create_dir_all(&sources)?;
    fs::create_dir_all(&destination)?;
    fs::write(sources.join("new.txt"), b"brand new")?;
    fs::write(sources.join("replace.txt"), b"new replacement")?;
    fs::write(destination.join("replace.txt"), b"old replacement")?;
    fs::write(sources.join("keep.txt"), b"new keep")?;
    fs::write(destination.join("keep.txt"), b"old keep")?;

    let events = paste_until(
        paste_request(
            76,
            &destination,
            vec![
                item(sources.join("new.txt"), TransferConflict::FailIfExists),
                item(
                    sources.join("replace.txt"),
                    TransferConflict::ReplaceExisting,
                ),
                item(sources.join("keep.txt"), TransferConflict::KeepBoth),
            ],
            false,
        ),
        settled,
    );

    expect_last_pasted(&events);
    expect_contents(&[
        (destination.join("new.txt"), b"brand new"),
        (destination.join("replace.txt"), b"new replacement"),
        (destination.join("keep.txt"), b"old keep"),
        (destination.join("keep (1).txt"), b"new keep"),
    ])?;
    Ok(())
}

/// A paste onto a mount whose prompt confirmed `confirmed`, which the mount
/// may store beside `fixed`.
struct Confirmed {
    mount: &'static str,
    quirks: Preset,
    confirmed: &'static str,
    fixed: Option<&'static str>,
    conflict: TransferConflict,
}

impl Confirmed {
    fn context(&self) -> String {
        let Self {
            mount,
            confirmed,
            fixed,
            conflict,
            ..
        } = self;
        format!("{mount} {confirmed} beside {fixed:?}, {conflict:?}")
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
        for item in std::iter::once(self.confirmed).chain(self.fixed) {
            if self.merging() {
                fs::create_dir_all(disk.join(item))?;
            }
            fs::write(disk.join(self.file(item)), item)?;
        }
        if self.merging() {
            fs::create_dir(source.join(self.confirmed))?;
        }
        fs::write(source.join(self.file(self.confirmed)), "incoming")
    }

    /// The confirmed name took the incoming item and the fixed one kept its own.
    fn expect_acted_on_confirmed(&self, disk: &Path) -> Result<(), Box<dyn Error>> {
        let context = self.context();
        assert_eq!(
            fs::read(disk.join(self.file(self.confirmed)))?,
            b"incoming",
            "{context}"
        );
        if let Some(fixed) = self.fixed {
            assert_eq!(
                fs::read(disk.join(self.file(fixed)))?,
                fixed.as_bytes(),
                "{context}"
            );
        }
        assert_eq!(
            names(disk)?.len(),
            1 + usize::from(self.fixed.is_some()),
            "{context}"
        );
        Ok(())
    }
}

#[test]
fn replace_and_merge_act_on_the_name_the_prompt_confirmed() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let (replace, merge) = (TransferConflict::ReplaceExisting, TransferConflict::Merge);
    let (vfat, ntfs3, cifs): (Preset, Preset, Preset) = (Quirks::vfat, Quirks::ntfs3, Quirks::cifs);
    let cases = [
        ("vfat", vfat, "CON", Some("CON_"), replace),
        ("ntfs3", ntfs3, "a:b", Some("a_b"), replace),
        ("ntfs3", ntfs3, "a:b", None, replace),
        ("cifs", cifs, "a:b", Some("a_b"), replace),
        ("ntfs3", ntfs3, "x:y", Some("x_y"), merge),
        ("ntfs3", ntfs3, "x:y", None, merge),
    ]
    .map(|(mount, quirks, confirmed, fixed, conflict)| Confirmed {
        mount,
        quirks,
        confirmed,
        fixed,
        conflict,
    });
    for case in cases {
        let root = tempfile::tempdir()?;
        let (source, disk) = (root.path().join("source"), root.path().join("disk"));
        case.seed(&source, &disk)?;
        let _disk = FsQuirks::install(&disk, (case.quirks)());

        run_paste(paste_request(
            1615,
            &disk,
            vec![item(source.join(case.confirmed), case.conflict)],
            false,
        ));

        case.expect_acted_on_confirmed(&disk)?;
    }
    Ok(())
}
