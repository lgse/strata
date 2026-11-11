// SPDX-License-Identifier: MIT

use super::super::super::names::fix::refusal;
use super::*;

const HANGUL: &str = "\u{1112}\u{1161}\u{11ab}";
const FLAG: &str = "\u{1f1eb}\u{1f1f7}";
const CONJUNCT: &str = "\u{915}\u{94d}\u{937}";
const SCOTLAND: &str = "\u{1f3f4}\u{e0067}\u{e0062}\u{e0073}\u{e0063}\u{e0074}\u{e007f}";
const FAMILY: &str = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{200d}\u{1f466}";
const HEART: &str = "\u{2764}\u{fe0f}";

fn posix_bytes() -> NameProfile {
    NameProfile::posix(Some(255))
}

fn bare_units() -> NameProfile {
    NameProfile::windows(Reserved::Bare, NameEncoding::Utf8)
}

/// A name, the profile that cuts it, whether it names a folder, and the cut.
type Cut = (String, NameProfile, bool, String);

/// Cuts that end on a grapheme cluster boundary.
fn cluster_cuts() -> Vec<Cut> {
    let (bytes, units) = (posix_bytes(), bare_units());
    let repeat = |text: &str, count| text.repeat(count);
    vec![
        (
            format!("abx{}.txt", repeat("\u{301}", 600)),
            units,
            false,
            format!("abx{}.txt", repeat("\u{301}", 248)),
        ),
        (
            format!("{}.txt", repeat(HANGUL, 40)),
            bytes,
            false,
            format!("{}.txt", repeat(HANGUL, 27)),
        ),
        (repeat(FLAG, 40), bytes, false, repeat(FLAG, 31)),
        (
            format!("abcd{}", repeat(CONJUNCT, 40)),
            bytes,
            false,
            format!("abcd{}", repeat(CONJUNCT, 27)),
        ),
        (repeat(SCOTLAND, 20), units, false, repeat(SCOTLAND, 18)),
        (
            format!("{}{}", repeat("a", 20), repeat(FAMILY, 12)),
            bytes,
            false,
            format!("{}{}", repeat("a", 20), repeat(FAMILY, 9)),
        ),
        (
            format!("aaaaaaa{}", repeat(SCOTLAND, 9)),
            bytes,
            false,
            format!("aaaaaaa{}", repeat(SCOTLAND, 8)),
        ),
        (
            format!("{}x{}", repeat("a", 230), repeat("\u{301}", 20)),
            bytes,
            false,
            format!("{}x{}", repeat("a", 230), repeat("\u{301}", 12)),
        ),
        (repeat(HEART, 50), bytes, false, repeat(HEART, 42)),
    ]
}

/// Cuts that keep the extension, or the device mark, of the name.
fn extension_cuts() -> Vec<Cut> {
    let (bytes, units) = (posix_bytes(), bare_units());
    let full = NameProfile::windows(Reserved::Full, NameEncoding::Utf8);
    let narrow = NameProfile::posix(Some(64));
    let repeat = |text: &str, count| text.repeat(count);
    vec![
        (
            format!("{}.tar.gz", repeat("a", 300)),
            bytes,
            false,
            format!("{}.tar.gz", repeat("a", 248)),
        ),
        (
            format!("x.{}", repeat("a", 300)),
            bytes,
            false,
            format!("x.{}", repeat("a", 253)),
        ),
        (
            format!("{}.txt", repeat("a", 300)),
            bytes,
            true,
            repeat("a", 255),
        ),
        (
            format!("\u{1f3f4}{}.{}", repeat("\u{e0067}", 12), repeat("a", 31)),
            narrow,
            false,
            format!("\u{1f3f4}{}.{}", repeat("\u{e0067}", 7), repeat("a", 31)),
        ),
        (
            format!("CON{}x", repeat(" ", 300)),
            units,
            false,
            "CON_".to_owned(),
        ),
        (
            format!("CON{}x.txt", repeat(" ", 248)),
            full,
            false,
            format!("CON_{}.txt", repeat(" ", 247)),
        ),
    ]
}

