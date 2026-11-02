// SPDX-License-Identifier: MIT

//! Top-level items a paste's destination takes for one another.

use super::super::super::transfer_targets::transfer_targets;
use super::super::fs_quirks::Op;
use super::*;
use crate::services::ShownName;

fn both_ways() -> impl Iterator<Item = (bool, bool)> + Clone {
    [false, true]
        .into_iter()
        .flat_map(|first| [false, true].map(|second| (first, second)))
}

/// Eight more files, past which a paste copies several at once.
fn padding(prefix: &str) -> Vec<String> {
    (0..8).map(|index| format!("{prefix}pad {index}")).collect()
}

fn case_insensitive_sftp_server() -> Quirks {
    Quirks {
        case_insensitive: true,
        alias_inodes: true,
        ..Quirks::sshfs()
    }
}

fn check_case_pairs_pasted(
    preset: Preset,
    moving: bool,
    parallel: bool,
    context: &str,
) -> Result<(), Box<dyn Error>> {
    let (_root, source, destination) = source_and_destination()?;
    fs::create_dir_all(&source)?;
    let padding = padding("");
    let mut items = vec!["Makefile", "makefile"];
    if parallel {
        items.extend(padding.iter().map(String::as_str));
    } else {
        items.extend(["Docs", "docs"]);
    }
    write_items(&source, &items, "ocs")?;
    let sources = FsQuirks::install(&source, Quirks::ext4());
    let _destination = FsQuirks::install(&destination, preset());

    let events = run_paste(paste_of(&source, &destination, &items, moving));

    let numbered = if parallel {
        vec!["makefile"]
    } else {
        vec!["docs", "makefile"]
    };
    assert_eq!(
        renamed_beneath(&destination, &followed_report(&events)),
        numbered_once(&numbered, RenameCause::Alias),
        "{context}"
    );
    for name in numbered {
        let saved_as = destination.join(format!("{name} (1)"));
        assert_eq!(
            read_item(&saved_as, name.ends_with("ocs"))?,
            name,
            "{context}"
        );
    }
    assert_eq!(names(&destination)?.len(), items.len(), "{context}");
    assert_eq!(names(&source)?.is_empty(), moving, "{context}");
    assert_eq!(
        sources.count(Op::OpenSource),
        items.len(),
        "{context}: each file is read once"
    );
    Ok(())
}

#[test]
fn top_level_items_a_destination_takes_for_an_earlier_one_are_numbered_and_reported()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // Multigrain times, so a move reads its just-written sources only to
    // copy them, not again to compare them.
    let _kernel = KernelGuard::set(Some((6, 13, 0)));
    let mounts: [(&str, Preset); 3] = [
        ("casefolded ext4", casefolded_ext4),
        ("cifs", Quirks::cifs),
        (
            "sshfs to a case-insensitive server",
            case_insensitive_sftp_server,
        ),
    ];
    for (mount, preset) in mounts {
        for (moving, parallel) in both_ways() {
            let context = format!("{mount}, moving: {moving}, parallel: {parallel}");
            check_case_pairs_pasted(preset, moving, parallel, &context)?;
        }
    }
    Ok(())
}

/// Checks a later spelling's contents, and whether the paste looked the name up.
fn expect_later_spellings(
    destination: &Path,
    scope: &FsQuirks,
    numbered: bool,
    context: &str,
) -> Result<(), Box<dyn Error>> {
    for name in ["docs", "makefile", "project/notes"] {
        let saved_as = destination.join(if numbered {
            format!("{name} (1)")
        } else {
            name.to_owned()
        });
        assert_eq!(read_item(&saved_as, name == "docs")?, name, "{context}");
        let spelling = Path::new(name).file_name().ok_or("no name")?;
        assert_eq!(
            scope.used_name(&spelling.to_string_lossy()),
            !numbered,
            "{context}: {name}"
        );
    }
    Ok(())
}

#[test]
fn smb_without_posix_extensions_numbers_case_pairs_without_looking_up_the_later_spelling()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    // The cifs client turns server inode numbers off for the whole mount once
    // it looks a folder up under a second spelling.
    let later = ["docs", "makefile", "project/notes"];
    for (preset, numbered) in [(Quirks::cifs as Preset, true), (Quirks::cifs_posix, false)] {
        let (_root, source, destination) = source_and_destination()?;
        fs::create_dir_all(source.join("project"))?;
        write_items(
            &source,
            &[
                "Docs",
                "docs",
                "Makefile",
                "makefile",
                "project/Notes",
                "project/notes",
            ],
            "ocs",
        )?;
        let scope = FsQuirks::install(&destination, preset());

        let events = run_paste(paste_of(
            &source,
            &destination,
            &["Docs", "Makefile", "docs", "makefile", "project"],
            false,
        ));

        let context = format!("{:?}", preset().mount_type);
        let expected = if numbered {
            numbered_once(&later, RenameCause::Alias)
        } else {
            Vec::new()
        };
        assert_eq!(
            renamed_beneath(&destination, &followed_report(&events)),
            expected,
            "{context}"
        );
        expect_later_spellings(&destination, &scope, numbered, &context)?;
    }
    Ok(())
}

