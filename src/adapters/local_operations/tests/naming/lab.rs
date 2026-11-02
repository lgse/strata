// SPDX-License-Identifier: MIT

use rustix::{fs::Mode, io::Errno};

use super::super::super::{
    fs_ops::{Entry, fs},
    mount_profile::Profiles,
};
use super::super::fs_quirks::skips_malformed_name_on_fuse;
use super::*;
use crate::adapters::volume::gvfs_bridge_profile;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Rules {
    Posix,
    /// FAT, exFAT and ntfs3, which store device names.
    Bare,
    /// `windows_names`, which refuses them.
    Full,
    Smb,
    SmbNocase,
    SmbPosix,
    Mtp,
    /// GVfs, through libsmbclient, which maps nothing.
    GvfsSmb,
    AnyGvfs,
}

const LAB_MOUNTS: [(&str, Preset, Rules); 13] = [
    ("ext4", Quirks::ext4, Rules::Posix),
    ("nfs42", Quirks::nfs42, Rules::Posix),
    ("nfs3", Quirks::nfs3, Rules::Posix),
    ("sshfs", Quirks::sshfs, Rules::Posix),
    ("vfat", Quirks::vfat, Rules::Bare),
    ("exfat", Quirks::exfat, Rules::Bare),
    ("exfat-fuse", Quirks::exfat_fuse, Rules::Bare),
    ("ntfs3", Quirks::ntfs3, Rules::Bare),
    (
        "ntfs3 +windows_names",
        Quirks::ntfs3_windows_names,
        Rules::Full,
    ),
    ("ntfs-3g", Quirks::ntfs_3g, Rules::Full),
    ("cifs", Quirks::cifs, Rules::Smb),
    ("cifs-nocase", Quirks::cifs_nocase, Rules::SmbNocase),
    ("cifs-posix", Quirks::cifs_posix, Rules::SmbPosix),
];

