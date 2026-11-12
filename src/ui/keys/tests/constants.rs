// SPDX-License-Identifier: MIT

use gtk::{gdk, glib::translate::IntoGlib};

macro_rules! named {
    ($($name:ident = $keysym:literal,)*) => {
        &[$((stringify!($name), $keysym)),*]
    };
}

const CONSTANTS: &[(&str, u32)] = strata_desktop::key_constants!(named);

#[test]
fn every_key_constant_is_the_gdk_key_of_its_name() {
    assert!(CONSTANTS.len() > 200, "{} constants", CONSTANTS.len());
    for (constant, keysym) in CONSTANTS {
        let name = constant
            .strip_prefix('_')
            .filter(|digits| digits.starts_with(|first: char| first.is_ascii_digit()))
            .unwrap_or(constant);
        // GDK answers an unknown name with VoidSymbol, so its own name reads as no key.
        let gdk = if name == "VoidSymbol" {
            Some(gdk::Key::VoidSymbol)
        } else {
            gdk::Key::from_name(name)
        };
        assert_eq!(gdk.map(IntoGlib::into_glib), Some(*keysym), "{constant}");
    }
}
