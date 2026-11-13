// SPDX-License-Identifier: MIT

//! Holds `Xkb`'s accelerator reader and writer to `gtk_accelerator_parse` and
//! `gtk_accelerator_name`, as the GTK application's key backend calls them.
//!
//! The domain is generated: every set of the six modifiers in several spellings and orders,
//! crossed with every declared key's name in four cases and a sample of all names; every name
//! either library reads or writes but the code-point spellings, in four cases and under three
//! prefixes; a table of odd
//! inputs found by asking GTK, and mutations of a few accelerators at every position; every
//! keysym written, and read back from what was written. A difference that only a key lookup
//! of another library explains is tolerated in a tolerant run, and nothing else is.

use gtk::{gdk, glib::translate::IntoGlib};
use strata_desktop::keys::{Backend, Modifiers, Xkb};

use super::{Compared, Lookup, compare, gdk_key, key, keysyms, names};

const SIX: [Modifiers; 6] = [
    Modifiers::SHIFT_MASK,
    Modifiers::CONTROL_MASK,
    Modifiers::ALT_MASK,
    Modifiers::META_MASK,
    Modifiers::SUPER_MASK,
    Modifiers::HYPER_MASK,
];

/// Ways to write each of [`SIX`], in its order: GTK's own first.
const SPELLINGS: [[&str; 5]; 6] = [
    ["Shift", "shft", "SHIFT", "sHfT", "ShFt"],
    ["Control", "ctrl", "CTL", "Primary", "pRiMaRy"],
    ["Alt", "alt", "ALT", "aLt", "AlT"],
    ["Meta", "meta", "META", "mEtA", "MeTa"],
    ["Super", "super", "SUPER", "sUpEr", "SuPeR"],
    ["Hyper", "hyper", "HYPER", "hYpEr", "HyPeR"],
];

/// Inputs found by asking GTK, kept for the classes they stand for.
const ODD: &[&str] = &[
    "",
    "<",
    ">",
    "<>",
    "<a>",
    "<>a",
    "\0",
    "<Control>",
    "<Shift><Control>",
    "<Control><Control>a",
    "<Release>a",
    "<release>a",
    "<Lock>a",
    "<Caps>a",
    "<Mod1>a",
    "<Mod2>a",
    "<Mod4>a",
    "<Mod5>a",
    "<Button1>a",
    "<AltGr>a",
    "<Cmd>a",
    "<Win>a",
    "<Control_L>a",
    "<Super_L>a",
    "<\u{17f}hift>a",
    "<Sh\u{131}ft>a",
    "<SH\u{130}FT>a",
    "<Contr\u{f4}l>a",
    "<Control\u{200b}>a",
    "<Control> <Shift>a",
    "<Control>a<Shift>",
    "<Control>a<Control>b",
    "a<Control>",
    "<Control>>a",
    "<<Control>a",
    "<Control><a",
    "<Control><>",
    "<Control>a\0b",
    "<Control>\0a",
    "<Con\0trol>a",
    "\0<Control>a",
    "<Control>\u{a0}a",
    "<Control>-",
    "<Control>+",
    "<Control><",
    "<Control>>",
    "<Control>?",
    "<Control>\u{e9}",
    "<Control>\u{c9}",
    "<Control>\u{441}",
    "<Control>\u{20ac}",
    "<Control>\u{ff41}",
    "<Control>a\u{301}",
    "<Control>A A",
    "<Control>ab",
    "<Control>00",
    "<Control>10",
    "<Control>-1",
    "<Control>0x61",
    "<Control>0X61",
    "<Control>0x26",
    "<Control>0x1b4",
    "<Control>0xffffff",
    "<Control>0x0",
    "<Control>0x",
    "<Control>U+0041",
    "<Control>U0041",
    "<Control>u0041",
    "<Control>U+10FFFF",
    "<Control>VoidSymbol",
    "<Control>NoSymbol",
    "<Control>F35",
    "<Control>F36",
    "<Control>keycode38",
    "<Control>#38",
    "<0x26>",
    "<Release>0x26",
];

