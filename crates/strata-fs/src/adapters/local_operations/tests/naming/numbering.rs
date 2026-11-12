// SPDX-License-Identifier: MIT

use super::super::super::{
    names::{Key, PlanEntry, Planned, plan_names},
    paste_targets, transfer_source_name,
};
use super::*;
use crate::adapters::volume::NameLimit;

#[test]
fn copy_suffix_parsing_and_candidate_naming() {
    for (input, stem, suffix) in [
        ("name", "name", None),
        ("name (1)", "name", Some(1u64)),
        ("name (2)", "name", Some(2)),
        ("name (42)", "name", Some(42)),
        ("name (foo)", "name (foo)", None),
        ("name (0)", "name (0)", None),
        ("name (2", "name (2", None),
        (
            "name (18446744073709551615)",
            "name (18446744073709551615)",
            None,
        ),
        (".gitignore", ".gitignore", None),
        (".gitignore (1)", ".gitignore", Some(1)),
    ] {
        assert_eq!(
            parse_copy_suffix(OsStr::new(input)),
            (OsStr::new(stem), suffix)
        );
    }

    for (stem, extension, number, expected) in [
        ("name", ".ext", 1u64, "name (1).ext"),
        ("name", ".ext", 2, "name (2).ext"),
        ("name", "", 1, "name (1)"),
        ("name", "", 2, "name (2)"),
        (".gitignore", "", 1, ".gitignore (1)"),
        (".config", "", 2, ".config (2)"),
        ("archive", ".tar.gz", 1, "archive (1).tar.gz"),
        ("archive", ".tar.gz", 3, "archive (3).tar.gz"),
        ("backup.tar", ".gz", 1, "backup.tar (1).gz"),
    ] {
        assert_eq!(
            numbered_name(
                OsStr::new(stem),
                number,
                OsStr::new(extension),
                NameLimit::default(),
                false
            ),
            OsString::from(expected)
        );
    }
    assert_eq!(
        numbered_names(
            OsStr::new("a (18446744073709551614).txt"),
            false,
            NameLimit::default()
        )
        .take(2)
        .collect::<Vec<_>>(),
        ["a (18446744073709551615).txt", "a (1).txt"]
    );
}

/// An entry's name, whether it is a folder, whether it is fixed, the name
/// planned for it, and why that differs.
type Sibling = (String, bool, bool, String, Option<RenameCause>);

fn fixed_sibling(name: &str, is_directory: bool, planned: &str) -> Sibling {
    let cause = (name != planned).then_some(RenameCause::Invalid);
    (
        name.to_owned(),
        is_directory,
        true,
        planned.to_owned(),
        cause,
    )
}

fn file(name: &str, planned: &str) -> Sibling {
    fixed_sibling(name, false, planned)
}

fn folder(name: &str, planned: &str) -> Sibling {
    fixed_sibling(name, true, planned)
}

fn alias(name: &str, is_directory: bool, planned: &str) -> Sibling {
    (
        name.to_owned(),
        is_directory,
        true,
        planned.to_owned(),
        Some(RenameCause::Alias),
    )
}

fn unfixed(name: &str) -> Sibling {
    (name.to_owned(), false, false, name.to_owned(), None)
}

/// Rows whose fixed names are numbered past names with invalid characters.
fn invalid_sibling_rows() -> Vec<(Key, Vec<Sibling>)> {
    let cjk = |count| "\u{4e2d}".repeat(count);
    vec![
        (
            Key::Folded,
            vec![file("a:b", "a_b (1)"), file("a_b", "a_b")],
        ),
        (
            Key::Folded,
            vec![
                file("a?b", "a_b (3)"),
                file("a_b (1)", "a_b (1)"),
                file("a:b", "a_b (2)"),
                file("a_b", "a_b"),
            ],
        ),
        (
            Key::Folded,
            vec![file("A:b", "A_b (1)"), file("a_b", "a_b")],
        ),
        (Key::Exact, vec![file("A:b", "A_b"), file("a_b", "a_b")]),
        (Key::Folded, vec![unfixed("x:y"), file("x?y", "x_y")]),
        (
            Key::Folded,
            vec![folder("v1.2:", "v1.2_ (1)"), folder("v1.2_", "v1.2_")],
        ),
        (
            Key::Folded,
            vec![
                file(
                    &format!("{}.txt.", cjk(250)),
                    &format!("{} (1).txt", cjk(247)),
                ),
                file(&format!("{}.txt", cjk(250)), &format!("{}.txt", cjk(250))),
            ],
        ),
    ]
}