#[test]
fn cut_names_keep_their_extension_and_whole_clusters() {
    // Pango keeps consonants joined by a virama together from 1.54 on.
    let joins_conjuncts = pango::version_check(1, 54, 0).is_none();
    for (name, profile, folder, expected) in cluster_cuts().into_iter().chain(extension_cuts()) {
        if name.contains(CONJUNCT) && !joins_conjuncts {
            continue;
        }
        let (cut, cause) = fixed(OsStr::new(&name), &profile, folder);
        assert_eq!(
            (cut.as_os_str(), cause),
            (OsStr::new(&expected), Some(RenameCause::TooLong)),
            "{name:?}"
        );
        assert_eq!(fixed(&cut, &profile, folder).0, cut, "{name:?}");
    }
}

#[test]
fn device_names_get_a_mark_where_windows_reserves_them() {
    let bare = NameProfile::windows(Reserved::Bare, NameEncoding::Utf8);
    let full = NameProfile::windows(Reserved::Full, NameEncoding::Utf8);
    let ports = [
        "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "\u{b9}", "\u{b2}", "\u{b3}",
    ];
    let devices = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"]
        .map(String::from)
        .into_iter()
        .chain(
            ["COM", "LPT"]
                .into_iter()
                .flat_map(|port| ports.map(|number| format!("{port}{number}"))),
        );
    for device in devices {
        for word in [device.clone(), device.to_lowercase()] {
            let dotted = format!("{word}.txt");
            let marked = format!("{word}_.txt");
            let bare_dotted = if device == "NUL" { &marked } else { &dotted };
            for (profile, name, expected) in [
                (bare, &word, &format!("{word}_")),
                (bare, &dotted, bare_dotted),
                (full, &dotted, &marked),
            ] {
                let fixed = fix_name(OsStr::new(name), &profile, false).name;
                assert_eq!(fixed, OsStr::new(expected), "{name} {:?}", profile.reserved);
            }
        }
    }
    for name in ["COM10", "LPT", "CONOUT", "AUXX", "NULL.txt", "CON_"] {
        assert_eq!(
            fix_name(OsStr::new(name), &full, false).cause,
            None,
            "{name}"
        );
    }
}

#[test]
fn only_the_characters_an_smb_mapping_reads_back_are_replaced() {
    for (mapped, replaced, kept) in [
        (
            PrivateUse::Sfm,
            &[
                '\u{f001}', '\u{f01f}', '\u{f020}', '\u{f022}', '\u{f025}', '\u{f027}', '\u{f029}',
            ][..],
            &['\u{f000}', '\u{f026}', '\u{f02a}', '\u{f03a}'][..],
        ),
        (
            PrivateUse::Sfu,
            &[
                '\u{f02a}', '\u{f03a}', '\u{f03c}', '\u{f03e}', '\u{f03f}', '\u{f07c}',
            ][..],
            &['\u{f001}', '\u{f022}', '\u{f029}', '\u{f05c}'][..],
        ),
    ] {
        let profile = NameProfile {
            mapped: Some(mapped),
            ..NameProfile::smb(Some(255))
        };
        for (characters, changed) in [(replaced, true), (kept, false)] {
            for character in characters {
                let name = format!("x{character}y");
                let fixed = fix_name(OsStr::new(&name), &profile, false).name;
                let expected = if changed { "x_y" } else { name.as_str() };
                assert_eq!(fixed, OsStr::new(expected), "{mapped:?} {name:?}");
            }
        }
    }
}

const STARTS: [&[u8]; 6] = [b"a", b" ", b"CON", b"nul", b"\xff", "\u{4e2d}".as_bytes()];
const RUNS: [&[u8]; 14] = [
    b"a",
    b" ",
    b".",
    b":",
    b"\x01",
    b"\xed\xa0",
    "\u{4e2d}".as_bytes(),
    "\u{1f600}".as_bytes(),
    "\u{301}".as_bytes(),
    "\u{200d}".as_bytes(),
    "\u{1f1eb}".as_bytes(),
    "\u{f022}".as_bytes(),
    "\u{915}\u{94d}".as_bytes(),
    b"CON",
];
const ENDS: [&[u8]; 5] = [b"", b".txt", b".tar.gz", b" .", "\u{301}.c".as_bytes()];

/// Draws names from a start, up to four runs of a repeated piece, and an end.
struct NameSampler {
    state: u32,
}

impl NameSampler {
    fn new() -> Self {
        Self { state: 0x9e37_79b9 }
    }

    fn below(&mut self, limit: usize) -> usize {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 17;
        self.state ^= self.state << 5;
        self.state as usize % limit
    }