struct Row {
    name: OsString,
    folder: bool,
    fixed: Vec<(&'static [Rules], String, RenameCause)>,
}

impl Row {
    fn expected(&self, rules: Rules) -> (&OsStr, Option<RenameCause>) {
        self.fixed
            .iter()
            .find(|(applies, ..)| applies.contains(&rules))
            .map_or((self.name.as_os_str(), None), |(_, name, cause)| {
                (OsStr::new(name), Some(*cause))
            })
    }
}

fn row(
    name: impl AsRef<[u8]>,
    folder: bool,
    fixed: &[(&'static [Rules], &str, RenameCause)],
) -> Row {
    Row {
        name: OsString::from_vec(name.as_ref().to_vec()),
        folder,
        fixed: fixed
            .iter()
            .map(|(rules, name, cause)| (*rules, (*name).to_owned(), *cause))
            .collect(),
    }
}

fn file(name: impl AsRef<[u8]>, fixed: &[(&'static [Rules], &str, RenameCause)]) -> Row {
    row(name, false, fixed)
}

const WINDOWS: &[Rules] = &[
    Rules::Bare,
    Rules::Full,
    Rules::Smb,
    Rules::SmbNocase,
    Rules::Mtp,
    Rules::GvfsSmb,
    Rules::AnyGvfs,
];
const FULL: &[Rules] = &[
    Rules::Full,
    Rules::Smb,
    Rules::SmbNocase,
    Rules::Mtp,
    Rules::GvfsSmb,
    Rules::AnyGvfs,
];
const MAPPED: &[Rules] = &[Rules::Smb, Rules::SmbNocase];
const UTF8: &[Rules] = &[
    Rules::Bare,
    Rules::Full,
    Rules::Smb,
    Rules::SmbNocase,
    Rules::SmbPosix,
    Rules::Mtp,
    Rules::GvfsSmb,
    Rules::AnyGvfs,
];
const ALL_BUT_MTP: &[Rules] = &[
    Rules::Posix,
    Rules::Bare,
    Rules::Full,
    Rules::Smb,
    Rules::SmbNocase,
    Rules::SmbPosix,
    Rules::GvfsSmb,
];
const BYTES: &[Rules] = &[
    Rules::Posix,
    Rules::Smb,
    Rules::SmbNocase,
    Rules::SmbPosix,
    Rules::Mtp,
    Rules::GvfsSmb,
    Rules::AnyGvfs,
];
const ASTRAL_BYTES: &[Rules] = &[
    Rules::Posix,
    Rules::Smb,
    Rules::SmbPosix,
    Rules::Mtp,
    Rules::GvfsSmb,
    Rules::AnyGvfs,
];
const UNITS: &[Rules] = &[Rules::Bare, Rules::Full];
const NOCASE: &[Rules] = &[Rules::SmbNocase];
const MTP: &[Rules] = &[Rules::Mtp, Rules::AnyGvfs];
const SAMBA_FOLDERS: &[Rules] = &[
    Rules::Smb,
    Rules::SmbNocase,
    Rules::SmbPosix,
    Rules::GvfsSmb,
    Rules::AnyGvfs,
];

fn character_rows() -> Vec<Row> {
    use RenameCause::Invalid;
    let mut rows = Vec::new();
    for forbidden in ['\\', ':', '*', '?', '"', '<', '>', '|'] {
        rows.push(file(format!("x{forbidden}y"), &[(WINDOWS, "x_y", Invalid)]));
    }
    for control in 1..0x20 {
        rows.push(file([b'x', control, b'y'], &[(WINDOWS, "x_y", Invalid)]));
    }
    rows.push(file("x\u{7f}y", &[]));
    for trailing in ["x.", "x..", "x ", "x. ", "x ."] {
        rows.push(file(trailing, &[(WINDOWS, "x", Invalid)]));
    }
    rows.extend([
        file("...", &[(WINDOWS, "_", Invalid)]),
        file(" ", &[(WINDOWS, "_", Invalid)]),
        file(" x", &[(MTP, "x", Invalid)]),
        file(" .x", &[]),
        file(".x", &[]),
    ]);
    rows
}

fn device_rows() -> Vec<Row> {
    use RenameCause::Invalid;
    vec![
        file("CON", &[(WINDOWS, "CON_", Invalid)]),
        file("con", &[(WINDOWS, "con_", Invalid)]),
        file("CON.txt", &[(FULL, "CON_.txt", Invalid)]),
        file("NUL", &[(WINDOWS, "NUL_", Invalid)]),
        file("COM1", &[(WINDOWS, "COM1_", Invalid)]),
        file("LPT9.log", &[(FULL, "LPT9_.log", Invalid)]),
        file("CONIN$", &[(WINDOWS, "CONIN$_", Invalid)]),
        file("COM\u{b9}", &[(WINDOWS, "COM\u{b9}_", Invalid)]),
        file("PRN.tar.gz", &[(FULL, "PRN_.tar.gz", Invalid)]),
        file("NUL.txt", &[(WINDOWS, "NUL_.txt", Invalid)]),
        file("aux.c", &[(FULL, "aux_.c", Invalid)]),
        file("CON .txt", &[(FULL, "CON_ .txt", Invalid)]),
        file("aux.. ..", &[(WINDOWS, "aux_", Invalid)]),
    ]
}

fn length_rows() -> Vec<Row> {
    use RenameCause::{Invalid, TooLong};
    let repeat = |text: &str, count| text.repeat(count);
    let (cjk, emoji) = ("\u{4e2d}", "\u{1f600}");
    vec![
        file(repeat("a", 255), &[(MTP, &repeat("a", 254), TooLong)]),
        file(
            repeat("a", 256),
            &[
                (MTP, &repeat("a", 254), TooLong),
                (ALL_BUT_MTP, &repeat("a", 255), TooLong),
            ],
        ),
        file(repeat(cjk, 85), &[]),
        file(repeat(cjk, 86), &[(BYTES, &repeat(cjk, 85), TooLong)]),
        file(repeat(cjk, 255), &[(BYTES, &repeat(cjk, 85), TooLong)]),
        file(
            repeat(cjk, 256),
            &[
                (BYTES, &repeat(cjk, 85), TooLong),
                (UNITS, &repeat(cjk, 255), TooLong),
            ],
        ),
        file(repeat(emoji, 63), &[(NOCASE, &repeat("_", 63), Invalid)]),
        file(
            repeat(emoji, 64),
            &[
                (ASTRAL_BYTES, &repeat(emoji, 63), TooLong),
                (NOCASE, &repeat("_", 64), Invalid),
            ],
        ),
        file(
            format!("{}a", repeat(emoji, 127)),
            &[
                (ASTRAL_BYTES, &repeat(emoji, 63), TooLong),
                (NOCASE, &format!("{}a", repeat("_", 127)), Invalid),
            ],
        ),
        file(
            repeat(emoji, 128),
            &[
                (ASTRAL_BYTES, &repeat(emoji, 63), TooLong),
                (NOCASE, &repeat("_", 128), Invalid),
                (UNITS, &repeat(emoji, 127), TooLong),
            ],
        ),
    ]
}

fn encoding_rows() -> Vec<Row> {
    use RenameCause::{Invalid, TooLong};
    let repeat = |text: &str, count| text.repeat(count);
    vec![
        file(b"x\xffy", &[(UTF8, "x_y", Invalid)]),
        file(b"x\x80y", &[(UTF8, "x_y", Invalid)]),
        file(b"x\xc0\xafy", &[(UTF8, "x__y", Invalid)]),
        file(b"x\xed\xa0\x80y", &[(UTF8, "x___y", Invalid)]),
        file(b"x\xe4\xb8y", &[(UTF8, "x_y", Invalid)]),
        file("caf\u{e9}", &[]),
        file("cafe\u{301}", &[]),
        file("x\u{f022}y", &[(MAPPED, "x_y", Invalid)]),
        file("x\u{f001}y", &[(MAPPED, "x_y", Invalid)]),
        file("x\u{f029}y", &[(MAPPED, "x_y", Invalid)]),
        file("x\u{f026}y", &[]),
        file("x\u{f03a}y", &[]),
        file("x\u{ff1a}y", &[]),
        file("a\u{1f600}b", &[(NOCASE, "a_b", Invalid)]),
        file("\u{1d400}", &[(NOCASE, "_", Invalid)]),
        row(
            repeat("a", 255),
            true,
            &[
                (SAMBA_FOLDERS, &repeat("a", 213), TooLong),
                (MTP, &repeat("a", 254), TooLong),
            ],
        ),
        row(
            format!("{}.txt", repeat("x", 250)),
            true,
            &[(SAMBA_FOLDERS, &repeat("x", 213), TooLong)],
        ),
        file(format!("{}.txt", repeat("x", 250)), &[]),
    ]
}

/// Every name the fslab mounts were probed with, and more device names.
fn lab_corpus() -> Vec<Row> {
    let mut rows = character_rows();
    rows.extend(device_rows());
    rows.extend(length_rows());
    rows.extend(encoding_rows());
    rows
}

fn lab_profile(quirk_root: &Path) -> Result<NameProfile, Box<dyn Error>> {
    Ok(Profiles::default()
        .of(fs(), open_local_parent_directory(quirk_root)?.as_fd())
        .names)
}

fn listed_names(folder: &Path) -> Result<Vec<OsString>, Box<dyn Error>> {
    Ok(fs::read_dir(folder)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<Result<Vec<_>, _>>()?)
}

/// The row's name fixed for `profile` as its rules say.
fn fixed_as_expected(row: &Row, profile: &NameProfile, rules: Rules, context: &str) -> OsString {
    let (name, cause) = fixed(&row.name, profile, row.folder);
    assert_eq!((name.as_os_str(), cause), row.expected(rules), "{context}");
    name
}

/// The mount stores `name` in `folder` and lists it back unchanged.
fn assert_stored_as_is(
    folder: &Path,
    name: &OsStr,
    is_folder: bool,
    context: &str,
) -> Result<(), Box<dyn Error>> {
    let parent = open_local_parent_directory(folder)?;
    let at = Entry::new(&parent, name);
    let created = if is_folder {
        fs().mkdir(at, Mode::from_raw_mode(0o755))
    } else {
        fs().create_exclusive(at, Mode::from_raw_mode(0o644))
            .map(drop)
    };
    assert_eq!(created, Ok(()), "{context}");
    assert_eq!(listed_names(folder)?, [name.to_owned()], "{context}");
    Ok(())
}

fn assert_mount_stores_fixed_names(
    quirk_root: &Path,
    mount: &str,
    rules: Rules,
    corpus: &[Row],
) -> Result<(), Box<dyn Error>> {
    let profile = lab_profile(quirk_root)?;
    for (case, row) in corpus.iter().enumerate() {
        let context = format!("{mount}: {:?}", row.name);
        let stored = fixed_as_expected(row, &profile, rules, &context);
        assert_eq!(fixed(&stored, &profile, row.folder).0, stored, "{context}");
        // The root's own filesystem stops at 255 bytes, which UTF-16 mounts pass.
        if stored.len() > 255 || skips_malformed_name_on_fuse(quirk_root, &stored) {
            continue;
        }
        let folder = quirk_root.join(case.to_string());
        fs::create_dir(&folder)?;
        assert_stored_as_is(&folder, &stored, row.folder, &context)?;
    }
    Ok(())
}

#[test]
fn each_mount_gets_a_name_it_stores_as_is_for_every_lab_name() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let corpus = lab_corpus();
    for (index, (mount, preset, rules)) in LAB_MOUNTS.into_iter().enumerate() {
        let quirk_root = root.path().join(index.to_string());
        fs::create_dir(&quirk_root)?;
        let _quirks = FsQuirks::install(&quirk_root, preset());
        assert_mount_stores_fixed_names(&quirk_root, mount, rules, &corpus)?;
    }
    for (backend, profile, rules) in [
        ("mtp", gvfs_profile("mtp"), Rules::Mtp),
        ("smb", gvfs_profile("smb"), Rules::GvfsSmb),
        ("sftp", gvfs_profile("sftp"), Rules::Posix),
        ("unknown", gvfs_bridge_profile(None), Rules::AnyGvfs),
    ] {
        for row in &corpus {
            let context = format!("{backend}: {:?}", row.name);
            fixed_as_expected(row, &profile, rules, &context);
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Lab {
    Stored,
    Listed(&'static str),
    /// Listed under a Samba 8.3 alias.
    Mangled,
    Refused(Errno),
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    File,
    Folder,
    Symlink,
}

/// A name's bytes, what it names, and the lab's cell for each mount.
type Cells = (Vec<u8>, Kind, [Lab; 13]);

/// Columns of the lab, in the order of [`LAB_MOUNTS`].
const SSHFS: usize = 3;
const VFAT: usize = 4;
const EXFAT: usize = 5;
const EXFAT_FUSE: usize = 6;
const NTFS3: usize = 7;
const NTFS3_WINDOWS_NAMES: usize = 8;
const NTFS_3G: usize = 9;
const CIFS: usize = 10;
const CIFS_NOCASE: usize = 11;
const CIFS_POSIX: usize = 12;

const STORED: [Lab; 13] = [Lab::Stored; 13];
const INVAL: Lab = Lab::Refused(Errno::INVAL);
const NOENT: Lab = Lab::Refused(Errno::NOENT);
const ILSEQ: Lab = Lab::Refused(Errno::ILSEQ);

fn with(base: [Lab; 13], cells: &[(usize, Lab)]) -> [Lab; 13] {
    let mut row = base;
    for (column, cell) in cells {
        row[*column] = *cell;
    }
    row
}

/// Mounts with Windows rules refuse the name outright.
fn windows_cells() -> [Lab; 13] {
    with(
        STORED,
        &[
            (VFAT, INVAL),
            (EXFAT, INVAL),
            (EXFAT_FUSE, NOENT),
            (NTFS3_WINDOWS_NAMES, INVAL),
            (NTFS_3G, INVAL),
        ],
    )
}

/// Only `windows_names` and ntfs-3g refuse the name.
fn unreserved_cells() -> [Lab; 13] {
    with(STORED, &[(NTFS3_WINDOWS_NAMES, INVAL), (NTFS_3G, INVAL)])
}

fn character_cells() -> Vec<Cells> {
    use Kind::File;
    use Lab::Listed;
    let windows = windows_cells();
    let mut rows = ":*?\"<>|"
        .chars()
        .map(|forbidden| (format!("x{forbidden}y").into_bytes(), File, windows))
        .collect::<Vec<_>>();
    rows.extend([
        (
            b"x\\y".to_vec(),
            File,
            with(windows, &[(CIFS, INVAL), (CIFS_NOCASE, INVAL)]),
        ),
        (b"x\x01y".to_vec(), File, windows),
        (b"x\x1fy".to_vec(), File, windows),
        (b"x\x7fy".to_vec(), File, STORED),
        (b" x".to_vec(), File, STORED),
        (b".x".to_vec(), File, STORED),
        (
            b"x.".to_vec(),
            File,
            with(
                unreserved_cells(),
                &[(VFAT, Listed("x")), (EXFAT, Listed("x"))],
            ),
        ),
        (
            b"x ".to_vec(),
            File,
            with(unreserved_cells(), &[(VFAT, INVAL)]),
        ),
        (
            b"x .".to_vec(),
            File,
            with(unreserved_cells(), &[(VFAT, INVAL), (EXFAT, Listed("x "))]),
        ),
        (
            b"...".to_vec(),
            File,
            with(unreserved_cells(), &[(VFAT, NOENT), (EXFAT, NOENT)]),
        ),
    ]);
    rows
}

fn reserved_cells() -> Vec<Cells> {
    use Kind::File;
    let reserved = with(
        unreserved_cells(),
        &[(CIFS, Lab::Mangled), (CIFS_NOCASE, Lab::Mangled)],
    );
    vec![
        (b"CON".to_vec(), File, reserved),
        (b"con".to_vec(), File, reserved),
        (b"CON.txt".to_vec(), File, reserved),
        (b"NUL".to_vec(), File, reserved),
        (b"COM1".to_vec(), File, reserved),
        (b"PRN.tar.gz".to_vec(), File, reserved),
        (b"LPT9.log".to_vec(), File, unreserved_cells()),
        (b"CONIN$".to_vec(), File, STORED),
        ("COM\u{b9}".as_bytes().to_vec(), File, STORED),
    ]
}

/// exfat-fuse stored overlong and surrogate names and then failed; its
/// preset refuses them with `EUCLEAN`.
fn malformed(fuse: Lab, ntfs_3g: Lab, posix: &'static str) -> [Lab; 13] {
    with(
        STORED,
        &[
            (VFAT, INVAL),
            (EXFAT, ILSEQ),
            (EXFAT_FUSE, fuse),
            (NTFS3, INVAL),
            (NTFS3_WINDOWS_NAMES, INVAL),
            (NTFS_3G, ntfs_3g),
            (CIFS, NOENT),
            (CIFS_NOCASE, INVAL),
            (CIFS_POSIX, Lab::Listed(posix)),
        ],
    )
}

fn encoding_cells() -> Vec<Cells> {
    use Kind::File;
    use Lab::Listed;
    let corrupts = Lab::Refused(Errno::UCLEAN);
    vec![
        (b"x\xffy".to_vec(), File, malformed(ILSEQ, ILSEQ, "x?y")),
        (b"x\x80y".to_vec(), File, malformed(ILSEQ, ILSEQ, "x?y")),
        (
            b"x\xc0\xafy".to_vec(),
            File,
            malformed(corrupts, ILSEQ, "x??y"),
        ),
        (
            b"x\xed\xa0\x80y".to_vec(),
            File,
            malformed(corrupts, Lab::Stored, "x???y"),
        ),
        ("cafe\u{301}".as_bytes().to_vec(), File, STORED),
        (
            "x\u{f022}y".as_bytes().to_vec(),
            File,
            with(
                STORED,
                &[(CIFS, Listed("x:y")), (CIFS_NOCASE, Listed("x:y"))],
            ),
        ),
        ("x\u{f03a}y".as_bytes().to_vec(), File, STORED),
        ("x\u{ff1a}y".as_bytes().to_vec(), File, STORED),
        (
            "\u{1f600}".repeat(63).into_bytes(),
            File,
            with(STORED, &[(CIFS_NOCASE, INVAL)]),
        ),
    ]
}

fn length_and_link_cells() -> Vec<Cells> {
    use Kind::{File, Folder, Symlink};
    let refused = |errno| Lab::Refused(errno);
    vec![
        (vec![b'a'; 255], File, STORED),
        (vec![b'a'; 217], Folder, STORED),
        (
            vec![b'a'; 218],
            Folder,
            with(
                STORED,
                &[(CIFS, NOENT), (CIFS_NOCASE, NOENT), (CIFS_POSIX, NOENT)],
            ),
        ),
        (
            vec![b'a'; 256],
            File,
            with(
                [refused(Errno::NAMETOOLONG); 13],
                &[(SSHFS, refused(Errno::BADMSG))],
            ),
        ),
        (
            b"s".to_vec(),
            Symlink,
            with(
                STORED,
                &[
                    (VFAT, refused(Errno::PERM)),
                    (EXFAT, refused(Errno::PERM)),
                    (EXFAT_FUSE, refused(Errno::NOSYS)),
                    (CIFS, refused(Errno::OPNOTSUPP)),
                    (CIFS_NOCASE, refused(Errno::OPNOTSUPP)),
                ],
            ),
        ),
    ]
}

/// The lab's cells, in the order of [`LAB_MOUNTS`].
fn lab_cells() -> Vec<Cells> {
    let mut rows = character_cells();
    rows.extend(reserved_cells());
    rows.extend(encoding_cells());
    rows.extend(length_and_link_cells());
    rows
}

fn create_lab_entry(
    folder: &Path,
    name: &OsStr,
    kind: Kind,
) -> Result<Result<(), Errno>, Box<dyn Error>> {
    let parent = open_local_parent_directory(folder)?;
    let at = Entry::new(&parent, name);
    Ok(match kind {
        Kind::File => fs()
            .create_exclusive(at, Mode::from_raw_mode(0o644))
            .map(drop),
        Kind::Folder => fs().mkdir(at, Mode::from_raw_mode(0o755)),
        Kind::Symlink => fs().symlink(OsStr::new("t"), at),
    })
}

/// A name the lab probed on one mount, with the cell it should match.
struct Probe<'a> {
    name: &'a OsStr,
    cell: Lab,
    context: String,
}

impl Probe<'_> {
    /// What the preset did with the name.
    fn outcome(&self, created: Result<(), Errno>, listed: &[OsString]) -> Lab {
        let context = &self.context;
        match (created, listed) {
            (Err(error), []) => Lab::Refused(error),
            (Ok(()), [listed]) if listed == self.name => Lab::Stored,
            (Ok(()), [listed]) if self.cell == Lab::Mangled => {
                assert!(listed.as_encoded_bytes().contains(&b'~'), "{context}");
                Lab::Mangled
            }
            (Ok(()), [listed]) => match self.cell {
                Lab::Listed(expected) if listed == expected => self.cell,
                _ => panic!("{context}: listed as {listed:?}"),
            },
            (created, listed) => panic!("{context}: {created:?}, listed {listed:?}"),
        }
    }
}

fn assert_preset_matches_lab_column(
    quirk_root: &Path,
    mount: &str,
    column: usize,
    rows: &[Cells],
) -> Result<(), Box<dyn Error>> {
    for (case, (name, kind, cells)) in rows.iter().enumerate() {
        let name = std::os::unix::ffi::OsStrExt::from_bytes(name.as_slice());
        if skips_malformed_name_on_fuse(quirk_root, name) {
            continue;
        }
        let probe = Probe {
            name,
            cell: cells[column],
            context: format!("{mount}: {name:?}"),
        };
        let folder = quirk_root.join(case.to_string());
        fs::create_dir(&folder)?;
        let created = create_lab_entry(&folder, name, *kind)?;
        let listed = listed_names(&folder)?;
        assert_eq!(
            probe.outcome(created, &listed),
            probe.cell,
            "{}",
            probe.context
        );
    }
    Ok(())
}

#[test]
fn each_preset_does_with_a_raw_name_what_its_lab_mount_did() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let rows = lab_cells();
    for (column, (mount, preset, _)) in LAB_MOUNTS.into_iter().enumerate() {
        let quirk_root = root.path().join(column.to_string());
        fs::create_dir(&quirk_root)?;
        let _quirks = FsQuirks::install(&quirk_root, preset());
        assert_preset_matches_lab_column(&quirk_root, mount, column, &rows)?;
    }
    Ok(())
}
