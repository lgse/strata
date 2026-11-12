// SPDX-License-Identifier: MIT

use super::super::super::names::{
    fix::{Refusal, refusal},
    stricter_names,
};
use super::*;
use crate::adapters::volume::{StrictRule, Strictness};

#[test]
fn a_typed_name_is_refused_for_the_first_thing_its_mount_would_change() {
    use Refusal::{Characters, LeadingSpace, Reserved as Device, TooLong, TrailingDotOrSpace};
    let posix = NameProfile::posix(Some(255));
    let bmp = NameProfile::windows(Reserved::Full, NameEncoding::Utf8Bmp);
    let ascii = NameProfile::windows(Reserved::Bare, NameEncoding::Ascii);
    let smb = sfm_smb();
    let mtp = gvfs_profile("mtp");
    let folder_name = "f".repeat(214);
    let rows: [(NameProfile, &str, bool, Option<Refusal>); 23] = [
        (posix, "a:b.", false, None),
        (posix, "CON", false, None),
        (posix, &"a".repeat(255), false, None),
        (posix, &"a".repeat(256), false, Some(TooLong)),
        (fat(), "a:b:c?d.", false, Some(Characters(vec![':', '?']))),
        (fat(), "a\u{1}b", false, Some(Characters(vec!['\u{1}']))),
        (fat(), "x.", false, Some(TrailingDotOrSpace)),
        (fat(), "CON ", false, Some(TrailingDotOrSpace)),
        (fat(), " x", false, None),
        (fat(), "CON", false, Some(Device("CON".to_owned()))),
        (fat(), "nul.txt", false, Some(Device("nul".to_owned()))),
        (fat(), "aux.c", false, None),
        (fat(), &"\u{4e2d}".repeat(255), false, None),
        (fat(), &"\u{4e2d}".repeat(256), false, Some(TooLong)),
        (
            bmp,
            "a\u{1f600}b",
            false,
            Some(Characters(vec!['\u{1f600}'])),
        ),
        (bmp, "CON .txt", false, Some(Device("CON".to_owned()))),
        (ascii, "caf\u{e9}", false, Some(Characters(vec!['\u{e9}']))),
        (smb, "x\u{f022}y", false, Some(Characters(vec!['\u{f022}']))),
        (smb, &folder_name, true, Some(TooLong)),
        (smb, &folder_name, false, None),
        (mtp, " x", false, Some(LeadingSpace)),
        (mtp, " .x", false, None),
        (mtp, "x ", false, Some(TrailingDotOrSpace)),
    ];
    for (profile, name, new_folder, expected) in rows {
        assert_eq!(
            refusal(name, &profile, new_folder),
            expected,
            "{name:?} {profile:?}"
        );
    }
}

#[test]
fn a_typed_name_refusal_quotes_each_character_and_gives_the_code_point_of_those_that_show_nothing()
{
    use super::super::super::typed_names::refused;
    let bmp = NameProfile::windows(Reserved::Full, NameEncoding::Utf8Bmp);
    let ascii = NameProfile::windows(Reserved::Bare, NameEncoding::Ascii);
    let smb = sfm_smb();
    for (profile, name, shown) in [
        (fat(), "a:b?", &[":", "?"][..]),
        (fat(), "tab\tx", &["U+0009"]),
        (bmp, "a\u{1f600}", &["\u{1f600}"]),
        (ascii, "caf\u{e9}", &["\u{e9}"]),
        (ascii, "e\u{301}", &["U+0301"]),
        (ascii, "a\u{200b}b", &["U+200B"]),
        (ascii, "a\u{a0}b", &["U+00A0"]),
        (smb, "x\u{f022}y", &["U+F022"]),
    ] {
        let message = rust_i18n::t!(
            "Names in this location can’t contain %{characters}",
            characters =
                crate::i18n::list(shown.iter().map(|character| crate::i18n::quoted(character)))
        );
        assert_eq!(
            refused(name, &profile, false).as_deref(),
            Some(message.as_ref()),
            "{name:?}"
        );
    }
}

#[test]
fn a_refused_name_tries_each_rule_alone_before_rules_together() {
    use RenameCause::{Invalid, TooLong};
    use StrictRule::{Characters, DeviceNames, Length, TrailingDots, Utf8};
    let posix = NameProfile::posix(None);
    // As ntfs-3g where udev cannot name its device.
    let windows = NameProfile::windows(Reserved::Bare, NameEncoding::Utf8).learning();
    let (long, cut) = ("a".repeat(300), "a".repeat(255));
    type Fixes<'a> = Vec<(&'a [StrictRule], &'a str, RenameCause)>;
    let rows: [(&[u8], NameProfile, Fixes<'_>); 10] = [
        (b"plain", posix, vec![]),
        (b"x\xffy", posix, vec![(&[Utf8], "x_y", Invalid)]),
        (b"a:b", posix, vec![(&[Characters], "a_b", Invalid)]),
        (b"x.", posix, vec![(&[TrailingDots], "x", Invalid)]),
        (
            b"con.txt",
            posix,
            vec![(&[DeviceNames], "con_.txt", Invalid)],
        ),
        (long.as_bytes(), posix, vec![(&[Length], &cut, TooLong)]),
        (
            b"aux.c:",
            posix,
            vec![
                (&[Characters], "aux.c_", Invalid),
                (&[DeviceNames], "aux_.c:", Invalid),
                (&[Characters, DeviceNames], "aux_.c_", Invalid),
            ],
        ),
        (
            b"a:b.",
            posix,
            vec![
                (&[Characters], "a_b.", Invalid),
                (&[TrailingDots], "a:b", Invalid),
                (&[Characters, TrailingDots], "a_b", Invalid),
            ],
        ),
        // What the folder's own rules give comes first and needs nothing more.
        (b"aux.c", windows, vec![(&[DeviceNames], "aux_.c", Invalid)]),
        (
            b"aux.c.",
            windows,
            vec![(&[], "aux.c", Invalid), (&[DeviceNames], "aux_.c", Invalid)],
        ),
    ];
    for (name, rules, expected) in rows {
        let name = OsString::from_vec(name.to_vec());
        let expected = expected
            .into_iter()
            .map(|(strict, saved, cause)| {
                (Strictness::of(strict.iter().copied()), saved.into(), cause)
            })
            .collect::<Vec<(Strictness, OsString, RenameCause)>>();
        assert_eq!(stricter_names(&name, &rules, false), expected, "{name:?}");
    }
}
