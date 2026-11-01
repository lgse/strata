// SPDX-License-Identifier: MIT

//! The name a destination stores for a name it would refuse, alter or find
//! too long, by the rules of its mount's profile.

use std::{
    borrow::Cow,
    collections::HashSet,
    ffi::{OsStr, OsString},
    ops::Range,
    os::unix::ffi::{OsStrExt, OsStringExt},
};

use gtk::pango;

use crate::{
    adapters::volume::{
        NameEncoding, NameLimit, NameProfile, NameRules, PrivateUse, Reserved, Strip,
    },
    services::RenameCause,
};

#[derive(Debug)]
pub(in crate::adapters::local_operations) struct Fixed<'a> {
    pub name: Cow<'a, OsStr>,
    pub cause: Option<RenameCause>,
}

/// Idempotent: fixing a fixed name changes nothing.
pub(in crate::adapters::local_operations) fn fix_name<'a>(
    name: &'a OsStr,
    profile: &NameProfile,
    is_directory: bool,
) -> Fixed<'a> {
    let limit = Limit::of(profile.limit, is_directory);
    let fixed = tidied(storable(Cow::Borrowed(name), profile), profile);
    if limit.fits(fixed.as_bytes()) {
        let cause = (*fixed != *name).then_some(RenameCause::Invalid);
        return Fixed { name: fixed, cause };
    }
    let mut fixed = tidied(Cow::Owned(shortened(&fixed, limit, is_directory)), profile);
    if !limit.fits(fixed.as_bytes()) {
        // The cut left a device name, which took a `_` past the limit.
        fixed = trimmed(
            Cow::Owned(shortened(&fixed, limit, is_directory)),
            profile.strip,
        );
    }
    Fixed {
        name: Cow::Owned(fixed.into_owned()),
        cause: Some(RenameCause::TooLong),
    }
}

/// The extension keeps its dot: the last one, or `.tar` with the one after it. Folders and
/// names such as `.bashrc` have none.
pub(in crate::adapters::local_operations) fn split_extension(
    name: &OsStr,
    is_directory: bool,
) -> (&OsStr, &OsStr) {
    let bytes = name.as_bytes();
    let Some(dot) = bytes
        .iter()
        .rposition(|byte| *byte == b'.')
        .filter(|dot| !is_directory && *dot > 0 && dot + 1 < bytes.len())
    else {
        return (name, OsStr::new(""));
    };
    let compression = &bytes[dot + 1..];
    let dot = dot
        .checked_sub(b".tar".len())
        .filter(|tar| {
            *tar > 0
                && compression.len() <= 4
                && compression.iter().all(u8::is_ascii_alphanumeric)
                && bytes[*tar..dot].eq_ignore_ascii_case(b".tar")
        })
        .unwrap_or(dot);
    (
        OsStr::from_bytes(&bytes[..dot]),
        OsStr::from_bytes(&bytes[dot..]),
    )
}

#[derive(Debug, Eq, PartialEq)]
pub(in crate::adapters::local_operations) enum Refusal {
    Characters(Vec<char>),
    LeadingSpace,
    TrailingDotOrSpace,
    Reserved(String),
    TooLong,
}

/// `Some` exactly when [`fix_name`] changes `name`. A new folder's name may have to be shorter.
pub(in crate::adapters::local_operations) fn refusal(
    name: &str,
    profile: &NameProfile,
    new_folder: bool,
) -> Option<Refusal> {
    let mut seen = HashSet::new();
    let characters: Vec<char> = name
        .chars()
        .filter(|character| refuses(profile, *character) && seen.insert(*character))
        .collect();
    if !characters.is_empty() {
        return Some(Refusal::Characters(characters));
    }
    let typed = OsStr::new(name);
    if *trimmed(Cow::Borrowed(typed), profile.strip) != *typed {
        return Some(if name.ends_with(['.', ' ']) {
            Refusal::TrailingDotOrSpace
        } else {
            Refusal::LeadingSpace
        });
    }
    if let Some(word_end) = reserved_word(name.as_bytes(), profile.reserved) {
        return Some(Refusal::Reserved(name[..word_end].to_owned()));
    }
    (!Limit::of(profile.limit, new_folder).fits(name.as_bytes())).then_some(Refusal::TooLong)
}

