// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Presence {
    Absent,
    Any,
    Named(String),
    OneOf(Vec<String>),
}

impl Presence {
    pub(crate) fn matches(&self, actual: Option<&str>) -> bool {
        match self {
            Self::Absent => actual.is_none(),
            Self::Any => actual.is_some(),
            Self::Named(name) => actual == Some(name.as_str()),
            Self::OneOf(names) => {
                actual.is_some_and(|actual| names.iter().any(|name| name == actual))
            }
        }
    }
}

impl<'de> Deserialize<'de> for Presence {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct OneOf {
            one_of: Vec<String>,
        }
        match Value::deserialize(deserializer)? {
            Value::Null => Ok(Self::Absent),
            Value::Bool(true) => Ok(Self::Any),
            Value::String(name) => Ok(Self::Named(name)),
            Value::Object(object) => serde_json::from_value::<OneOf>(Value::Object(object))
                .map(|names| Self::OneOf(names.one_of))
                .map_err(serde::de::Error::custom),
            other => Err(serde::de::Error::custom(format!(
                "expected a name, true for any, null, or {{\"one_of\":[...]}}, not {other}"
            ))),
        }
    }
}

impl Serialize for Presence {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Absent => serializer.serialize_none(),
            Self::Any => serializer.serialize_bool(true),
            Self::Named(name) => serializer.serialize_str(name),
            Self::OneOf(names) => json!({ "one_of": names }).serialize(serializer),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub(crate) enum TextMatch {
    Exactly(String),
    Like(TextPattern),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TextPattern {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) starts_with: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) ends_with: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) contains: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) lowercase_contains: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) longer_than: Option<usize>,
}

impl TextMatch {
    pub(crate) fn matches(&self, actual: &str) -> bool {
        match self {
            Self::Exactly(text) => text == actual,
            Self::Like(pattern) => {
                pattern
                    .starts_with
                    .as_deref()
                    .is_none_or(|start| actual.starts_with(start))
                    && pattern
                        .ends_with
                        .as_deref()
                        .is_none_or(|end| actual.ends_with(end))
                    && pattern
                        .contains
                        .as_deref()
                        .is_none_or(|part| actual.contains(part))
                    && pattern
                        .lowercase_contains
                        .as_deref()
                        .is_none_or(|part| actual.to_lowercase().contains(part))
                    && pattern
                        .longer_than
                        .is_none_or(|length| actual.chars().count() > length)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub(crate) enum CaretMatch {
    Exactly(i32),
    Between { min: i32, max: i32 },
}

impl CaretMatch {
    pub(crate) fn matches(&self, actual: i32) -> bool {
        match self {
            Self::Exactly(offset) => *offset == actual,
            Self::Between { min, max } => (*min..=*max).contains(&actual),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub(crate) enum Names {
    Exactly(Vec<String>),
    Including(Box<NameFilter>),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NameFilter {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) contains: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) excludes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) at_least: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) ends_with: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) starts_with: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) first: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) first_above: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) any_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) toggled: Option<Toggled>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) in_order: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) set: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub(crate) occurrences: std::collections::BTreeMap<String, usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) sorted_casefold_from: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Toggled {
    pub(crate) names: Vec<String>,
    pub(crate) against: Vec<String>,
}

impl Names {
    pub(crate) fn matches(&self, actual: &[String]) -> bool {
        match self {
            Self::Exactly(expected) => expected == actual,
            Self::Including(filter) => {
                filter.contains.iter().all(|name| actual.contains(name))
                    && !filter.excludes.iter().any(|name| actual.contains(name))
                    && filter.count.is_none_or(|count| count == actual.len())
                    && filter.at_least.is_none_or(|least| actual.len() >= least)
                    && actual.ends_with(&filter.ends_with)
                    && actual.starts_with(&filter.starts_with)
                    && filter
                        .first
                        .as_ref()
                        .is_none_or(|first| actual.first() == Some(first))
                    && filter
                        .first_above
                        .as_ref()
                        .is_none_or(|bound| actual.first().is_some_and(|first| first > bound))
                    && filter
                        .any_from
                        .as_ref()
                        .is_none_or(|bound| actual.iter().any(|name| name >= bound))
                    && filter.toggled.as_ref().is_none_or(|toggled| {
                        toggled
                            .names
                            .iter()
                            .all(|name| actual.contains(name) != toggled.against.contains(name))
                    })
                    && in_order(actual, &filter.in_order)
                    && filter.set.as_ref().is_none_or(|set| {
                        actual.iter().all(|name| set.contains(name))
                            && set.iter().all(|name| actual.contains(name))
                    })
                    && filter.occurrences.iter().all(|(name, count)| {
                        actual.iter().filter(|listed| *listed == name).count() == *count
                    })
                    && filter.sorted_casefold_from.is_none_or(|from| {
                        let rest = actual.get(from..).unwrap_or_default();
                        let mut sorted = rest.to_vec();
                        sorted.sort_by_key(|name| name.to_lowercase());
                        sorted == rest
                    })
            }
        }
    }
}

fn in_order(actual: &[String], expected: &[String]) -> bool {
    let positions: Option<Vec<usize>> = expected
        .iter()
        .map(|name| actual.iter().position(|item| item == name))
        .collect();
    positions.is_some_and(|positions| positions.is_sorted())
}
