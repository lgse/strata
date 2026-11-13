// SPDX-License-Identifier: MIT

//! Holds `strata_desktop::keys::Xkb`, the key library of the Rune interface, to GDK over
//! every keysym.
//!
//! `Xkb` asks libxkbcommon and corrects its answers by rule wherever GDK answers differently.
//! This test is what keeps those rules complete: it fails on any difference they do not
//! cover, and writes every one of them to `key-lookups/` under the test's target directory,
//! in the form the rule tables take. It needs no display. `docs/testing.md`, "The key
//! lookups and the test image", says what a change of GTK, GLib or libxkbcommon does to it.

use std::{fmt::Debug, fs, path::PathBuf};

use gtk::{
    gdk,
    glib::translate::{FromGlib, IntoGlib},
};
use strata_desktop::keys::{Backend, Key, Xkb, library_name};

#[expect(
    unsafe_code,
    reason = "gdk4 has no safe way to make a key from its number"
)]
fn gdk_key(keysym: u32) -> gdk::Key {
    // SAFETY: a `gdk::Key` is its keysym, and every `u32` is one.
    unsafe { gdk::Key::from_glib(keysym) }
}

fn key(keysym: u32) -> Key {
    Key::from_keysym(keysym)
}

/// The legacy keysyms, the direct-Unicode ones, the vendors' keys and `VoidSymbol`.
fn keysyms() -> impl Iterator<Item = u32> {
    (0..0x2_0000)
        .chain(
            (0x0100_0000..0x0111_0000)
                .filter(|keysym| !(0x0100_d800..0x0100_e000).contains(keysym)),
        )
        .chain(0x1000_0000..0x1009_0000)
        .chain([0x00ff_ffff])
}

fn characters() -> impl Iterator<Item = char> {
    (1..0x3_0000).filter_map(char::from_u32)
}

/// The names in the installed header of libxkbcommon, when it is installed: the ones the
/// library reads, among them the names it never writes.
fn header_names() -> Vec<String> {
    let Ok(header) = fs::read_to_string("/usr/include/xkbcommon/xkbcommon-keysyms.h") else {
        return Vec::new();
    };
    header
        .lines()
        .filter_map(|line| line.strip_prefix("#define XKB_KEY_"))
        .filter_map(|rest| rest.split_whitespace().next())
        .map(str::to_owned)
        .collect()
}

/// One lookup compared over a domain: what differs, each as the row a rule table would hold
/// for it, with what `Xkb` answered after it.
struct Compared {
    lookup: &'static str,
    compared: usize,
    differences: Vec<String>,
    /// Differences of a class that follows the installed libraries, not the rules.
    tolerated: Vec<String>,
}

/// A lookup to compare: `Xkb`'s answer, GDK's, how a difference is written as a row, and
/// whether a difference is one that a library of another version explains.
struct Lookup<I, T> {
    name: &'static str,
    xkb: fn(&I) -> T,
    gdk: fn(&I) -> T,
    row: fn(&I, &T) -> String,
    tolerated: fn(&I, &T, &T) -> bool,
}

/// Whether this is the pinned test image, where the rules are exact and nothing is
/// tolerated. `./scripts/quality.sh` names the commit it tests there.
fn in_the_test_image() -> bool {
    std::env::var_os("STRATA_QUALITY_COMMIT").is_some()
}

fn compare<I, T: PartialEq + Debug>(
    lookup: Lookup<I, T>,
    domain: impl Iterator<Item = I>,
) -> Compared {
    let strict = in_the_test_image();
    let mut compared = Compared {
        lookup: lookup.name,
        compared: 0,
        differences: Vec::new(),
        tolerated: Vec::new(),
    };
    for input in domain {
        compared.compared += 1;
        let (xkb, gdk) = ((lookup.xkb)(&input), (lookup.gdk)(&input));
        if xkb == gdk {
            continue;
        }
        let row = format!("{} // xkb: {xkb:x?}", (lookup.row)(&input, &gdk));
        if !strict && (lookup.tolerated)(&input, &xkb, &gdk) {
            compared.tolerated.push(row);
        } else {
            compared.differences.push(row);
        }
    }
    compared
}

fn folder() -> PathBuf {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("key-lookups");
    fs::create_dir_all(&folder).expect("a folder for the differences");
    folder
}

impl Compared {
    /// The differences written to a file, and a line for the failure message.
    fn report(&self) -> Option<String> {
        if self.differences.is_empty() {
            return None;
        }
        let file = folder().join(format!("{}.txt", self.lookup));
        fs::write(&file, self.differences.join("\n")).expect("the differences");
        Some(format!(
            "\n{}: {} of {} differ from GDK; all of them are in {}\n  {}\n",
            self.lookup,
            self.differences.len(),
            self.compared,
            file.display(),
            self.differences
                .iter()
                .take(8)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n  ")
        ))
    }

    /// The tolerated differences written to a file, and their count for the test's output.
    fn tolerance(&self) -> String {
        if !self.tolerated.is_empty() {
            let file = folder().join(format!("tolerated-{}.txt", self.lookup));
            fs::write(file, self.tolerated.join("\n")).expect("the tolerated differences");
        }
        format!("{} {}", self.lookup, self.tolerated.len())
    }
}

fn never<I, T>(_: &I, _: &T, _: &T) -> bool {
    false
}

