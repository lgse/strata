// SPDX-License-Identifier: MIT

//! The key library of the GTK application: GDK's names, characters and case, and GTK's
//! accelerator strings, for the shared catalog in `strata_desktop::keys`. Each method is one
//! call, so what the application reads, writes, labels and matches is what GDK says. It goes
//! with GTK.

use gtk::gdk;
use strata_desktop::keys::{Backend, Key, Modifiers};

use super::{conflicts, gdk_key, gdk_modifiers, key_from_gdk, modifiers_from_gdk};

struct Gdk;

impl Backend for Gdk {
    fn key_name(&self, key: Key) -> Option<String> {
        gdk_key(key).name().map(String::from)
    }

    fn key_from_name(&self, name: &str) -> Option<Key> {
        gdk::Key::from_name(name).map(key_from_gdk)
    }

    fn key_to_unicode(&self, key: Key) -> Option<char> {
        gdk_key(key).to_unicode()
    }

    fn key_from_unicode(&self, character: char) -> Key {
        Key::from_keysym(gdk::unicode_to_keyval(u32::from(character)))
    }

    fn key_to_lower(&self, key: Key) -> Key {
        key_from_gdk(gdk_key(key).to_lower())
    }

    fn key_to_upper(&self, key: Key) -> Key {
        key_from_gdk(gdk_key(key).to_upper())
    }

    fn accelerator_parse(&self, accelerator: &str) -> Option<(Key, Modifiers)> {
        gtk::accelerator_parse(accelerator)
            .map(|(key, modifiers)| (key_from_gdk(key), modifiers_from_gdk(modifiers)))
    }

    fn accelerator_name(&self, key: Key, modifiers: Modifiers) -> String {
        gtk::accelerator_name(gdk_key(key), gdk_modifiers(modifiers)).into()
    }
}

/// Installs GDK as the key library of this process, with the GTK stages' answer to which
/// bindings never fire, and says whether this call was the one that installed. `main` calls
/// it before anything reads a key.
pub(crate) fn install() -> bool {
    strata_desktop::keys::install(&Gdk, conflicts::never_fires)
}

/// A `#[test]` has no start-up to install from, and a test that forgot would fail only for
/// what it happens to read. So the test binary installs as it loads, before any test runs.
#[cfg(test)]
#[used]
#[expect(
    unsafe_code,
    reason = "Rust has no hook that runs before a test binary's tests"
)]
// SAFETY: the loader calls every `.init_array` entry once with the C ABI before `main`, and
// this one only sets a `OnceLock`.
#[unsafe(link_section = ".init_array")]
static INSTALL_FOR_TESTS: extern "C" fn() = {
    extern "C" fn install_for_tests() {
        install();
    }
    install_for_tests
};