/// Characters put at every position of a few accelerators, and around each modifier word.
const INSERTED: [char; 12] = [
    '<', '>', ' ', '\t', '\n', '\0', 'x', '_', '1', '\u{e9}', '\u{a0}', '\u{17f}',
];

/// Accelerators changed at every position: a character put in, one taken out, and the string
/// cut short.
const MUTATED: [&str; 4] = [
    "<Control>a",
    "<Shift><Alt>F1",
    "<Primary>KP_Enter",
    "<Meta><Super>XF86AudioPlay",
];

/// GTK's ten modifier words, each in lower case.
const WORDS: [&str; 10] = [
    "shift", "shft", "control", "ctrl", "ctl", "primary", "alt", "meta", "super", "hyper",
];

/// The modifiers of `flags` whose bit is set in `subset`.
fn modifiers_of(subset: usize, flags: &[Modifiers]) -> Modifiers {
    flags
        .iter()
        .enumerate()
        .filter(|(bit, _)| subset >> bit & 1 == 1)
        .fold(Modifiers::empty(), |all, (_, modifier)| all | *modifier)
}

/// One modifier set written in one spelling scheme: `scheme` picks the column of
/// [`SPELLINGS`], and past them a column that changes with each modifier.
fn spelled(subset: usize, scheme: usize, reversed: bool, doubled: bool) -> String {
    let mut words: Vec<&str> = (0..SIX.len())
        .filter(|bit| subset >> bit & 1 == 1)
        .map(|bit| SPELLINGS[bit][(scheme + bit * usize::from(scheme >= 5)) % 5])
        .collect();
    if reversed {
        words.reverse();
    }
    let once: String = words.iter().map(|word| format!("<{word}>")).collect();
    if doubled { once.repeat(2) } else { once }
}

fn prefixes() -> Vec<String> {
    let mut prefixes = Vec::new();
    for subset in 0..1 << SIX.len() {
        for scheme in 0..6 {
            for reversed in [false, true] {
                prefixes.push(spelled(subset, scheme, reversed, scheme == 0 && reversed));
            }
        }
    }
    prefixes.sort();
    prefixes.dedup();
    prefixes
}

macro_rules! declared {
    ($($name:ident = $keysym:literal,)*) => {
        &[$($keysym),*]
    };
}

const DECLARED: &[u32] = strata_desktop::key_constants!(declared);

fn cases_of(name: &str) -> [String; 4] {
    let mut capital = name.to_lowercase();
    if let Some(first) = capital.get(..1) {
        capital = first.to_uppercase() + &capital[1..];
    }
    [
        name.to_owned(),
        name.to_uppercase(),
        name.to_lowercase(),
        capital,
    ]
}

/// libxkbcommon's spelling of a direct-Unicode keysym, `U0100`, in each form the first test
/// reads it in: it reads a million of them, and the odd inputs keep a few.
fn is_a_code_point(name: &str) -> bool {
    let plain = name.trim_start_matches("XF86").trim_start_matches('_');
    plain.strip_prefix('U').is_some_and(|digits| {
        digits.len() >= 4 && digits.bytes().all(|digit| digit.is_ascii_hexdigit())
    })
}

/// The names of the first test that are words.
fn word_names() -> Vec<String> {
    names()
        .into_iter()
        .filter(|name| !is_a_code_point(name))
        .collect()
}

/// The declared keys' names in four cases and every 20th of all names, under every prefix.
fn modifier_sets(all_names: &[String]) -> impl Iterator<Item = String> {
    let declared = DECLARED
        .iter()
        .filter_map(|keysym| gdk_key(*keysym).name())
        .flat_map(|name| cases_of(&name));
    let mut keys: Vec<String> = declared
        .chain(all_names.iter().step_by(20).cloned())
        .collect();
    keys.sort();
    keys.dedup();
    let prefixes = prefixes();
    keys.into_iter().flat_map(move |key| {
        prefixes
            .iter()
            .map(move |prefix| format!("{prefix}{key}"))
            .collect::<Vec<_>>()
    })
}

