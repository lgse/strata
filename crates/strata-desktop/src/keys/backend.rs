// SPDX-License-Identifier: MIT

use std::sync::OnceLock;

use super::{Binding, Command, Key, Map, Modifiers};

pub trait Backend: Sync {
    fn key_name(&self, key: Key) -> Option<String>;
    fn key_from_name(&self, name: &str) -> Option<Key>;
    /// The character `key` types, if it types one.
    fn key_to_unicode(&self, key: Key) -> Option<char>;
    /// The key that types `character`.
    fn key_from_unicode(&self, character: char) -> Key;
    fn key_to_lower(&self, key: Key) -> Key;
    fn key_to_upper(&self, key: Key) -> Key;
    fn accelerator_parse(&self, accelerator: &str) -> Option<(Key, Modifiers)>;
    fn accelerator_name(&self, key: Key, modifiers: Modifiers) -> String;
}

pub type NeverFires = fn(Map, Command, Binding, bool) -> bool;

struct Installed {
    backend: &'static dyn Backend,
    never_fires: NeverFires,
}

static INSTALLED: OnceLock<Installed> = OnceLock::new();

pub fn install(backend: &'static dyn Backend, never_fires: NeverFires) -> bool {
    let mut installed_now = false;
    INSTALLED.get_or_init(|| {
        installed_now = true;
        Installed {
            backend,
            never_fires,
        }
    });
    installed_now
}

fn installed() -> &'static Installed {
    INSTALLED
        .get()
        .expect("an interface installs its key backend at start-up, before any key is read")
}

pub(super) fn backend() -> &'static dyn Backend {
    installed().backend
}

/// See [`NeverFires`].
pub(super) fn never_fires(map: Map, command: Command, binding: Binding, typing: bool) -> bool {
    (installed().never_fires)(map, command, binding, typing)
}