fn storable<'a>(name: Cow<'a, OsStr>, profile: &NameProfile) -> Cow<'a, OsStr> {
    let checks_encoding = profile.encoding != NameEncoding::AnyBytes;
    if !checks_encoding && profile.rules == NameRules::Posix && profile.mapped.is_none() {
        return name;
    }
    let refused = |character: char| refuses(profile, character);
    let chunks = || name.as_bytes().utf8_chunks();
    if !chunks().any(|chunk| {
        chunk.valid().chars().any(refused) || checks_encoding && !chunk.invalid().is_empty()
    }) {
        return name;
    }
    let mut stored = Vec::with_capacity(name.len());
    for chunk in chunks() {
        for character in chunk.valid().chars() {
            let character = if refused(character) { '_' } else { character };
            stored.extend_from_slice(character.encode_utf8(&mut [0; 4]).as_bytes());
        }
        match chunk.invalid() {
            [] => {}
            _ if checks_encoding => stored.push(b'_'),
            invalid => stored.extend_from_slice(invalid),
        }
    }
    Cow::Owned(OsString::from_vec(stored))
}

fn refuses(profile: &NameProfile, character: char) -> bool {
    !holds(profile.encoding, character)
        || profile.rules == NameRules::Windows && windows_refuses(character)
        || profile
            .mapped
            .is_some_and(|mapped| reads_back(mapped, character))
}

fn holds(encoding: NameEncoding, character: char) -> bool {
    match encoding {
        NameEncoding::AnyBytes | NameEncoding::Utf8 => true,
        NameEncoding::Utf8Bmp => character <= '\u{ffff}',
        NameEncoding::Ascii => character.is_ascii(),
    }
}

fn windows_refuses(character: char) -> bool {
    matches!(
        character,
        '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*' | '\u{1}'..='\u{1f}'
    )
}

/// The private-use characters an SMB client turns back into ASCII.
fn reads_back(mapped: PrivateUse, character: char) -> bool {
    match mapped {
        // The client reads U+F026 back unchanged.
        PrivateUse::Sfm => {
            ('\u{f001}'..='\u{f029}').contains(&character) && character != '\u{f026}'
        }
        PrivateUse::Sfu => matches!(
            character,
            '\u{f02a}' | '\u{f03a}' | '\u{f03c}' | '\u{f03e}' | '\u{f03f}' | '\u{f07c}'
        ),
    }
}

fn tidied<'a>(name: Cow<'a, OsStr>, profile: &NameProfile) -> Cow<'a, OsStr> {
    unreserved(trimmed(name, profile.strip), profile.reserved)
}

