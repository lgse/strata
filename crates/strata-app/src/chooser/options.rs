// SPDX-License-Identifier: MIT

use super::source::CompiledFilter;
use crate::portal::{PortalChoice, PortalFileFilter};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ChoiceControl {
    Boolean {
        id: String,
        label: String,
        checked: bool,
    },
    Select {
        id: String,
        label: String,
        options: Vec<(String, String)>,
        selected: usize,
        open: bool,
    },
}

impl ChoiceControl {
    pub(crate) fn from_portal(choice: &PortalChoice) -> Self {
        if choice.pairs.is_empty() {
            Self::Boolean {
                id: choice.id.clone(),
                label: choice.label.clone(),
                checked: choice.initial_selection == "true",
            }
        } else {
            let selected = choice
                .pairs
                .iter()
                .position(|(value, _)| value == &choice.initial_selection)
                .unwrap_or(0);
            Self::Select {
                id: choice.id.clone(),
                label: choice.label.clone(),
                options: choice.pairs.clone(),
                selected,
                open: false,
            }
        }
    }

    pub(crate) fn value(&self) -> (String, String) {
        match self {
            Self::Boolean { id, checked, .. } => (id.clone(), checked.to_string()),
            Self::Select {
                id,
                options,
                selected,
                ..
            } => (
                id.clone(),
                options
                    .get(*selected)
                    .map_or_else(String::new, |(val, _)| val.clone()),
            ),
        }
    }

    pub(crate) fn is_open(&self) -> bool {
        matches!(self, Self::Select { open: true, .. })
    }

    /// A check box's click.
    pub(crate) fn toggle_checked(&mut self) {
        if let Self::Boolean { checked, .. } = self {
            *checked = !*checked;
        }
    }

    /// Opens or closes a select's list.
    pub(crate) fn set_open(&mut self, to: bool) {
        if let Self::Select { open, .. } = self {
            *open = to;
        }
    }

    /// A select's option labelled `chosen`, which closes its list.
    pub(crate) fn choose(&mut self, chosen: &str) {
        if let Self::Select {
            options,
            selected,
            open,
            ..
        } = self
        {
            if let Some(position) = options.iter().position(|(_, text)| text == chosen) {
                *selected = position;
            }
            *open = false;
        }
    }

    pub(crate) fn close(&mut self) -> bool {
        match self {
            Self::Select { open, .. } if *open => {
                *open = false;
                true
            }
            _ => false,
        }
    }
}

pub(crate) fn normalize_portal_filters(
    filters: &[PortalFileFilter],
    current: Option<&PortalFileFilter>,
) -> (Vec<CompiledFilter>, Option<usize>) {
    let mut all = filters.to_vec();
    if let Some(current) = current
        && !all.contains(current)
    {
        all.push(current.clone());
    }
    let selected = current
        .and_then(|current| all.iter().position(|filter| filter == current))
        .or_else(|| (!all.is_empty()).then_some(0));
    let compiled = all.into_iter().map(CompiledFilter::new).collect();
    (compiled, selected)
}

pub(crate) fn build_choices(choices: &[PortalChoice]) -> Vec<ChoiceControl> {
    choices.iter().map(ChoiceControl::from_portal).collect()
}
