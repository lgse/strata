// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{At, CaretMatch, Names, Predicate, Target, TextMatch, yes};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LargestImage {
    pub(crate) within: Target,
    #[serde(default)]
    pub(crate) wider_than: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) size: Option<[i64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) exceeds: Option<[i64; 2]>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChildrenCheck {
    pub(crate) comm_prefix: String,
    pub(crate) count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SectionsCheck {
    pub(crate) within: Target,
    pub(crate) section: String,
    pub(crate) names: Names,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PanesWith {
    pub(crate) target: Target,
    pub(crate) names: Names,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NameCheck {
    pub(crate) target: Target,
    pub(crate) text: TextMatch,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub(crate) rendered: bool,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_true(value: &bool) -> bool {
    *value
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PixelCheck {
    pub(crate) target: Target,
    pub(crate) rgb: [u8; 3],
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AccentPixels {
    pub(crate) target: Target,
    #[serde(default)]
    pub(crate) inset: u16,
    pub(crate) more_than: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HitPath {
    pub(crate) mark: String,
    pub(crate) hits: std::collections::BTreeMap<String, String>,
    pub(crate) name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Replaced {
    pub(crate) target: Target,
    pub(crate) since: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DescriptionCheck {
    pub(crate) target: Target,
    pub(crate) text: TextMatch,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MentionCheck {
    pub(crate) target: Target,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) words: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) equals: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileCheck {
    pub(crate) path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) exists: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) mode_bits: Option<ModeBits>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModeBits {
    pub(crate) mask: String,
    pub(crate) set: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SomeWindow {
    pub(crate) title: String,
    pub(crate) holds: Box<Predicate>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WindowCount {
    pub(crate) title: String,
    pub(crate) count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChildNames {
    pub(crate) of: Target,
    pub(crate) role: String,
    pub(crate) names: Names,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SubtreeText {
    pub(crate) target: Target,
    pub(crate) contains: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) strip_newlines: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InsideCheck {
    pub(crate) target: Target,
    pub(crate) of: Target,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) axis: Option<PointAxis>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EditorWindowCheck {
    pub(crate) of: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) pane: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScrollCheck {
    pub(crate) at_least: serde_json::Number,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BandCheck {
    pub(crate) from: String,
    pub(crate) to: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Resized {
    pub(crate) target: Target,
    pub(crate) since: String,
    pub(crate) width: Change,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub(crate) enum Change {
    Exactly(i64),
    Bounded {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<i64>,
    },
}

impl Change {
    pub(crate) fn matches(&self, change: i64) -> bool {
        match self {
            Self::Exactly(expected) => *expected == change,
            Self::Bounded { min, max } => {
                min.is_none_or(|min| change >= min) && max.is_none_or(|max| change <= max)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContainsPoint {
    pub(crate) target: Target,
    pub(crate) point: Target,
    pub(crate) axis: PointAxis,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PointAxis {
    X,
    Y,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FooterFill {
    pub(crate) ends: Vec<String>,
    pub(crate) total: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Repainted {
    pub(crate) since: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContrastBelow {
    pub(crate) of: String,
    pub(crate) to: String,
    pub(crate) share: serde_json::Number,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Elapsed {
    pub(crate) since: String,
    pub(crate) ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FieldRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) of: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FieldCheck {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) of: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) text: Option<TextMatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) caret: Option<CaretMatch>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub(crate) selection: Option<Option<[i32; 2]>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) focused: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) showing: Option<bool>,
}

impl FieldCheck {
    pub(crate) fn field(&self) -> FieldRef {
        FieldRef {
            of: self.of.clone(),
            name: self.name.clone(),
        }
    }
}

/// Tells an explicit null apart from a missing key.
fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PrefCheck {
    pub(crate) key: String,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub(crate) value: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) contains: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) one_of: Option<Vec<Value>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StateCheck {
    pub(crate) target: Target,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) selected: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) focused: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) checked: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) pressed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) sensitive: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) selectable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) editable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) focusable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) busy: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) marked: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Movement {
    pub(crate) target: Target,
    pub(crate) direction: Direction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) since: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) y_within: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) within: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BlankCheck {
    pub(crate) target: Target,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) at: Option<At>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Direction {
    Up,
    Down,
    Left,
    Right,
    None,
}
