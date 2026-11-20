// SPDX-License-Identifier: MIT

use super::*;

/// How a name breaks UTF-8, worst last. A decoder that only checks the
/// shape of each sequence takes overlong forms and encoded surrogates.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Utf8Fault {
    None,
    Surrogate,
    Overlong,
    Shape,
}

pub fn utf8_fault(name: &OsStr) -> Utf8Fault {
    let bytes = name.as_encoded_bytes();
    let mut worst = Utf8Fault::None;
    let mut index = 0;
    while index < bytes.len() {
        let len = match bytes[index] {
            0x00..=0x7f => 1,
            0xc0..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf7 => 4,
            _ => return Utf8Fault::Shape,
        };
        let Some(sequence) = bytes
            .get(index..index + len)
            .filter(|sequence| sequence[1..].iter().all(|byte| byte & 0xc0 == 0x80))
        else {
            return Utf8Fault::Shape;
        };
        let fault = match std::str::from_utf8(sequence) {
            Ok(_) => Utf8Fault::None,
            Err(_) if sequence[0] == 0xed => Utf8Fault::Surrogate,
            Err(_) => Utf8Fault::Overlong,
        };
        worst = worst.max(fault);
        index += len;
    }
    worst
}

pub fn has_windows_forbidden(name: &OsStr) -> bool {
    name.as_encoded_bytes()
        .iter()
        .any(|byte| b"\\:*?\"<>|".contains(byte) || (0x01..0x20).contains(byte))
}

pub fn trimmed_dots(name: &OsStr) -> &[u8] {
    let bytes = name.as_encoded_bytes();
    let end = bytes
        .iter()
        .rposition(|byte| *byte != b'.')
        .map_or(0, |last| last + 1);
    &bytes[..end]
}

pub fn ends_with_dot_or_space(name: &OsStr) -> bool {
    matches!(name.as_encoded_bytes().last(), Some(b'.' | b' '))
}

/// The device names both `windows_names` drivers check: `CON`, `PRN`,
/// `AUX`, `NUL`, `COM1`-`COM9` and `LPT1`-`LPT9`, alone or before a dot.
pub fn device_name(name: &OsStr) -> bool {
    let bytes = name.as_encoded_bytes();
    let ends_at = |len: usize| bytes.len() == len || bytes.get(len) == Some(&b'.');
    let base = |len: usize| bytes.get(..len).map(<[u8]>::to_ascii_uppercase);
    let three = base(3).is_some_and(|base| {
        [&b"CON"[..], b"PRN", b"AUX", b"NUL"].contains(&base.as_slice()) && ends_at(3)
    });
    let numbered = base(3).is_some_and(|base| [&b"COM"[..], b"LPT"].contains(&base.as_slice()))
        && bytes
            .get(3)
            .is_some_and(|digit| (b'1'..=b'9').contains(digit))
        && ends_at(4);
    three || numbered
}

pub fn vfat_refuses(name: &OsStr) -> Option<Errno> {
    if utf8_fault(name) != Utf8Fault::None || has_windows_forbidden(name) {
        return Some(Errno::INVAL);
    }
    match trimmed_dots(name) {
        [] => Some(Errno::NOENT),
        [.., b' '] => Some(Errno::INVAL),
        _ => None,
    }
}

pub fn exfat_refuses(name: &OsStr) -> Option<Errno> {
    if utf8_fault(name) != Utf8Fault::None {
        Some(Errno::ILSEQ)
    } else if has_windows_forbidden(name) {
        Some(Errno::INVAL)
    } else {
        trimmed_dots(name).is_empty().then_some(Errno::NOENT)
    }
}

/// The daemon stores overlong forms and encoded surrogates, writing an
/// overlong `/` as a real one and aborting at the next listing. Nothing may
/// send them, so they fail with an errno no driver gives.
pub fn exfat_fuse_refuses(name: &OsStr) -> Option<Errno> {
    match utf8_fault(name) {
        Utf8Fault::Shape => Some(Errno::ILSEQ),
        Utf8Fault::Overlong | Utf8Fault::Surrogate => Some(Errno::UCLEAN),
        Utf8Fault::None => has_windows_forbidden(name).then_some(Errno::NOENT),
    }
}

