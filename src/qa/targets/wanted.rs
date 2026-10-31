// SPDX-License-Identifier: MIT

use gtk::prelude::*;

use super::{
    super::{
        protocol::{StateName, Target},
        state,
    },
    LookupError, accessible_description, accessible_name, missing, presented, role_name, states,
};

pub(crate) struct Wanted<'a> {
    pub(crate) role: &'a str,
    pub(crate) other_roles: &'a [String],
    pub(crate) name: Option<&'a str>,
    pub(crate) description: Option<&'a str>,
    pub(crate) name_ends_with: &'a [String],
    pub(crate) nth: Option<i64>,
    pub(crate) states: &'a [StateName],
    pub(crate) name_matches: Option<regex::Regex>,
    pub(crate) has: Option<Box<Wanted<'a>>>,
}

pub(crate) fn pattern(pattern: Option<&str>) -> Result<Option<regex::Regex>, LookupError> {
    pattern
        .map(regex::Regex::new)
        .transpose()
        .map_err(|error| LookupError::Broken(error.to_string()))
}

pub(crate) fn has_state(widget: &gtk::Widget, state: StateName) -> bool {
    let now = states(widget);
    let orientation = widget
        .dynamic_cast_ref::<gtk::Orientable>()
        .map(OrientableExt::orientation);
    match state {
        StateName::Selected => now.selected,
        StateName::Focused => now.focused,
        StateName::Checked => now.checked,
        StateName::Pressed => now.pressed,
        StateName::Sensitive => now.sensitive,
        StateName::Selectable => now.selectable,
        StateName::Editable => now.editable,
        StateName::Busy => now.busy,
        StateName::Horizontal => orientation == Some(gtk::Orientation::Horizontal),
        StateName::Vertical => orientation == Some(gtk::Orientation::Vertical),
    }
}

impl<'a> Wanted<'a> {
    pub(crate) fn of(target: &'a Target) -> Result<Self, LookupError> {
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

    pub(crate) fn matches(&self, widget: &gtk::Widget) -> bool {
        let role = role_name(widget);
        (self.role == "*"
            || role == self.role
            || self.other_roles.iter().any(|other| other == role))
            && self.has.as_ref().is_none_or(|has| {
                presented(widget)
                    .iter()
                    .any(|inner| has.matches(inner) && state::rendered_bounds(inner).is_some())
            })
            && self.name.is_none_or(|name| accessible_name(widget) == name)
            && self
                .name_matches
                .as_ref()
                .is_none_or(|pattern| pattern.is_match(&accessible_name(widget)))
            && self.states.iter().all(|state| has_state(widget, *state))
            && (self.name_ends_with.is_empty() || {
                let name = accessible_name(widget);
                self.name_ends_with
                    .iter()
                    .any(|suffix| name.ends_with(suffix.as_str()))
            })
            && self
                .description
                .is_none_or(|description| accessible_description(widget) == description)
    }

    pub(crate) fn find(&self, root: &gtk::Widget) -> Result<gtk::Widget, LookupError> {
        let found = presented(root)
            .into_iter()
            .filter(|widget| self.matches(widget) && state::rendered_bounds(widget).is_some());
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

    pub(crate) fn find_presented(&self, root: &gtk::Widget) -> Option<gtk::Widget> {
        let found = presented(root)
            .into_iter()
            .filter(|widget| self.matches(widget));
        pick(found.collect(), self.nth.unwrap_or(0))
    }
}

pub(super) fn pick(mut items: Vec<gtk::Widget>, index: i64) -> Option<gtk::Widget> {
    let length = i64::try_from(items.len()).ok()?;
    let index = if index < 0 { length + index } else { index };
    let index = usize::try_from(index).ok()?;
    (index < items.len()).then(|| items.swap_remove(index))
}
