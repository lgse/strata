// SPDX-License-Identifier: MIT

//! Why a value is a default, or the last good one: reported as data, each kind once.

use std::collections::HashSet;
use std::mem::discriminant;

use super::keys::Key;

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Fallback {
    NoBus(String),
    NoPortal(String),
    /// The portal's answer lacks this key, so it has its default and isn't known.
    Unset(Key),
    Invalid {
        key: Key,
        value: String,
    },
    /// The portal left the bus. The values it gave are kept.
    PortalLost,
}

impl Fallback {
    fn identity(&self) -> (std::mem::Discriminant<Fallback>, Option<Key>) {
        let key = match self {
            Fallback::Unset(key) | Fallback::Invalid { key, .. } => Some(*key),
            Fallback::NoBus(_) | Fallback::NoPortal(_) | Fallback::PortalLost => None,
        };
        (discriminant(self), key)
    }
}

#[derive(Default)]
pub(super) struct Reported(HashSet<(std::mem::Discriminant<Fallback>, Option<Key>)>);

impl Reported {
    /// True the first time `fallback`'s kind (and key) is seen.
    pub(super) fn first(&mut self, fallback: &Fallback) -> bool {
        self.0.insert(fallback.identity())
    }
}