pub fn ntfs3_refuses(name: &OsStr) -> Option<Errno> {
    (utf8_fault(name) != Utf8Fault::None).then_some(Errno::INVAL)
}

pub fn windows_names_refuses(name: &OsStr) -> Option<Errno> {
    (utf8_fault(name) != Utf8Fault::None
        || has_windows_forbidden(name)
        || ends_with_dot_or_space(name)
        || device_name(name))
    .then_some(Errno::INVAL)
}

/// Encoded surrogates pass.
pub fn ntfs_3g_refuses(name: &OsStr) -> Option<Errno> {
    if utf8_fault(name) >= Utf8Fault::Overlong {
        Some(Errno::ILSEQ)
    } else {
        (has_windows_forbidden(name) || ends_with_dot_or_space(name) || device_name(name))
            .then_some(Errno::INVAL)
    }
}

/// Only `\` is refused; `mapposix` maps the other characters.
pub fn cifs_refuses(name: &OsStr) -> Option<Errno> {
    if utf8_fault(name) != Utf8Fault::None {
        Some(Errno::NOENT)
    } else {
        name.as_encoded_bytes()
            .contains(&b'\\')
            .then_some(Errno::INVAL)
    }
}

/// `nocase` also refuses every character above U+FFFF.
pub fn cifs_nocase_refuses(name: &OsStr) -> Option<Errno> {
    let astral = name
        .to_str()
        .is_some_and(|name| name.chars().any(|character| u32::from(character) > 0xffff));
    (utf8_fault(name) != Utf8Fault::None || astral || cifs_refuses(name).is_some())
        .then_some(Errno::INVAL)
}

/// What the SMB client lists: the character each `mapposix` code point
/// stands for, and an 8.3 alias for a name Samba takes for a device.
pub fn smb_listed(name: &OsStr) -> Option<OsString> {
    let name = name.to_str()?;
    let (base, extension) = name.split_once('.').unwrap_or((name, ""));
    let reserved = [
        "AUX", "CLOCK$", "COM1", "COM2", "COM3", "COM4", "CON", "LPT1", "LPT2", "LPT3", "NUL",
        "PRN",
    ];
    if reserved
        .iter()
        .any(|device| base.eq_ignore_ascii_case(device))
    {
        let extension = extension.rsplit('.').next().unwrap_or_default();
        let extension = extension.get(..3).unwrap_or(extension).to_ascii_uppercase();
        let dot = if extension.is_empty() { "" } else { "." };
        let first = base[..1].to_ascii_uppercase();
        return Some(OsString::from(format!("{first}MANGL~F{dot}{extension}")));
    }
    let listed = name
        .chars()
        .map(|character| match character {
            '\u{f001}'..='\u{f01f}' => {
                char::from_u32(u32::from(character) - 0xf000).unwrap_or(character)
            }
            '\u{f020}' => '"',
            '\u{f021}' => '*',
            '\u{f022}' => ':',
            '\u{f023}' => '<',
            '\u{f024}' => '>',
            '\u{f025}' => '?',
            '\u{f027}' => '|',
            '\u{f028}' => ' ',
            '\u{f029}' => '.',
            character => character,
        })
        .collect::<String>();
    (listed != name).then(|| OsString::from(listed))
}

pub fn without_trailing_dots(name: &OsStr) -> Option<OsString> {
    let trimmed = trimmed_dots(name);
    (trimmed.len() < name.len()).then(|| OsString::from_vec(trimmed.to_vec()))
}

/// SMB carries UTF-16, so the client sends `?` for every byte that is not UTF-8.
pub fn question_marks(name: &OsStr) -> Option<OsString> {
    let bytes = name.as_encoded_bytes();
    if std::str::from_utf8(bytes).is_ok() {
        return None;
    }
    let mut stored = Vec::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        stored.extend_from_slice(chunk.valid().as_bytes());
        stored.extend(chunk.invalid().iter().map(|_| b'?'));
    }
    Some(OsString::from_vec(stored))
}