/// Every name in four cases, bare, with Control, and with three modifiers.
fn every_name(all_names: &[String]) -> impl Iterator<Item = String> + '_ {
    all_names.iter().flat_map(|name| {
        cases_of(name).into_iter().flat_map(|name| {
            ["", "<Control>", "<shft><Primary><alt>"].map(|prefix| format!("{prefix}{name}"))
        })
    })
}

fn mutations(accelerator: &str) -> Vec<String> {
    let positions: Vec<usize> = accelerator
        .char_indices()
        .map(|(at, _)| at)
        .chain([accelerator.len()])
        .collect();
    let mut mutated = Vec::new();
    for at in &positions {
        let (head, tail) = accelerator.split_at(*at);
        mutated.extend(INSERTED.map(|inserted| format!("{head}{inserted}{tail}")));
        mutated.push(head.to_owned());
        let mut rest = tail.chars();
        rest.next();
        mutated.push(format!("{head}{}", rest.as_str()));
    }
    mutated
}

/// Words near GTK's: every word of up to three letters, every ASCII case of each of the ten,
/// and each of the ten with a character before or after it.
fn modifier_words() -> Vec<String> {
    let letters = || ('a'..='z').map(String::from);
    let mut words: Vec<String> = letters()
        .chain(letters().flat_map(|first| letters().map(move |second| format!("{first}{second}"))))
        .collect();
    let three: Vec<String> = words
        .iter()
        .filter(|word| word.len() == 2)
        .flat_map(|two| letters().map(move |third| format!("{two}{third}")))
        .collect();
    words.extend(three);
    for word in WORDS {
        words.extend((0..1_u32 << word.len()).map(|flips| {
            word.chars()
                .enumerate()
                .map(|(at, letter)| {
                    if flips >> at & 1 == 1 {
                        letter.to_ascii_uppercase()
                    } else {
                        letter
                    }
                })
                .collect()
        }));
        for inserted in INSERTED.iter().chain(&['s', 'l', 'L', '1']) {
            words.push(format!("{inserted}{word}"));
            words.push(format!("{word}{inserted}"));
        }
    }
    words.into_iter().map(|word| format!("<{word}>a")).collect()
}

/// Longer than the binding's stack buffer, so it reaches GTK by another path. No NUL: the
/// binding asserts there is none in a long string in a debug build.
fn long() -> [String; 3] {
    [
        format!("{}a", "<Control>".repeat(60)),
        format!("<Control>{}", "a".repeat(400)),
        format!("<{}>a", "Control".repeat(60)),
    ]
}

fn odd() -> Vec<String> {
    let mut odd: Vec<String> = ODD.iter().map(|odd| (*odd).to_owned()).collect();
    odd.extend(
        MUTATED
            .iter()
            .flat_map(|accelerator| mutations(accelerator)),
    );
    odd.extend(modifier_words());
    odd.extend(long());
    odd
}

type Parsed = Option<(u32, u32)>;

fn xkb_parse(accelerator: &str) -> Parsed {
    Xkb.accelerator_parse(accelerator)
        .map(|(key, modifiers)| (key.keysym(), modifiers.bits()))
}

/// As the GTK application's backend reads it, which keeps only the modifiers `Modifiers` has.
fn gdk_parse(accelerator: &str) -> Parsed {
    gtk::accelerator_parse(accelerator).map(|(key, modifiers)| {
        let modifiers = Modifiers::from_bits_truncate(modifiers.bits());
        (key.into_glib(), modifiers.bits())
    })
}

