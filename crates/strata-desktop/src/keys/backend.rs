// SPDX-License-Identifier: MIT

//! What a key's name, character and case are, and how an accelerator string reads and
//! writes, asked of whichever library the running interface has.
//!
//! The GTK application installs a backend that asks GDK and GTK, so everything it reads,
//! writes, labels and matches is what it always was. The Rune interface installs
//! [`Table`](super::Table), which answers for the keys Strata declares and needs no library;
//! a backend that asks `xkbcommon` for the rest, corrected to GDK's answers, is to sit on
//! it. The seam is process-wide, set once at start-up, and exists only while there are two
//! interfaces: it goes when GTK does.
//!
//! One more answer is installed with it, because it too is an interface's own: which
//! bindings never fire there ([`NeverFires`]).

use std::sync::OnceLock;

use super::{Binding, Command, Key, Map, Modifiers};

/// The answers of one key library. No method keeps state between calls, but a library may
/// need its toolkit: GTK's two accelerator calls want GTK initialized and its main thread.
/// So callers stay on the interface thread. The trait is `Sync` only because the installed
/// backend is one value for the process.
pub trait Backend: Sync {
    /// The name of `key`, as `gdk_keyval_name` gives it: what a saved keymap holds and what a
    /// label falls back to. `None` for a value that is not a key.
    fn key_name(&self, key: Key) -> Option<String>;
    /// The key `name` stands for. The GDK backend answers `None` for the name `VoidSymbol`:
    /// GDK gives that key for a name it does not know, and gdk4 reads it as no key. Another
    /// backend answers the same.
    fn key_from_name(&self, name: &str) -> Option<Key>;
    /// The character `key` types, if it types one.
    fn key_to_unicode(&self, key: Key) -> Option<char>;
    /// The key that types `character`.
    fn key_from_unicode(&self, character: char) -> Key;
    fn key_to_lower(&self, key: Key) -> Key;
    fn key_to_upper(&self, key: Key) -> Key;
    /// Reads an accelerator string, such as `<Control><Shift>a`, in the format of
    /// `gtk_accelerator_parse`. `None` for a string that is not one.
    fn accelerator_parse(&self, accelerator: &str) -> Option<(Key, Modifiers)>;
    /// Writes `key` with `modifiers` as an accelerator string, in the format of
    /// `gtk_accelerator_name`.
    fn accelerator_name(&self, key: Key, modifiers: Modifiers) -> String;
}

/// Whether a binding in a map's entry for a command never fires on this interface, so that
/// a label leaves it out. The last argument says whether plain printable keys are typing
/// into a search. An interface answers from its own key stages: the GTK application's
/// answer is `ui::keys::conflicts::never_fires`.
pub type NeverFires = fn(Map, Command, Binding, bool) -> bool;

struct Installed {
    backend: &'static dyn Backend,
    never_fires: NeverFires,
}

static INSTALLED: OnceLock<Installed> = OnceLock::new();

/// Makes `backend` the process's key library and `never_fires` its answer for labels. The
/// first call decides, and a later one is ignored. The answer is whether this call was the
/// one that installed: `false` means an earlier pair stays, which is fine for a caller that
/// installs the same pair again and a mistake for one that meant another.
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

/// The installed backend.
///
/// # Panics
///
/// When none was installed.
pub(super) fn backend() -> &'static dyn Backend {
    installed().backend
}

/// See [`NeverFires`].
pub(super) fn never_fires(map: Map, command: Command, binding: Binding, typing: bool) -> bool {
    (installed().never_fires)(map, command, binding, typing)
}
