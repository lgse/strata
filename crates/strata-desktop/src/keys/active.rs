// SPDX-License-Identifier: MIT

//! What the catalog reads of the live settings: which map is in use, the keymap with the
//! user's overrides, and whether plain keys type into a search.
//!
//! The catalog reads them at each press and each label, from whoever the interface
//! registered on its thread. Nothing registered, or a source that has gone, means the
//! shipped defaults. The source is only read: it is never called back from.

use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

use super::{Keymap, Map};

/// The live settings the catalog resolves through.
pub trait Active {
    fn tenxer_mode(&self) -> bool;
    /// Whether plain printable keys go to type-to-search: the setting is on and 10xer is off.
    fn type_to_search_active(&self) -> bool;
    /// The tables with the saved overrides applied.
    fn keymap(&self) -> Rc<Keymap>;
}

/// The three answers as values, for an interface that resolves them itself.
pub struct Fixed {
    pub map: Map,
    pub type_to_search: bool,
    pub keymap: Rc<Keymap>,
}

impl Active for Fixed {
    fn tenxer_mode(&self) -> bool {
        self.map == Map::Tenxer
    }

    fn type_to_search_active(&self) -> bool {
        self.type_to_search
    }

    fn keymap(&self) -> Rc<Keymap> {
        self.keymap.clone()
    }
}

thread_local! {
    static ACTIVE: RefCell<Option<Weak<dyn Active>>> = const { RefCell::new(None) };
}

/// Makes `source` what this thread's presses and labels resolve through, for as long as
/// something else keeps it alive.
pub fn set_active(source: Weak<dyn Active>) {
    ACTIVE.with(|active| active.replace(Some(source)));
}

/// The registered source while it lives.
pub(super) fn active() -> Option<Rc<dyn Active>> {
    ACTIVE.with(|active| active.borrow().as_ref().and_then(Weak::upgrade))
}
