// SPDX-License-Identifier: MIT

use super::*;
use crate::adapters::volume::learn_mount_capability;
use fs_quirks::Op;
use std::os::unix::fs::FileTypeExt;

/// A report to paste beside one the destination already holds.
fn report_beside_existing(root: &Path) -> io::Result<(PathBuf, PathBuf)> {
    let source_dir = root.join("source");
    let source = source_dir.join("report.txt");
    let destination = root.join("dest");
    fs::create_dir_all(&source_dir)?;
    fs::create_dir_all(&destination)?;
    fs::write(&source, b"incoming")?;
    fs::write(destination.join("report.txt"), b"existing")?;
    Ok((source, destination))
}

#[test]
fn keeping_both_in_a_cross_folder_paste_generates_a_unique_name() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new()?;
    let (source, destination) = report_beside_existing(scratch.path())?;

    let created = run_paste_collecting_created(paste_request(
        74,
        &destination,
        vec![item(&source, TransferConflict::KeepBoth)],
        false,
    ))?;

    assert_eq!(
        created.into_iter().flatten().collect::<Vec<_>>(),
        vec![Location::local(destination.join("report (1).txt"))]
    );
    expect_contents(&[
        (destination.join("report.txt"), b"existing"),
        (destination.join("report (1).txt"), b"incoming"),
    ])?;
    assert!(source.exists());
    Ok(())
}

#[test]
fn keeping_both_while_moving_renames_the_destination_instead_of_replacing_it()
-> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new()?;
    let (source, destination) = report_beside_existing(scratch.path())?;

    let events = run_paste(paste_request(
        75,
        &destination,
        vec![item(&source, TransferConflict::KeepBoth)],
        true,
    ));

    assert_eq!(
        item_moves(&events),
        [(
            Location::local(&source),
            Location::local(destination.join("report (1).txt"))
        )]
    );
    expect_contents(&[
        (destination.join("report.txt"), b"existing"),
        (destination.join("report (1).txt"), b"incoming"),
    ])?;
    assert!(!source.exists());
    Ok(())
}

/// A long name, whether it names a folder, and the numbered name it gets.
struct LongName {
    name: String,
    folder: bool,
    numbered: String,
}

impl LongName {
    /// Puts the item under `sources` and its namesake under `destination`,
    /// giving the file that must hold the incoming contents afterwards.
    fn seed(&self, sources: &Path, destination: &Path) -> io::Result<PathBuf> {
        let source = sources.join(&self.name);
        fs::create_dir(sources)?;
        fs::create_dir(destination)?;
        if self.folder {
            fs::create_dir(&source)?;
            fs::create_dir(destination.join(&self.name))?;
            fs::write(source.join("inside.txt"), b"incoming")?;
            Ok(destination.join(&self.numbered).join("inside.txt"))
        } else {
            fs::write(&source, b"incoming")?;
            fs::write(destination.join(&self.name), b"existing")?;
            Ok(destination.join(&self.numbered))
        }
    }
}

fn long_names() -> Vec<LongName> {
    let repeat = |text: &str, count| text.repeat(count);
    vec![
        LongName {
            name: format!("{}.txt", repeat("a", 251)),
            folder: false,
            numbered: format!("{} (1).txt", repeat("a", 247)),
        },
        LongName {
            name: repeat("d", 254),
            folder: true,
            numbered: format!("{} (1)", repeat("d", 251)),
        },
        LongName {
            name: format!("{}.tar.gz", repeat("a", 247)),
            folder: false,
            numbered: format!("{}.tar (1).gz", repeat("a", 244)),
        },
    ]
}

fn assert_keep_both_numbers(
    mount: &str,
    preset: Option<Preset>,
    moving: bool,
    row: &LongName,
) -> Result<(), Box<dyn Error>> {
    let context = format!("{mount}, moving: {moving}, {}", row.name);
    let root = tempfile::tempdir()?;
    let _quirks = preset.map(|preset| FsQuirks::install(root.path(), preset()));
    let sources = root.path().join("sources");
    let source = sources.join(&row.name);
    let destination = root.path().join("destination");
    let incoming = row.seed(&sources, &destination)?;

    run_paste(paste_request(
        78,
        &destination,
        vec![item(&source, TransferConflict::KeepBoth)],
        moving,
    ));

    let mut expected = vec![row.name.clone(), row.numbered.clone()];
    expected.sort();
    assert_eq!(names(&destination)?, expected, "{context}");
    assert_eq!(fs::read(incoming)?, b"incoming", "{context}");
    assert_eq!(source.exists(), !moving, "{context}");
    Ok(())
}

