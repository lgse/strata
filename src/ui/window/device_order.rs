// SPDX-License-Identifier: MIT

//! Keeps each drive's rows together in the sidebar Devices section, whatever
//! order the volume monitor reports them in.

use std::cmp::Ordering;

use crate::app::compare_display_names;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DriveRank {
    /// The backend's own ranking: udisks puts fixed disks before removable and
    /// hot-plugged ones.
    pub sort_key: Option<String>,
    pub name: String,
    /// Keeps two drives of the same model apart.
    pub identity: Option<String>,
}

impl DriveRank {
    fn compare(&self, other: &Self) -> Ordering {
        (self.sort_key.is_none(), &self.sort_key)
            .cmp(&(other.sort_key.is_none(), &other.sort_key))
            .then_with(|| compare_names(&self.name, &other.name))
            .then_with(|| compare_identities(self.identity.as_deref(), other.identity.as_deref()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum DeviceHome {
    Drive(DriveRank),
    NoDrive,
    Network,
}

impl DeviceHome {
    fn compare(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Drive(left), Self::Drive(right)) => left.compare(right),
            _ => self.position().cmp(&other.position()),
        }
    }

    fn position(&self) -> u8 {
        match self {
            Self::Drive(_) => 0,
            Self::NoDrive => 1,
            Self::Network => 2,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct DevicePlacement {
    pub home: DeviceHome,
    pub name: String,
    /// Keeps volumes that share a name, such as "129 GB Volume", from swapping
    /// places between rebuilds.
    pub identity: Option<String>,
}

impl DevicePlacement {
    fn compare(&self, other: &Self) -> Ordering {
        self.home
            .compare(&other.home)
            .then_with(|| compare_names(&self.name, &other.name))
            .then_with(|| compare_identities(self.identity.as_deref(), other.identity.as_deref()))
    }
}

/// Returns one list per drive in display order, followed by rows with no drive
/// and then network shares.
pub(super) fn group_by_drive<T>(mut rows: Vec<(DevicePlacement, T)>) -> Vec<Vec<T>> {
    rows.sort_by(|(left, _), (right, _)| left.compare(right));
    let mut groups: Vec<(DeviceHome, Vec<T>)> = Vec::new();
    for (placement, row) in rows {
        match groups.last_mut() {
            Some((home, group)) if *home == placement.home => group.push(row),
            _ => groups.push((placement.home, vec![row])),
        }
    }
    groups.into_iter().map(|(_, group)| group).collect()
}

fn compare_names(left: &str, right: &str) -> Ordering {
    compare_display_names(left, right).then_with(|| left.cmp(right))
}

fn compare_identities(left: Option<&str>, right: Option<&str>) -> Ordering {
    compare_names(left.unwrap_or_default(), right.unwrap_or_default())
}

#[cfg(test)]
mod tests;
