// SPDX-License-Identifier: MIT

//! Names a copied folder's destination takes for one another.

use std::os::unix::fs::MetadataExt;

use super::*;

/// `Docs` and `docs`, `Makefile`, `makefile` and `makefile (1)`, and links `Link` and `link`.
fn write_case_pairs(source: &Path) -> io::Result<()> {
    fs::create_dir_all(source.join("Docs"))?;
    fs::create_dir_all(source.join("docs"))?;
    fs::write(source.join("Docs/upper.txt"), b"upper")?;
    fs::write(source.join("docs/lower.txt"), b"lower")?;
    for name in ["Makefile", "makefile", "makefile (1)"] {
        fs::write(source.join(name), name)?;
    }
    std::os::unix::fs::symlink("upper", source.join("Link"))?;
    std::os::unix::fs::symlink("lower", source.join("link"))
}

fn sorted<T: Ord>(mut items: Vec<T>) -> Vec<T> {
    items.sort();
    items
}

fn expect_links(expected: &[(PathBuf, &str)]) -> io::Result<()> {
    for (link, target) in expected {
        assert_eq!(fs::read_link(link)?, Path::new(target));
    }
    Ok(())
}

#[test]
fn case_aliases_inside_a_copied_folder_are_numbered_and_reported() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    // Names are claimed in byte order where they may collide, so the later
    // one is numbered; elsewhere the one made last is.
    let rows: [(&str, Preset, &str, &str); 2] = [
        ("claimed", ciopfs, "makefile", "makefile (2)"),
        ("made", casefolded_ext4, "Makefile", "Makefile (2)"),
    ];
    for (row, preset, aliased, saved_as) in rows {
        let (_root, source, destination) = source_and_destination()?;
        write_case_pairs(&source)?;
        let _sources = FsQuirks::install(&source, Quirks::default());
        let _destination = FsQuirks::install(&destination, preset());
        let target = destination.join("copy");

        let renamed = copy_renaming(&source, &target, Some(1))?;

        let expected = owned_pairs(&[
            ("docs", "docs (1)"),
            ("link", "link (1)"),
            (aliased, saved_as),
        ]);
        assert_eq!(sorted(renamed), sorted(expected), "{row}");
        let other = if aliased == "makefile" {
            "Makefile"
        } else {
            "makefile"
        };
        let mut expected = vec!["Docs", "Link", "docs (1)", "link (1)", "makefile (1)"];
        expected.extend([other, saved_as]);
        expected.sort_unstable();
        assert_eq!(names(&target)?, expected, "{row}");
        // The sibling `makefile (1)` keeps its own name.
        expect_contents(
            &[
                (target.join(saved_as), aliased),
                (target.join(other), other),
                (target.join("makefile (1)"), "makefile (1)"),
                (target.join("Docs/upper.txt"), "upper"),
                (target.join("docs (1)/lower.txt"), "lower"),
            ],
            row,
        )?;
        expect_links(&[
            (target.join("Link"), "upper"),
            (target.join("link (1)"), "lower"),
        ])?;
    }
    Ok(())
}

#[test]
fn aliases_of_long_names_are_numbered_within_the_limit() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let (upper, lower) = (
        format!("A{}.txt", "a".repeat(250)),
        format!("{}.txt", "a".repeat(251)),
    );
    let rows: [(&str, Preset, &str, String); 2] = [
        (
            "claimed",
            ciopfs,
            &lower,
            format!("{} (1).txt", "a".repeat(247)),
        ),
        (
            "made",
            casefolded_ext4,
            &upper,
            format!("A{} (1).txt", "a".repeat(246)),
        ),
    ];
    for (row, preset, aliased, saved_as) in rows {
        let (_root, source, destination) = source_and_destination()?;
        fs::create_dir(&source)?;
        fs::write(source.join(&upper), "upper")?;
        fs::write(source.join(&lower), "lower")?;
        let _destination = FsQuirks::install(&destination, preset());
        let target = destination.join("copy");

        let renamed = copy_renaming(&source, &target, Some(1))?;

        assert_eq!(renamed, [(aliased.to_owned(), saved_as.clone())], "{row}");
        let kept = if aliased == upper { &lower } else { &upper };
        let mut expected = vec![kept.clone(), saved_as.clone()];
        expected.sort();
        assert_eq!(names(&target)?, expected, "{row}");
    }
    Ok(())
}

#[test]
fn a_linked_name_that_aliases_another_is_numbered_and_reported() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let (_root, source, destination) = source_and_destination()?;
    fs::create_dir_all(&source)?;
    fs::write(source.join("a"), b"linked")?;
    fs::hard_link(source.join("a"), source.join("B"))?;
    fs::write(source.join("b"), b"other")?;
    let _sources = FsQuirks::install(&source, Quirks::default());
    let _destination = FsQuirks::install(&destination, casefolded_ext4());
    let target = destination.join("copy");

    // One worker takes the folder's files last first, so `B` is linked
    // after `b` is made.
    let renamed = copy_renaming(&source, &target, Some(1))?;

    assert_eq!(renamed, [("B".to_owned(), "B (1)".to_owned())]);
    assert_eq!(names(&target)?, ["B (1)", "a", "b"]);
    assert_eq!(
        fs::metadata(target.join("B (1)"))?.ino(),
        fs::metadata(target.join("a"))?.ino()
    );
    expect_contents(&[(target.join("b"), "other")], "the unlinked file")?;
    Ok(())
}

#[test]
fn names_a_fat_target_takes_for_one_are_numbered_and_reported() -> Result<(), Box<dyn Error>> {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT.lock()?;
    let (_root, source, destination) = source_and_destination()?;
    fs::create_dir_all(&source)?;
    // `a:b` would be saved as `a_b`, which the real `a_b` keeps.
    for name in ["Makefile", "a:b", "a_b", "makefile"] {
        fs::write(source.join(name), name)?;
    }
    let _sources = FsQuirks::install(&source, Quirks::default());
    let _destination = FsQuirks::install(&destination, Quirks::vfat());
    let target = destination.join("copy");
    let options = super::super::super::copy_engine::Options {
        names: NamePolicy::new(NameMode::Fix),
        ..super::super::super::copy_engine::Options::default()
    };

    let (copied, report) = reporting(
        &source,
        super::super::super::copy_engine::Destination::New(target.clone(), Taken::Fail),
        options,
    );

    copied?;
    assert_eq!(
        renamed_beneath(&target, &report),
        [
            ("a:b", "a_b (1)", RenameCause::Invalid),
            ("makefile", "makefile (1)", RenameCause::Alias),
        ]
        .map(|(name, saved_as, cause)| (name.to_owned(), saved_as.to_owned(), cause))
    );
    assert_eq!(
        names(&target)?,
        ["Makefile", "a_b", "a_b (1)", "makefile (1)"]
    );
    expect_contents(
        &[
            (target.join("a_b"), "a_b"),
            (target.join("a_b (1)"), "a:b"),
            (target.join("makefile (1)"), "makefile"),
        ],
        "fat",
    )?;
    Ok(())
}