fn trimmed(name: Cow<'_, OsStr>, strip: Strip) -> Cow<'_, OsStr> {
    if strip == Strip::None {
        return name;
    }
    let bytes = name.as_bytes();
    let end = bytes
        .iter()
        .rposition(|byte| !matches!(byte, b'.' | b' '))
        .map_or(0, |last| last + 1);
    let start = match strip {
        // Stripping the spaces before a dot would hide the item.
        Strip::Ends => match bytes[..end].iter().position(|byte| *byte != b' ') {
            Some(first) if bytes[first] == b'.' => 0,
            first => first.unwrap_or(end),
        },
        Strip::None | Strip::Trailing => 0,
    };
    if start == end {
        Cow::Borrowed(OsStr::new("_"))
    } else if end - start == bytes.len() {
        name
    } else {
        part(name, start..end)
    }
}

fn part(name: Cow<'_, OsStr>, range: Range<usize>) -> Cow<'_, OsStr> {
    match name {
        Cow::Borrowed(name) => Cow::Borrowed(OsStr::from_bytes(&name.as_bytes()[range])),
        Cow::Owned(name) => {
            let mut bytes = name.into_vec();
            bytes.truncate(range.end);
            bytes.drain(..range.start);
            Cow::Owned(OsString::from_vec(bytes))
        }
    }
}

/// Puts `_` after a device name Windows would open instead of the file:
/// `CON` becomes `CON_`, and `nul.txt` becomes `nul_.txt`.
fn unreserved(name: Cow<'_, OsStr>, reserved: Reserved) -> Cow<'_, OsStr> {
    let bytes = name.as_bytes();
    let Some(word_end) = reserved_word(bytes, reserved) else {
        return name;
    };
    let mut unreserved = Vec::with_capacity(bytes.len() + 1);
    unreserved.extend_from_slice(&bytes[..word_end]);
    unreserved.push(b'_');
    unreserved.extend_from_slice(&bytes[word_end..]);
    Cow::Owned(OsString::from_vec(unreserved))
}

fn reserved_word(name: &[u8], reserved: Reserved) -> Option<usize> {
    if reserved == Reserved::None {
        return None;
    }
    let base_end = name
        .iter()
        .position(|byte| *byte == b'.')
        .unwrap_or(name.len());
    // Windows drops the spaces after a device name.
    let word_end = name[..base_end]
        .iter()
        .rposition(|byte| *byte != b' ')
        .map_or(0, |last| last + 1);
    let word = &name[..word_end];
    let refused = is_device(word)
        && match reserved {
            Reserved::None => false,
            Reserved::Bare => base_end == name.len() || word.eq_ignore_ascii_case(b"NUL"),
            Reserved::Full => true,
        };
    refused.then_some(word_end)
}

fn is_device(word: &[u8]) -> bool {
    const DEVICES: [&[u8]; 6] = [b"CON", b"PRN", b"AUX", b"NUL", b"CONIN$", b"CONOUT$"];
    const PORT_NUMBERS: [&str; 13] = [
        "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "\u{b9}", "\u{b2}", "\u{b3}",
    ];
    DEVICES
        .iter()
        .any(|device| word.eq_ignore_ascii_case(device))
        || word.split_at_checked(3).is_some_and(|(port, number)| {
            (port.eq_ignore_ascii_case(b"COM") || port.eq_ignore_ascii_case(b"LPT"))
                && PORT_NUMBERS.iter().any(|digit| number == digit.as_bytes())
        })
}

#[derive(Clone, Copy)]
struct Limit {
    bytes: Option<usize>,
    units: Option<usize>,
}

impl Limit {
    fn of(limit: NameLimit, is_directory: bool) -> Self {
        let folder_bytes = limit.folder_bytes.filter(|_| is_directory);
        let bytes = match (limit.bytes, folder_bytes) {
            (Some(bytes), Some(folder_bytes)) => Some(bytes.min(folder_bytes)),
            (bytes, folder_bytes) => bytes.or(folder_bytes),
        };
        Self {
            bytes: bytes.map(usize::from),
            units: limit.units.map(usize::from),
        }
    }

    fn fits(self, name: &[u8]) -> bool {
        self.holds(name.len(), || units(name))
    }

    /// Counts units only past as many bytes, since a name never takes more
    /// units than bytes.
    fn holds(self, bytes: usize, count_units: impl FnOnce() -> usize) -> bool {
        self.bytes.is_none_or(|limit| bytes <= limit)
            && self
                .units
                .is_none_or(|limit| bytes <= limit || count_units() <= limit)
    }

    fn half(self) -> Self {
        Self {
            bytes: self.bytes.map(|bytes| bytes / 2),
            units: self.units.map(|units| units / 2),
        }
    }

    fn without(self, part: &[u8]) -> Self {
        Self {
            bytes: self.bytes.map(|bytes| bytes.saturating_sub(part.len())),
            units: self.units.map(|limit| limit.saturating_sub(units(part))),
        }
    }

    fn prefix_end(self, name: &[u8]) -> usize {
        let (mut bytes, mut units) = (0, 0);
        for (piece_bytes, piece_units) in pieces(name) {
            if !self.holds(bytes + piece_bytes, || units + piece_units) {
                break;
            }
            bytes += piece_bytes;
            units += piece_units;
        }
        bytes
    }
}

fn pieces(name: &[u8]) -> impl Iterator<Item = (usize, usize)> + '_ {
    name.utf8_chunks().flat_map(|chunk| {
        chunk
            .valid()
            .chars()
            .map(|character| (character.len_utf8(), character.len_utf16()))
            .chain(chunk.invalid().iter().map(|_| (1, 1)))
    })
}

pub(in crate::adapters::local_operations) fn units(name: &[u8]) -> usize {
    pieces(name).map(|(_, units)| units).sum()
}

/// Only the stem [`split_extension`] finds in `stem` and `extension` together is cut, so
/// `.tar` stays before ` (1).gz`; where that extension takes more than half the limit, the
/// whole name is cut and `tail` goes last.
pub(in crate::adapters::local_operations) fn fit(
    stem: &OsStr,
    tail: &str,
    extension: &OsStr,
    limit: NameLimit,
    is_directory: bool,
) -> OsString {
    let limit = Limit::of(limit, is_directory);
    let joined = [stem.as_bytes(), tail.as_bytes(), extension.as_bytes()].concat();
    if limit.fits(&joined) {
        return OsString::from_vec(joined);
    }
    let name = [stem.as_bytes(), extension.as_bytes()].concat();
    OsString::from_vec(cut(&name, stem.len(), tail.as_bytes(), limit, is_directory))
}

fn shortened(name: &OsStr, limit: Limit, is_directory: bool) -> OsString {
    OsString::from_vec(cut(name.as_bytes(), name.len(), b"", limit, is_directory))
}

fn cut(name: &[u8], at: usize, tail: &[u8], limit: Limit, is_directory: bool) -> Vec<u8> {
    let (stem, _) = split_extension(OsStr::from_bytes(name), is_directory);
    let (stem_end, at) = if limit.half().fits(&name[stem.len()..]) {
        (stem.len().min(at), at)
    } else {
        (name.len(), name.len())
    };
    let stem = &name[..stem_end];
    let kept = [&name[stem_end..at], tail, &name[at..]].concat();
    let end = match limit.without(&kept).prefix_end(stem) {
        0 => pieces(stem).next().map_or(0, |(bytes, _)| bytes),
        end => cluster_start(stem, end),
    };
    [&stem[..end], &kept].concat()
}

/// The furthest a cut moves back to keep a grapheme cluster whole: longer
/// clusters, such as a letter under hundreds of marks, are cut inside.
const CLUSTER_WALK_BACK: usize = 8;

fn cluster_start(stem: &[u8], end: usize) -> usize {
    let mut offset = 0;
    for chunk in stem.utf8_chunks() {
        let text = chunk.valid();
        if offset < end && end < offset + text.len() {
            return text_cluster_start(text, end - offset)
                .map(|start| offset + start)
                .filter(|start| *start > 0)
                .unwrap_or(end);
        }
        offset += text.len() + chunk.invalid().len();
        if offset >= end {
            break;
        }
    }
    end
}

fn text_cluster_start(text: &str, end: usize) -> Option<usize> {
    let cursor = cursor_positions(text)?;
    let starts: Vec<usize> = text.char_indices().map(|(start, _)| start).collect();
    let at = starts.partition_point(|start| *start < end);
    (at.saturating_sub(CLUSTER_WALK_BACK)..=at)
        .rev()
        .find(|index| cursor[*index])
        .map(|index| starts[index])
}

#[expect(
    unsafe_code,
    reason = "Pango's bindings leave out the log attributes that hold its cluster breaks"
)]
fn cursor_positions(text: &str) -> Option<Vec<bool>> {
    // A `PangoLogAttr` is one `guint` of bit-fields; this is its fifth.
    const IS_CURSOR_POSITION: u32 = if cfg!(target_endian = "little") {
        1 << 4
    } else {
        1 << 27
    };
    let length = i32::try_from(text.len()).ok()?;
    let mut attrs = vec![0_u32; text.chars().count() + 1];
    let attrs_len = i32::try_from(attrs.len()).ok()?;
    // SAFETY: `text` is valid UTF-8 of `length` bytes, and `attrs` holds the
    // one 32-bit `PangoLogAttr` per character, plus one, that the call fills.
    // A null language means none. The call shares no state across threads
    // but libthai's breaker, which Pango locks.
    unsafe {
        pango::ffi::pango_get_log_attrs(
            text.as_ptr().cast(),
            length,
            -1,
            std::ptr::null_mut(),
            attrs.as_mut_ptr().cast(),
            attrs_len,
        );
    }
    Some(
        attrs
            .iter()
            .map(|attr| attr & IS_CURSOR_POSITION != 0)
            .collect(),
    )
}