/// A paste of `notes.txt`, and `docs` or padding, from both `one/` and `two/`.
#[derive(Clone, Copy)]
struct IdenticalNames {
    preset: Preset,
    one_filesystem: bool,
    moving: bool,
    parallel: bool,
    prompted: bool,
}

/// The names a prompt showed for `sources`, or none where nothing prompted.
fn shown_names(
    destination: &Path,
    sources: &[Location],
    moving: bool,
    prompted: bool,
) -> Result<Vec<Option<ShownName>>, Box<dyn Error>> {
    if !prompted {
        return Ok(vec![None; sources.len()]);
    }
    Ok(glib::MainContext::default()
        .block_on(transfer_targets(
            &Location::local(destination),
            sources,
            moving,
            &gio::Cancellable::new(),
        ))?
        .into_iter()
        .map(|target| target.name)
        .collect())
}

fn paste_shown(
    destination: &Path,
    sources: Vec<Location>,
    shown: Vec<Option<ShownName>>,
    moving: bool,
) -> Vec<OperationEvent> {
    run_paste(PasteRequest {
        id: OperationRequestId(1530),
        destination: Location::local(destination),
        items: sources
            .into_iter()
            .zip(shown)
            .map(|(source, target_name)| PasteItem {
                target_name,
                ..fail_if_exists(source)
            })
            .collect(),
        move_sources: moving,
    })
}

/// Checks that one/'s item keeps its name and two/'s is numbered beside it.
fn expect_numbered_pairs(
    destination: &Path,
    numbered: &[(&str, &str)],
    context: &str,
) -> io::Result<()> {
    for (name, saved_as) in numbered {
        let folder = *name == "docs";
        let (first, second) = (destination.join(name), destination.join(saved_as));
        assert_eq!(
            read_item(&first, folder)?,
            format!("one/{name}"),
            "{context}"
        );
        assert_eq!(
            read_item(&second, folder)?,
            format!("two/{name}"),
            "{context}"
        );
    }
    Ok(())
}

fn check_identical_names(case: IdenticalNames, context: &str) -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    fs::create_dir_all(source.join("one"))?;
    fs::create_dir_all(source.join("two"))?;
    fs::create_dir(&destination)?;
    let padding = padding("one/");
    let mut items = vec!["one/notes.txt", "two/notes.txt"];
    if case.parallel {
        items.extend(padding.iter().map(String::as_str));
    } else {
        items.extend(["one/docs", "two/docs"]);
    }
    write_items(&source, &items, "docs")?;
    let mount_root = if case.one_filesystem {
        root.path()
    } else {
        destination.as_path()
    };
    let _scope = FsQuirks::install(mount_root, (case.preset)());
    let sources = items
        .iter()
        .map(|item| Location::local(source.join(item)))
        .collect::<Vec<_>>();
    let shown = shown_names(&destination, &sources, case.moving, case.prompted)?;

    let events = paste_shown(&destination, sources, shown, case.moving);

    let mut numbered = vec![("notes.txt", "notes (1).txt")];
    if !case.parallel {
        numbered.insert(0, ("docs", "docs (1)"));
    }
    let expected = numbered
        .iter()
        .map(|(name, saved_as)| (name.to_string(), saved_as.to_string(), RenameCause::Taken))
        .collect::<Vec<_>>();
    assert_eq!(
        renamed_beneath(&destination, &followed_report(&events)),
        expected,
        "{context}"
    );
    expect_numbered_pairs(&destination, &numbered, context)?;
    assert_eq!(names(&destination)?.len(), items.len(), "{context}");
    for folder in ["one", "two"] {
        assert_eq!(
            names(&source.join(folder))?.is_empty(),
            case.moving,
            "{context}"
        );
    }
    Ok(())
}

#[test]
fn identical_top_level_names_from_different_folders_are_numbered_and_reported()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let rows: [(&str, Preset, bool); 3] = [
        ("one filesystem", Quirks::ext4, true),
        ("another filesystem", Quirks::ext4, false),
        ("vfat", Quirks::vfat, false),
    ];
    // A prompt's names reach the paste as the names it showed.
    let cases = both_ways().flat_map(|case| [false, true].map(|prompted| (case, prompted)));
    for (mount, preset, one_filesystem) in rows {
        for ((moving, parallel), prompted) in cases.clone() {
            let context =
                format!("{mount}, moving: {moving}, parallel: {parallel}, prompted: {prompted}");
            let case = IdenticalNames {
                preset,
                one_filesystem,
                moving,
                parallel,
                prompted,
            };
            check_identical_names(case, &context)?;
        }
    }
    Ok(())
}