fn is_a_number(name: &Option<String>) -> bool {
    name.as_deref().is_none_or(|name| name.starts_with("0x"))
}

/// One library names the key and the other writes its number: the two know different keys.
fn one_has_no_name(_: &u32, xkb: &Option<String>, gdk: &Option<String>) -> bool {
    is_a_number(xkb) != is_a_number(gdk)
}

/// One library reads the name and the other does not.
fn one_does_not_read(_: &String, xkb: &Option<u32>, gdk: &Option<u32>) -> bool {
    xkb.is_some() != gdk.is_some()
}

/// A direct-Unicode keysym whose character has another case in one library's Unicode and
/// none in the other's: GLib's and the compiler's Unicode tables are of different versions.
fn case_skew(keysym: &u32, xkb: &u32, gdk: &u32) -> bool {
    let Some(character) = keysym
        .checked_sub(0x0100_0000)
        .filter(|code| *code < 0x11_0000)
        .and_then(char::from_u32)
    else {
        return false;
    };
    let unchanged = Xkb.key_from_unicode(character).keysym();
    (xkb == &unchanged) != (gdk == &unchanged)
}

/// A key or a code point spelled as a number, in the forms libxkbcommon reads. GDK reads
/// none of them, and `Xkb` refuses them itself.
const NUMBERS: [&str; 10] = [
    "U41",
    "U0041",
    "U20AC",
    "u20ac",
    "UD800",
    "U10FFFF",
    "0x61",
    "0X61",
    "0x0061",
    "0x1000061",
];

/// Every name either library writes or reads: what GDK writes, what libxkbcommon writes
/// (for a direct-Unicode keysym that is a number, `U0100`), the header's names, each with
/// and without the vendor prefix, and the numbers.
fn names() -> Vec<String> {
    let written = keysyms().flat_map(|keysym| {
        [
            gdk_key(keysym).name().map(String::from),
            library_name(keysym),
        ]
    });
    let mut names: Vec<String> = written
        .flatten()
        .chain(header_names())
        .filter(|name| !name.starts_with("0x") && !name.starts_with("U+"))
        .flat_map(|name| {
            let plain = name.strip_prefix("XF86").unwrap_or(&name).to_owned();
            [format!("XF86{plain}"), plain]
        })
        .collect();
    names.extend(NUMBERS.iter().map(|number| (*number).to_owned()));
    names.sort();
    names.dedup();
    names
}

fn keysym_pair(keysym: &u32, gdk: &u32) -> String {
    format!("({keysym:#x}, {gdk:#x}),")
}

#[test]
fn every_key_lookup_answers_as_gdk_does() {
    let started = std::time::Instant::now();
    let compared = [
        compare(
            Lookup {
                name: "name",
                xkb: |keysym| Xkb.key_name(key(*keysym)),
                gdk: |keysym| gdk_key(*keysym).name().map(String::from),
                row: |keysym, name| format!("({keysym:#x}, {name:?}),"),
                tolerated: one_has_no_name,
            },
            keysyms(),
        ),
        compare(
            Lookup {
                name: "from_name",
                xkb: |name: &String| Xkb.key_from_name(name).map(Key::keysym),
                gdk: |name| gdk::Key::from_name(name.as_str()).map(IntoGlib::into_glib),
                row: |name, keysym| match keysym {
                    Some(keysym) => format!("({name:?}, Some({keysym:#x})),"),
                    None => format!("({name:?}, None),"),
                },
                tolerated: one_does_not_read,
            },
            names().into_iter(),
        ),
        compare(
            Lookup {
                name: "to_unicode",
                xkb: |keysym| Xkb.key_to_unicode(key(*keysym)),
                gdk: |keysym| gdk_key(*keysym).to_unicode(),
                row: |keysym, character| format!("({keysym:#x}, {character:?}),"),
                tolerated: never,
            },
            keysyms(),
        ),
        compare(
            Lookup {
                name: "from_unicode",
                xkb: |character: &char| Xkb.key_from_unicode(*character).keysym(),
                gdk: |character| gdk::unicode_to_keyval(u32::from(*character)),
                row: |character, keysym| format!("({character:?}, {keysym:#x}),"),
                tolerated: never,
            },
            characters(),
        ),
        compare(
            Lookup {
                name: "to_lower",
                xkb: |keysym| Xkb.key_to_lower(key(*keysym)).keysym(),
                gdk: |keysym| gdk_key(*keysym).to_lower().into_glib(),
                row: keysym_pair,
                tolerated: case_skew,
            },
            keysyms(),
        ),
        compare(
            Lookup {
                name: "to_upper",
                xkb: |keysym| Xkb.key_to_upper(key(*keysym)).keysym(),
                gdk: |keysym| gdk_key(*keysym).to_upper().into_glib(),
                row: keysym_pair,
                tolerated: case_skew,
            },
            keysyms(),
        ),
    ];
    let report: String = compared.iter().filter_map(Compared::report).collect();
    let tolerated: Vec<String> = compared.iter().map(Compared::tolerance).collect();
    let run = format!(
        "GTK {}.{}.{}, {} header names, compared in {:?}; tolerated as the libraries' own: {}",
        gtk::major_version(),
        gtk::minor_version(),
        gtk::micro_version(),
        header_names().len(),
        started.elapsed(),
        tolerated.join(", ")
    );
    println!("{run}");
    assert!(report.is_empty(), "{run}{report}");
}
