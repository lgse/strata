// SPDX-License-Identifier: MIT

use super::super::{input, observe::catalog_accent};

#[test]
fn single_characters_are_latin1_or_unicode_keysyms() {
    assert_eq!(input::keysym("j"), Ok(0x6a));
    assert_eq!(input::keysym("é"), Ok(0xe9));
    assert_eq!(input::keysym("о"), Ok(0x0100_043e));
    assert!(input::keysym("NotAKey").is_err());
}

#[test]
fn the_theme_accent_is_read_from_the_catalog_entry_with_the_theme_id() {
    let catalog = "[[themes]]\nid = \"dark\"\naccent = \"#2ccceb\"\n\n\
                   [[themes]]\nid = \"light\"\naccent = \"#010203\"\n";
    assert_eq!(catalog_accent(catalog, "light"), Ok([1, 2, 3]));
    assert_eq!(catalog_accent(catalog, "dark"), Ok([0x2c, 0xcc, 0xeb]));
    assert!(catalog_accent(catalog, "missing").is_err());
    assert!(catalog_accent("[[themes]]\nid = \"x\"\naccent = \"teal\"\n", "x").is_err());
}