/// The key a name reads as, in lower case, in each library.
fn key_part_differs(accelerator: &str) -> bool {
    let text = accelerator.split('\0').next().unwrap_or_default();
    let part = text.rsplit('>').next().unwrap_or(text);
    let xkb = Xkb
        .key_from_name(part)
        .map(|key| Xkb.key_to_lower(key).keysym());
    let gdk = gdk::Key::from_name(part).map(|key| key.to_lower().into_glib());
    xkb != gdk
}

type Written = (u32, Modifiers);

fn xkb_name((keysym, modifiers): &Written) -> String {
    Xkb.accelerator_name(key(*keysym), *modifiers)
}

fn gdk_name((keysym, modifiers): &Written) -> String {
    let modifiers = gdk::ModifierType::from_bits_truncate(modifiers.bits());
    gtk::accelerator_name(gdk_key(*keysym), modifiers).into()
}

fn name_part_differs(keysym: u32) -> bool {
    let xkb = Xkb.key_name(Xkb.key_to_lower(key(keysym)));
    let gdk = gdk_key(keysym).to_lower().name().map(String::from);
    xkb != gdk
}

fn name_explained((keysym, _): &Written, _: &String, _: &String) -> bool {
    name_part_differs(*keysym)
}

fn round_trip_explained(input: &Written, _: &Parsed, _: &Parsed) -> bool {
    name_part_differs(input.0) || key_part_differs(&gdk_name(input))
}

/// Every keysym with all seven modifiers, and every declared key with every set of the
/// seven.
fn writer_domain() -> impl Iterator<Item = Written> {
    let seven: Vec<Modifiers> = SIX.into_iter().chain([Modifiers::LOCK_MASK]).collect();
    let all = keysyms().map(|keysym| (keysym, Modifiers::all()));
    let declared = DECLARED.iter().flat_map(move |keysym| {
        let seven = seven.clone();
        (0..1 << seven.len()).map(move |subset| (*keysym, modifiers_of(subset, &seven)))
    });
    all.chain(declared)
}

/// Every keysym with Shift and Control, and every declared key with every set of the six.
fn round_trip_domain() -> impl Iterator<Item = Written> {
    let all = keysyms().map(|keysym| (keysym, Modifiers::SHIFT_MASK | Modifiers::CONTROL_MASK));
    let declared = DECLARED
        .iter()
        .flat_map(|keysym| (0..1 << SIX.len()).map(|subset| (*keysym, modifiers_of(subset, &SIX))));
    all.chain(declared)
}

fn row<I: std::fmt::Debug, T: std::fmt::Debug>(input: &I, gdk: &T) -> String {
    format!("{input:?}: GTK {gdk:x?}")
}

fn parse(name: &'static str) -> Lookup<String, Parsed> {
    Lookup {
        name,
        xkb: |accelerator| xkb_parse(accelerator),
        gdk: |accelerator| gdk_parse(accelerator),
        row,
        tolerated: |accelerator, _, _| key_part_differs(accelerator),
    }
}

/// The comparisons: three of the reader, one of the writer, and one of reading back what
/// each wrote.
pub(super) fn compared() -> [Compared; 5] {
    let all_names = word_names();
    [
        compare(
            parse("accelerator_parse_modifiers"),
            modifier_sets(&all_names),
        ),
        compare(parse("accelerator_parse_names"), every_name(&all_names)),
        compare(parse("accelerator_parse_odd"), odd().into_iter()),
        compare(
            Lookup {
                name: "accelerator_name",
                xkb: xkb_name,
                gdk: gdk_name,
                row,
                tolerated: name_explained,
            },
            writer_domain(),
        ),
        compare(
            Lookup {
                name: "accelerator_round_trip",
                xkb: |input| xkb_parse(&xkb_name(input)),
                gdk: |input| gdk_parse(&gdk_name(input)),
                row,
                tolerated: round_trip_explained,
            },
            round_trip_domain(),
        ),
    ]
}
