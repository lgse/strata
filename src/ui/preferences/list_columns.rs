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
    Duration,
    Dimensions,
    Created,
}

pub(crate) struct ListColumnDefinition {
    stored_id: &'static str,
    pub title: &'static str,
    pub css_class: &'static str,
    pub sort_key: Option<SortKey>,
    pub width: i32,
    pub min_width: i32,
    pub optional: bool,
}

impl ListColumnId {
    pub const ALL: [Self; 8] = [
        Self::Name,
        Self::Mode,
        Self::Size,
        Self::Type,
        Self::Modified,
        Self::Duration,
        Self::Dimensions,
        Self::Created,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub(crate) fn definition(self) -> ListColumnDefinition {
        let (stored_id, title, css_class, sort_key, width, min_width, optional) = match self {
            Self::Name => (
                "name",
                "Name",
                "list-col-name",
                Some(SortKey::Name),
                160,
                160,
                false,
            ),
            Self::Mode => ("mode", "Mode", "list-col-mode", None, 160, 80, false),
            Self::Size => (
                "size",
                "Size",
                "list-col-size",
                Some(SortKey::Size),
                90,
                70,
                false,
            ),
            Self::Type => (
                "type",
                "Type",
                "list-col-type",
                Some(SortKey::Type),
                120,
                80,
                false,
            ),
            Self::Modified => (
                "modified",
                "Modified",
                "list-col-modified",
                Some(SortKey::Modified),
                150,
                110,
                false,
            ),
            Self::Duration => (
                "duration",
                "Duration",
                "list-col-duration",
                Some(SortKey::Duration),
                105,
                80,
                true,
            ),
            Self::Dimensions => (
                "dimensions",
                "Dimensions",
                "list-col-dimensions",
                Some(SortKey::Dimensions),
                130,
                105,
                true,
            ),
            Self::Created => (
                "created",
                "Created",
                "list-col-created",
                Some(SortKey::Created),
                150,
                110,
                true,
            ),
        };
        ListColumnDefinition {
            stored_id,
            title,
            css_class,
            sort_key,
            width,
            min_width,
            optional,
        }
    }

    pub fn title(self) -> &'static str {
        self.definition().title
    }

    pub fn css_class(self) -> &'static str {
        self.definition().css_class
    }

    fn stored_name(self) -> &'static str {
        self.definition().stored_id
    }
}

pub fn default_list_column_order() -> Vec<ListColumnId> {
    ListColumnId::ALL.to_vec()
}

pub(super) fn is_default_list_column_order(order: &[ListColumnId]) -> bool {
    order == ListColumnId::ALL
}

pub(crate) fn sanitize_list_column_order(order: &[ListColumnId]) -> Vec<ListColumnId> {
    let mut seen = std::collections::HashSet::new();
    order
        .iter()
        .copied()
        .chain(ListColumnId::ALL)
        .filter(|col| seen.insert(*col))
        .collect()
}
pub(crate) fn sanitize_list_hidden_columns(hidden: &[ListColumnId]) -> Vec<ListColumnId> {
    let mut seen = std::collections::HashSet::new();
    hidden
        .iter()
        .copied()
        .filter(|col| *col != ListColumnId::Name && seen.insert(*col))
        .collect()
}
/// Unscaled pixels; an unset Name lets it absorb the remaining space.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct ListColumns(std::collections::BTreeMap<String, i32>);

impl ListColumns {
    pub fn width(&self, column: ListColumnId) -> Option<i32> {
        self.0.get(column.stored_name()).copied()
    }

    pub(super) fn is_valid(&self) -> bool {
        self.0.values().all(|width| *width > 0)
    }
}

impl FromIterator<(ListColumnId, i32)> for ListColumns {
    fn from_iter<T: IntoIterator<Item = (ListColumnId, i32)>>(iter: T) -> Self {
        Self(
            iter.into_iter()
                .map(|(column, width)| (column.stored_name().to_owned(), width))
                .collect(),
        )
    }
}