/// Rows whose fixed names are numbered past names that fold to the same key.
fn alias_sibling_rows() -> Vec<(Key, Vec<Sibling>)> {
    vec![
        (
            Key::Folded,
            vec![file("A.txt", "A.txt"), alias("a.txt", false, "a (1).txt")],
        ),
        (
            Key::Folded,
            vec![
                alias("a.txt", false, "a (2).txt"),
                file("a (1).txt", "a (1).txt"),
                file("A.txt", "A.txt"),
            ],
        ),
        (
            Key::Folded,
            vec![
                alias("a_b", false, "a_b (2)"),
                file("a:b", "a_b (1)"),
                file("A_b", "A_b"),
            ],
        ),
        (
            Key::Folded,
            vec![
                folder("Photos.2024", "Photos.2024"),
                alias("photos.2024", true, "photos.2024 (1)"),
            ],
        ),
        (
            Key::Folded,
            vec![unfixed("a.txt"), alias("A.txt", false, "A (1).txt")],
        ),
        (
            Key::Exact,
            vec![file("A.txt", "A.txt"), file("a.txt", "a.txt")],
        ),
    ]
}

fn assert_planned_as(profile: &NameProfile, key: Key, row: &[Sibling]) {
    let entries = row
        .iter()
        .map(|(name, is_directory, fixing, ..)| PlanEntry {
            name: OsStr::new(name),
            is_directory: *is_directory,
            fixing: *fixing,
            repeatable: false,
        })
        .collect::<Vec<_>>();
    let planned = row
        .iter()
        .map(|(.., planned, cause)| Planned {
            target: OsString::from(planned),
            cause: *cause,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        plan_names(&entries, profile, key),
        Ok(planned),
        "{:?} {key:?} {row:?}",
        profile.folds
    );
}

#[test]
fn fixed_names_are_numbered_past_the_names_their_siblings_keep() {
    // NTFS keeps names that differ in case apart, so only fixed names avoid them.
    let ntfs = NameProfile::windows(Reserved::Bare, NameEncoding::Utf8);
    let case_kept = [
        vec![file("A.txt", "A.txt"), file("a.txt", "a.txt")],
        vec![file("a:b", "a_b (1)"), file("A_b", "A_b")],
    ];
    let rows = invalid_sibling_rows()
        .into_iter()
        .chain(alias_sibling_rows())
        .map(|(key, row)| (fat(), key, row))
        .chain(case_kept.map(|row| (ntfs, Key::Folded, row)));
    for (profile, key, row) in rows {
        for reversed in [false, true] {
            let mut row = row.clone();
            if reversed {
                row.reverse();
            }
            assert_planned_as(&profile, key, &row);
        }
    }
}

#[test]
fn planning_fails_where_a_fixed_name_has_no_free_number() {
    use super::super::super::placement::{NUMBERED_NAME_ATTEMPTS, PlaceError};
    let taken = std::iter::once(OsString::from("a_b"))
        .chain((1..=NUMBERED_NAME_ATTEMPTS).map(|number| OsString::from(format!("a_b ({number})"))))
        .collect::<Vec<_>>();
    let entries = taken
        .iter()
        .map(|name| PlanEntry {
            name,
            is_directory: false,
            fixing: false,
            repeatable: false,
        })
        .chain([PlanEntry {
            name: OsStr::new("a:b"),
            is_directory: false,
            fixing: true,
            repeatable: false,
        }])
        .collect::<Vec<_>>();
    assert_eq!(
        plan_names(&entries, &fat(), Key::Exact),
        Err(PlaceError::Exists)
    );
}

/// A name, whether it names a folder, the limit of its mount, and the first
/// numbered name within that limit.
type Cut = (String, bool, NameLimit, String);

fn cut_to_posix_bytes() -> Vec<Cut> {
    let bytes = NameProfile::posix(Some(255)).limit;
    let repeat = |text: &str, count| text.repeat(count);
    let cjk = "\u{4e2d}";
    vec![
        (
            format!("{}.txt", repeat("a", 251)),
            false,
            bytes,
            format!("{} (1).txt", repeat("a", 247)),
        ),
        (
            format!("{} (1).txt", repeat("a", 247)),
            false,
            bytes,
            format!("{} (2).txt", repeat("a", 247)),
        ),
        (
            format!("{}.tar.gz", repeat("a", 247)),
            false,
            bytes,
            format!("{}.tar (1).gz", repeat("a", 244)),
        ),
        (
            format!("{}.v2", repeat("d", 250)),
            false,
            bytes,
            format!("{} (1).v2", repeat("d", 248)),
        ),
        (
            format!("{}.v2", repeat("d", 250)),
            true,
            bytes,
            format!("{}. (1)", repeat("d", 250)),
        ),
        (
            format!("x.{}", repeat("a", 250)),
            false,
            bytes,
            format!("x.{} (1)", repeat("a", 249)),
        ),
        (
            repeat(cjk, 85),
            false,
            bytes,
            format!("{} (1)", repeat(cjk, 83)),
        ),
    ]
}

fn cut_to_units_and_smb_limits() -> Vec<Cut> {
    let bytes = NameProfile::posix(Some(255)).limit;
    let units = NameProfile::windows(Reserved::Bare, NameEncoding::Utf8).limit;
    let smb = NameProfile::smb(Some(255)).limit;
    let repeat = |text: &str, count| text.repeat(count);
    let (cjk, emoji, flag) = ("\u{4e2d}", "\u{1f600}", "\u{1f1eb}\u{1f1f7}");
    vec![
        (
            repeat(cjk, 85),
            false,
            units,
            format!("{} (1)", repeat(cjk, 85)),
        ),
        (
            repeat(emoji, 126),
            false,
            units,
            format!("{} (1)", repeat(emoji, 125)),
        ),
        (
            format!("abcd{}", repeat(flag, 31)),
            false,
            bytes,
            format!("abcd{} (1)", repeat(flag, 30)),
        ),
        (
            repeat("a", 212),
            true,
            smb,
            format!("{} (1)", repeat("a", 209)),
        ),
        (
            repeat("a", 212),
            false,
            smb,
            format!("{} (1)", repeat("a", 212)),
        ),
    ]
}

#[test]
fn numbered_names_are_cut_to_the_limit_of_their_mount() {
    for (name, folder, limit, expected) in cut_to_posix_bytes()
        .into_iter()
        .chain(cut_to_units_and_smb_limits())
    {
        assert_eq!(
            numbered_names(OsStr::new(&name), folder, limit).next(),
            Some(OsString::from(&expected)),
            "{name:?} {limit:?}"
        );
    }
}

#[test]
fn pasted_names_fixed_for_fat_are_planned_after_the_names_kept() {
    use gio::FileType::{Directory, Regular};
    let sources = [
        "/one/a:b",
        "/two/a_b",
        "/one/v1.2:",
        "/two/v1.2:",
        "/one/A:c",
        "/two/a_c",
        "/one/x",
        "/two/X",
        "/three/x",
    ]
    .map(|source| transfer_source_name(&gio::File::for_path(source)));
    let mut kinds = [Some(Regular); 9];
    kinds[3] = Some(Directory);
    let (invalid, alias, taken) = (
        Some(RenameCause::Invalid),
        Some(RenameCause::Alias),
        Some(RenameCause::Taken),
    );
    // Names repeated from another folder are numbered on any filesystem.
    for (fat_family, expected) in [
        (
            false,
            [
                ("a:b", None),
                ("a_b", None),
                ("v1.2:", None),
                ("v1.2: (1)", taken),
                ("A:c", None),
                ("a_c", None),
                ("x (1)", alias),
                ("X", None),
                ("x (2)", taken),
            ],
        ),
        (
            true,
            [
                ("a_b (1)", invalid),
                ("a_b", None),
                ("v1.2_", invalid),
                ("v1.2_ (1)", invalid),
                ("A_c (1)", invalid),
                ("a_c", None),
                ("x (1)", alias),
                ("X", None),
                ("x (2)", taken),
            ],
        ),
    ] {
        let crossing = [fat_family; 9];
        let targets = paste_targets(
            &sources,
            &fat(),
            (&crossing, &[true; 9], &[false; 9]),
            &kinds,
        )
        .expect("free names");
        assert_eq!(
            targets,
            expected.map(|(name, cause)| Some(Planned {
                target: OsString::from(name),
                cause,
            })),
            "{fat_family}"
        );
    }
}

#[test]
fn transfer_targets_resolve_trash_leaf_names() {
    let target_name = |source: &gio::File| {
        let planned = paste_targets(
            &[transfer_source_name(source)],
            &NameProfile::posix(Some(255)),
            (&[false], &[true], &[false]),
            &[None],
        )
        .ok()?
        .pop()??;
        Some(planned.target)
    };
    let local = gio::File::for_path("/home/user/notes.txt");
    assert_eq!(target_name(&local), Some(OsString::from("notes.txt")));

    let home_trash = gio::File::for_uri("trash:///report.txt");
    assert_eq!(target_name(&home_trash), Some(OsString::from("report.txt")));

    let external_trash =
        gio::File::for_uri("trash:///%5Cmnt%5CData%5C.Trash-1000%5Cfiles%5CSAMPLED");
    assert_eq!(
        target_name(&external_trash),
        Some(OsString::from("SAMPLED"))
    );

    let external_trash_numbered =
        gio::File::for_uri("trash:///%5Cmnt%5CData%5C.Trash-1000%5Cfiles%5CSAMPLED.2");
    assert_eq!(
        target_name(&external_trash_numbered),
        Some(OsString::from("SAMPLED.2"))
    );

    let home_trash_with_backslash = gio::File::for_uri("trash:///foo%5Cbar.txt");
    assert_eq!(
        target_name(&home_trash_with_backslash),
        Some(OsString::from("foo\\bar.txt"))
    );
}