#[test]
fn keeping_both_numbers_a_long_name_within_the_limit_of_its_mount() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let rows = long_names();
    let samba_folder = vec![LongName {
        name: "d".repeat(212),
        folder: true,
        numbered: format!("{} (1)", "d".repeat(209)),
    }];
    let ntfs_3g = || Quirks {
        rename_flags: fs_quirks::Support::Rejected(rustix::io::Errno::INVAL),
        ..Quirks::ntfs_3g()
    };
    let mounts: [(&str, Option<Preset>, _); 4] = [
        ("host", None, &rows),
        ("vfat", Some(Quirks::vfat), &rows),
        ("cifs", Some(Quirks::cifs), &samba_folder),
        ("ntfs-3g", Some(ntfs_3g), &rows),
    ];
    for (mount, preset, rows) in mounts {
        for moving in [false, true] {
            for row in rows {
                assert_keep_both_numbers(mount, preset, moving, row)?;
            }
        }
    }
    Ok(())
}

fn taking(path: &Path) -> fs_quirks::Racer {
    let path = path.to_path_buf();
    Box::new(move || {
        if fs::symlink_metadata(&path).is_err() {
            fs::write(&path, b"taken meanwhile").expect("take the name");
        }
    })
}

#[test]
fn keep_both_copy_renumbers_when_its_name_is_taken_meanwhile() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let unnamed_files_refused: Preset = || Quirks {
        tmpfile: fs_quirks::Support::Rejected(rustix::io::Errno::OPNOTSUPP),
        ..Quirks::ext4()
    };
    let rows: [(&str, &str, Preset); 3] = [
        ("report.txt", "report (1).txt", Quirks::default),
        ("report.txt", "report (1).txt", unnamed_files_refused),
        ("docs", "docs (1)", Quirks::default),
    ];
    for (name, first, quirks) in rows {
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        fs::create_dir_all(&sources)?;
        fs::create_dir_all(&destination)?;
        let source = sources.join(name);
        if name == "docs" {
            fs::create_dir(&source)?;
            fs::write(source.join("inside.txt"), b"incoming")?;
            fs::create_dir(destination.join(name))?;
        } else {
            fs::write(&source, b"incoming")?;
            fs::write(destination.join(name), b"existing")?;
        }
        let taken = destination.join(first);
        let _scope = FsQuirks::install(
            root.path(),
            Quirks {
                before: vec![
                    (Op::LinkFd, first, 0, taking(&taken)),
                    (Op::Rename, first, 0, taking(&taken)),
                ],
                ..quirks()
            },
        );

        let created = run_paste_collecting_created(paste_request(
            77,
            &destination,
            vec![item(&source, TransferConflict::KeepBoth)],
            false,
        ))?;

        let second = first.replace("(1)", "(2)");
        assert_eq!(
            created.into_iter().flatten().collect::<Vec<_>>(),
            [Location::local(destination.join(&second))],
            "{name}"
        );
        assert_eq!(fs::read(&taken)?, b"taken meanwhile", "{name}");
        let copied = destination.join(&second);
        let copied = if name == "docs" {
            copied.join("inside.txt")
        } else {
            copied
        };
        assert_eq!(fs::read(copied)?, b"incoming", "{name}");
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    File,
    Folder,
    Link,
    Pipe,
}

impl Kind {
    fn create(self, source: &Path) -> io::Result<()> {
        match self {
            Kind::File => fs::write(source, b"incoming"),
            Kind::Folder => {
                fs::create_dir(source)?;
                fs::write(source.join("inside.txt"), b"incoming")
            }
            Kind::Link => std::os::unix::fs::symlink("incoming", source),
            Kind::Pipe => Ok(rustix::fs::mknodat(
                rustix::fs::CWD,
                source,
                rustix::fs::FileType::Fifo,
                rustix::fs::Mode::from_raw_mode(0o600),
                0,
            )?),
        }
    }

    /// The item landed at `copied` as what it was.
    fn expect_landed(self, copied: &Path, row: &str) -> Result<(), Box<dyn Error>> {
        match self {
            Kind::File => assert_eq!(fs::read(copied)?, b"incoming", "{row}"),
            Kind::Folder => assert_eq!(fs::read(copied.join("inside.txt"))?, b"incoming", "{row}"),
            Kind::Link => assert_eq!(fs::read_link(copied)?, Path::new("incoming"), "{row}"),
            Kind::Pipe => assert!(fs::symlink_metadata(copied)?.file_type().is_fifo(), "{row}"),
        }
        Ok(())
    }
}

