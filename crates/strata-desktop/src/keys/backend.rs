// SPDX-License-Identifier: MIT

//! What a key's name, character and case are, and how an accelerator string reads and
//! writes, asked of whichever library the running interface has.
//!
//! The GTK application installs a backend that asks GDK and GTK, so everything it reads,
//! writes, labels and matches is what it always was. The Rune interface installs one that
//! asks `xkbcommon` and corrects it to GDK's answers. The seam is process-wide, set once at
//! start-up, and exists only while there are two interfaces: it goes when GTK does.
//!
//! One more answer is installed with it, because it too is an interface's own: which
//! bindings never fire there ([`NeverFires`]).

use std::sync::OnceLock;

use super::{Binding, Command, Key, Map, Modifiers};

/// The answers of one key library. Every method is a pure function of its arguments.
pub trait Backend: Sync {
    /// The name of `key`, as `gdk_keyval_name` gives it: what a saved keymap holds and what a
    /// label falls back to. `None` for a value that is not a key.
    fn key_name(&self, key: Key) -> Option<String>;
    /// The key `name` stands for.
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
/// first call decides; a later one is ignored, so a test binary whose tests each install the
/// same pair is fine.
pub fn install(backend: &'static dyn Backend, never_fires: NeverFires) {
    INSTALLED.get_or_init(|| Installed {
        backend,
        never_fires,
    });
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
