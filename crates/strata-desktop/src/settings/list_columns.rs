// SPDX-License-Identifier: MIT

//! The list view's columns: their identities, saved order and hidden set, and widths.

use super::*;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ListColumnId {
    Name,
    Mode,
    Size,
    #[serde(rename = "type")]
    Type,
    Modified,
}

impl ListColumnId {
    pub const ALL: [Self; 5] = [
        Self::Name,
        Self::Mode,
        Self::Size,
        Self::Type,
        Self::Modified,
    ];

    pub fn index(self) -> usize {
        match self {
            Self::Name => 0,
            Self::Mode => 1,
            Self::Size => 2,
            Self::Type => 3,
            Self::Modified => 4,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Mode => "Mode",
            Self::Size => "Size",
            Self::Type => "Type",
            Self::Modified => "Modified",
        }
    }

    pub fn css_class(self) -> &'static str {
        match self {
            Self::Name => "list-col-name",
            Self::Mode => "list-col-mode",
            Self::Size => "list-col-size",
            Self::Type => "list-col-type",
            Self::Modified => "list-col-modified",
        }
    }
}

pub fn default_list_column_order() -> Vec<ListColumnId> {
    ListColumnId::ALL.to_vec()
}

pub(super) fn is_default_list_column_order(order: &[ListColumnId]) -> bool {
    order == ListColumnId::ALL
}

pub fn sanitize_list_column_order(order: &[ListColumnId]) -> Vec<ListColumnId> {
    let mut seen = std::collections::HashSet::new();
    order
        .iter()
        .copied()
        .chain(ListColumnId::ALL)
        .filter(|col| seen.insert(*col))
        .collect()
}
pub fn sanitize_list_hidden_columns(hidden: &[ListColumnId]) -> Vec<ListColumnId> {
    let mut seen = std::collections::HashSet::new();
    hidden
        .iter()
        .copied()
        .filter(|col| *col != ListColumnId::Name && seen.insert(*col))
        .collect()
}
/// Unscaled pixels; an unset `name` lets Name absorb the remaining space.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ListColumns {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<i32>,
    pub mode: i32,
    pub size: i32,
    #[serde(rename = "type")]
    pub kind: i32,
    pub modified: i32,
}

impl ListColumns {
    pub fn is_valid(&self) -> bool {
        self.name.is_none_or(|width| width > 0)
            && [self.mode, self.size, self.kind, self.modified]
                .iter()
                .all(|width| *width > 0)
    }
}
