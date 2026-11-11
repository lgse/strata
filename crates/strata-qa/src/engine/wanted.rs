// SPDX-License-Identifier: MIT

use super::lookup::{missing, presented, rendered};
use crate::{Backend, LookupError, Orientation, StateName, Target};

/// A role target's criteria, with its name pattern compiled.
pub struct Wanted<'a> {
    pub role: &'a str,
    pub other_roles: &'a [String],
    pub name: Option<&'a str>,
    pub description: Option<&'a str>,
    pub name_ends_with: &'a [String],
    pub nth: Option<i64>,
    pub states: &'a [StateName],
    pub name_matches: Option<regex::Regex>,
    pub has: Option<Box<Wanted<'a>>>,
}

fn pattern(pattern: Option<&str>) -> Result<Option<regex::Regex>, LookupError> {
    pattern
        .map(regex::Regex::new)
        .transpose()
        .map_err(|error| LookupError::Broken(error.to_string()))
}

pub fn has_state<B: Backend + ?Sized>(backend: &B, node: &B::Node, state: StateName) -> bool {
    let now = backend.states(node);
    // Read for every state and not only the two that use it: a reply reports the criticals
    // GTK logs when some widgets are asked, so reading it less often could change a reply.
    let orientation = backend.orientation(node);
    match state {
        StateName::Selected => now.selected,
        StateName::Focused => now.focused,
        StateName::Checked => now.checked,
        StateName::Pressed => now.pressed,
        StateName::Sensitive => now.sensitive,
        StateName::Selectable => now.selectable,
        StateName::Editable => now.editable,
        StateName::Busy => now.busy,
        StateName::Horizontal => orientation == Some(Orientation::Horizontal),
        StateName::Vertical => orientation == Some(Orientation::Vertical),
    }
}

impl<'a> Wanted<'a> {
    pub fn of(target: &'a Target) -> Result<Self, LookupError> {
        let Target::Role {
            role,
            other_roles,
            name,
            description,
            name_ends_with,
            nth,
            states,
            name_matches,
            has,
            ..
        } = target
        else {
            return Err(LookupError::Broken(format!(
                "{} is not a role target",
                target.to_json()
            )));
        };
        Ok(Self {
            role,
            other_roles,
            name: name.as_deref(),
            description: description.as_deref(),
            name_ends_with,
            nth: *nth,
            states,
            name_matches: pattern(name_matches.as_deref())?,
            has: has
                .as_deref()
                .map(|has| Self::of(has).map(Box::new))
                .transpose()?,
        })
    }

    pub fn matches<B: Backend + ?Sized>(&self, backend: &B, node: &B::Node) -> bool {
        let role = backend.role(node);
        (self.role == "*"
            || role == self.role
            || self.other_roles.iter().any(|other| other == role))
            && self.has.as_ref().is_none_or(|has| {
                presented(backend, node)
                    .iter()
                    .any(|inner| has.matches(backend, inner) && rendered(backend, inner))
            })
            && self.name.is_none_or(|name| backend.name(node) == name)
            && self
                .name_matches
                .as_ref()
                .is_none_or(|pattern| pattern.is_match(&backend.name(node)))
            && self
                .states
                .iter()
                .all(|state| has_state(backend, node, *state))
            && (self.name_ends_with.is_empty() || {
                let name = backend.name(node);
                self.name_ends_with
                    .iter()
                    .any(|suffix| name.ends_with(suffix.as_str()))
            })
            && self
                .description
                .is_none_or(|description| backend.description(node) == description)
    }

    /// The drawn match the target's `nth` picks.
    pub fn find<B: Backend + ?Sized>(
        &self,
        backend: &B,
        root: &B::Node,
    ) -> Result<B::Node, LookupError> {
        let found = presented(backend, root)
            .into_iter()
            .filter(|node| self.matches(backend, node) && rendered(backend, node));
        pick(found.collect(), self.nth.unwrap_or(0)).ok_or_else(|| {
            missing(format!(
                "no {}{}{}{}{} on screen",
                self.nth
                    .map(|nth| format!("match {nth} of "))
                    .unwrap_or_default(),
                self.role,
                self.name
                    .map(|name| format!(" named {name:?}"))
                    .unwrap_or_default(),
                if self.name_ends_with.is_empty() {
                    String::new()
                } else {
                    format!(" named with an ending in {:?}", self.name_ends_with)
                },
                self.description
                    .map(|description| format!(" described {description:?}"))
                    .unwrap_or_default()
            ))
        })
    }

    /// The match the target's `nth` picks, drawn or not.
    pub fn find_presented<B: Backend + ?Sized>(
        &self,
        backend: &B,
        root: &B::Node,
    ) -> Option<B::Node> {
        let found = presented(backend, root)
            .into_iter()
            .filter(|node| self.matches(backend, node));
        pick(found.collect(), self.nth.unwrap_or(0))
    }
}

/// A negative index counts from the end.
pub(crate) fn pick<Item>(mut items: Vec<Item>, index: i64) -> Option<Item> {
    let length = i64::try_from(items.len()).ok()?;
    let index = if index < 0 { length + index } else { index };
    let index = usize::try_from(index).ok()?;
    (index < items.len()).then(|| items.swap_remove(index))
}