    /// The next name and whether it names a folder.
    fn next(&mut self) -> (OsString, bool) {
        let mut name = STARTS[self.below(STARTS.len())].to_vec();
        for _ in 0..=self.below(3) {
            let run = RUNS[self.below(RUNS.len())];
            for _ in 0..=self.below(120) {
                name.extend_from_slice(run);
            }
        }
        name.extend_from_slice(ENDS[self.below(ENDS.len())]);
        (OsString::from_vec(name), self.below(2) == 0)
    }
}

fn fixed_point_profiles() -> [NameProfile; 8] {
    [
        NameProfile::posix(Some(255)),
        NameProfile::posix(Some(64)),
        NameProfile::windows(Reserved::Bare, NameEncoding::Utf8),
        NameProfile::windows(Reserved::Full, NameEncoding::Utf8Bmp),
        NameProfile::windows(Reserved::Bare, NameEncoding::Ascii),
        sfm_smb(),
        NameProfile::smb_posix(Some(255)),
        gvfs_profile("mtp"),
    ]
}

/// A name fixed for a profile, which holds what fixing promises.
struct FixedName<'a> {
    name: &'a OsStr,
    fixed: OsString,
    profile: &'a NameProfile,
    folder: bool,
}

impl FixedName<'_> {
    fn context(&self) -> String {
        format!("{:?} {:?}", self.name, self.profile)
    }

    fn assert_fixed_again_unchanged(&self) {
        assert_eq!(
            fix_name(&self.fixed, self.profile, self.folder).name,
            self.fixed,
            "{}",
            self.context()
        );
    }

    fn assert_within_byte_limit(&self) {
        let limit = self.profile.limit;
        let bytes = limit
            .folder_bytes
            .filter(|_| self.folder)
            .into_iter()
            .chain(limit.bytes)
            .min();
        assert!(
            bytes.is_none_or(|bytes| self.fixed.len() <= usize::from(bytes)),
            "{}",
            self.context()
        );
    }

    /// Encoded profiles get valid UTF-8 in their repertoire and unit limit.
    fn assert_encoded(&self) {
        let utf8 = self.fixed.to_str();
        assert!(utf8.is_some(), "{}", self.context());
        let held = utf8
            .unwrap_or_default()
            .chars()
            .all(|character| match self.profile.encoding {
                NameEncoding::Ascii => character.is_ascii(),
                NameEncoding::Utf8Bmp => character <= '\u{ffff}',
                NameEncoding::AnyBytes | NameEncoding::Utf8 => true,
            });
        assert!(held, "{}", self.context());
        let units = utf8.map_or(0, |fixed| fixed.encode_utf16().count());
        assert!(
            self.profile
                .limit
                .units
                .is_none_or(|limit| units <= usize::from(limit)),
            "{}",
            self.context()
        );
    }

    fn assert_not_hidden(&self) {
        assert!(
            !self.fixed.as_encoded_bytes().starts_with(b"."),
            "{}",
            self.context()
        );
    }

    /// A typed name is refused exactly when fixing would change it, and the
    /// fixed name is never refused.
    fn assert_refusal_agrees(&self) {
        if let Some(typed) = self.name.to_str() {
            assert_eq!(
                refusal(typed, self.profile, self.folder).is_none(),
                self.fixed == self.name,
                "{}",
                self.context()
            );
        }
        if let Some(fixed) = self.fixed.to_str() {
            assert_eq!(
                refusal(fixed, self.profile, self.folder),
                None,
                "{}",
                self.context()
            );
        }
    }

    fn assert_holds(&self) {
        self.assert_fixed_again_unchanged();
        self.assert_within_byte_limit();
        if self.profile.encoding != NameEncoding::AnyBytes {
            self.assert_encoded();
        }
        self.assert_not_hidden();
        self.assert_refusal_agrees();
    }
}

#[test]
fn fixing_a_fixed_name_changes_nothing() {
    let profiles = fixed_point_profiles();
    let mut sampler = NameSampler::new();
    for _ in 0..2000 {
        let (name, folder) = sampler.next();
        for profile in &profiles {
            FixedName {
                name: &name,
                fixed: fix_name(&name, profile, folder).name.into_owned(),
                profile,
                folder,
            }
            .assert_holds();
        }
    }
}
