// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests;

use rustix::fs::FileType;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IdentityTraits {
    pub numbers: InodeNumbers,
    /// Whether a folder keeps its birth time when an entry is renamed into or within it.
    pub folder_births: bool,
}

impl Default for IdentityTraits {
    fn default() -> Self {
        Self {
            numbers: InodeNumbers::Unknown,
            folder_births: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InodeNumbers {
    Kept,
    /// Numbered afresh after eviction, or kept by a name for whatever a rename puts there.
    Renewed,
    /// Compared as kept, but no removal rests on an equal number alone.
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Proof {
    Same,
    Different,
    Unproven,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Attributes {
    pub size: u64,
    pub mtime: (i64, u32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Evidence {
    pub device: u64,
    pub inode: u64,
    pub kind: FileType,
    pub birth: Option<(i64, u32)>,
    /// A file's or link's; a folder's own size and times change with its entries.
    pub attributes: Option<Attributes>,
}

impl Evidence {
    pub fn compare(&self, traits: IdentityTraits, current: &Self) -> Proof {
        if current.device != self.device || current.kind != self.kind {
            return Proof::Different;
        }
        let folder = self.kind == FileType::Directory;
        // FUSE daemons can report a birth time of zero for none.
        let reported = |birth: Option<(i64, u32)>| birth.filter(|&birth| birth != (0, 0));
        let births = reported(self.birth)
            .zip(reported(current.birth))
            .filter(|_| !folder || traits.folder_births);
        if births.is_some_and(|(recorded, now)| recorded != now) {
            return Proof::Different;
        }
        if traits.numbers != InodeNumbers::Renewed {
            return if current.inode == self.inode {
                Proof::Same
            } else {
                Proof::Different
            };
        }
        if births.is_some() {
            return Proof::Same;
        }
        match self.attributes.zip(current.attributes).filter(|_| !folder) {
            Some((recorded, now)) if recorded == now => Proof::Same,
            Some(_) => Proof::Different,
            None => Proof::Unproven,
        }
    }
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "Trash, navigation and load stamps record it")
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ItemIdentity {
    pub evidence: Evidence,
    pub traits: IdentityTraits,
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "Trash, navigation and load stamps compare it")
)]
impl ItemIdentity {
    pub fn same(&self, other: &Self) -> bool {
        let numbers = match (self.traits.numbers, other.traits.numbers) {
            (InodeNumbers::Kept, InodeNumbers::Kept) => InodeNumbers::Kept,
            (InodeNumbers::Renewed, _) | (_, InodeNumbers::Renewed) => InodeNumbers::Renewed,
            _ => InodeNumbers::Unknown,
        };
        let traits = IdentityTraits {
            numbers,
            folder_births: self.traits.folder_births && other.traits.folder_births,
        };
        self.evidence.compare(traits, &other.evidence) == Proof::Same
    }
}