/// A move renames within one volume, so its folder is numbered as one;
/// copies cross volumes, and a folder is made in place where the mount
/// is known to place none.
#[derive(Clone, Copy, PartialEq)]
enum Way {
    Moved,
    Copied,
    InPlace,
}

/// An item pasted where `taken` names exist; where a stage cannot be
/// placed, it is copied out to a name another program takes first.
struct Numbered {
    name: &'static str,
    kind: Kind,
    way: Way,
    preset: Preset,
    taken: &'static [&'static str],
    racer: Option<(Op, &'static str, usize)>,
    expected: &'static str,
}

fn numbered_rows() -> [Numbered; 7] {
    let row = |name, kind, way, preset, taken, racer, expected| Numbered {
        name,
        kind,
        way,
        preset,
        taken,
        racer,
        expected,
    };
    [
        row(
            "my.photos",
            Kind::Folder,
            Way::Moved,
            Quirks::default,
            &["my.photos"],
            None,
            "my.photos (1)",
        ),
        row(
            "x (2).txt",
            Kind::File,
            Way::Copied,
            Quirks::default,
            &["x (2).txt"],
            None,
            "x (3).txt",
        ),
        row(
            "link",
            Kind::Link,
            Way::Copied,
            Quirks::default,
            &["link", "link (1)"],
            None,
            "link (2)",
        ),
        row(
            "pipe",
            Kind::Pipe,
            Way::Copied,
            Quirks::default,
            &["pipe", "pipe (1)"],
            None,
            "pipe (2)",
        ),
        row(
            "docs",
            Kind::Folder,
            Way::InPlace,
            Quirks::without_folder_placement,
            &["docs", "docs (1)"],
            None,
            "docs (2)",
        ),
        row(
            "docs",
            Kind::Folder,
            Way::Copied,
            Quirks::without_placement,
            &["docs"],
            Some((Op::Mkdir, "docs (1)", 1)),
            "docs (2)",
        ),
        row(
            "a.txt",
            Kind::File,
            Way::Copied,
            Quirks::without_placement,
            &["a.txt"],
            Some((Op::Create, "a (1).txt", 0)),
            "a (2).txt",
        ),
    ]
}

impl Numbered {
    /// Seeds the taken names and gives them, with the one the racer takes.
    fn seed_taken(&self, destination: &Path) -> io::Result<(Vec<&'static str>, Quirks)> {
        let mut existing = self.taken.to_vec();
        for name in self.taken {
            fs::write(destination.join(name), b"existing")?;
        }
        let mut quirks = (self.preset)();
        if let Some((op, first, skip)) = self.racer {
            existing.push(first);
            quirks.before = vec![(op, first, skip, taking(&destination.join(first)))];
        }
        Ok((existing, quirks))
    }

    /// Installs the quirks over the root a move renames within, or over the
    /// destination a copy crosses into from its own volume.
    fn install(&self, root: &Path, quirks: Quirks) -> (Option<FsQuirks>, FsQuirks) {
        let moving = self.way == Way::Moved;
        let sources =
            (!moving).then(|| FsQuirks::install(&root.join("sources"), Quirks::default()));
        let destination = root.join("destination");
        let scope = FsQuirks::install(if moving { root } else { &destination }, quirks);
        if self.way == Way::InPlace {
            learn_mount_capability(scope.key(), |capabilities| {
                capabilities.no_replace = Some(false);
                capabilities.hard_links = Some(true);
                capabilities.rename_over_empty_directory = Some(false);
            });
        }
        (sources, scope)
    }
}

#[test]
fn keep_both_takes_the_next_number_past_names_already_taken() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    for numbered in numbered_rows() {
        let row = format!("{} -> {}", numbered.name, numbered.expected);
        let root = tempfile::tempdir()?;
        let sources = root.path().join("sources");
        let destination = root.path().join("destination");
        fs::create_dir_all(&sources)?;
        fs::create_dir_all(&destination)?;
        let source = sources.join(numbered.name);
        numbered.kind.create(&source)?;
        let (mut existing, quirks) = numbered.seed_taken(&destination)?;
        let _scopes = numbered.install(root.path(), quirks);

        run_paste(paste_request(
            79,
            &destination,
            vec![item(&source, TransferConflict::KeepBoth)],
            numbered.way == Way::Moved,
        ));

        existing.push(numbered.expected);
        existing.sort_unstable();
        assert_eq!(names(&destination)?, existing, "{row}");
        numbered
            .kind
            .expect_landed(&destination.join(numbered.expected), &row)?;
    }
    Ok(())
}
