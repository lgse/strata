// SPDX-License-Identifier: MIT

use strata_desktop::keys::{self, Command, CommandKind, Group, Map, bindings::*, conflicts};

use crate::shortcuts::command_sections;

const SEARCH_AND_TOOLS: &str = "Search and tools";
const TENXER_SECTION: &str = "10xer mode";

/// Commands no reference row names.
const FALLBACK_SECTIONS: &[(Command, &str)] = &[
    (PDF_ZOOM_RESET, "Preview"),
    (PREVIOUS_MATCH, "Preview"),
    (REFERENCE_CATEGORIES, SEARCH_AND_TOOLS),
    (REFERENCE_SEARCH, SEARCH_AND_TOOLS),
    (REFERENCE_LIST, SEARCH_AND_TOOLS),
    (FILTER_SLASH, SEARCH_AND_TOOLS),
];

type Sections = Vec<(&'static str, Vec<Command>)>;

pub(super) fn sections(map: Map) -> Sections {
    let tenxer = command_sections(Map::Tenxer);
    let default = command_sections(Map::Default);
    let mut order: Vec<&'static str> = Vec::new();
    for (title, _) in tenxer.iter().chain(&default) {
        push_new(&mut order, title);
    }
    let (own, others) = match map {
        Map::Default => (&default, &tenxer),
        Map::Tenxer => (&tenxer, &default),
    };
    let commands = runnable(map);
    let filed = Filing { map, own, others };
    for (command, _) in FALLBACK_SECTIONS {
        push_new(&mut order, filed.section_of(*command));
    }
    push_new(&mut order, SEARCH_AND_TOOLS);
    order
        .into_iter()
        .map(|title| (title, filed.listed_under(title, &commands)))
        .filter(|(_, listed)| !listed.is_empty())
        .collect()
}

/// Adds `title` to the order of the sections once.
fn push_new(order: &mut Vec<&'static str>, title: &'static str) {
    if !order.contains(&title) {
        order.push(title);
    }
}

fn runnable(map: Map) -> Vec<Command> {
    keys::registry()
        .filter(|command| {
            (map == Map::Tenxer || !matches!(command.kind(), CommandKind::Trigger(_)))
                && !conflicts::no_path(*command, map)
        })
        .collect()
}

struct Filing<'a> {
    map: Map,
    own: &'a Sections,
    others: &'a Sections,
}

impl Filing<'_> {
    fn listing(&self) -> impl Iterator<Item = &(&'static str, Vec<Command>)> {
        [self.own, self.others].into_iter().flatten()
    }

    fn section_of(&self, command: Command) -> &'static str {
        let title = self
            .listing()
            .find(|(_, listed)| listed.contains(&command))
            .map(|(title, _)| *title)
            .or_else(|| {
                FALLBACK_SECTIONS
                    .iter()
                    .find(|(listed, _)| *listed == command)
                    .map(|(_, title)| *title)
            })
            .unwrap_or(SEARCH_AND_TOOLS);
        if self.map == Map::Default && title == TENXER_SECTION {
            SEARCH_AND_TOOLS
        } else {
            title
        }
    }

    fn listed_under(&self, title: &'static str, commands: &[Command]) -> Vec<Command> {
        let mut listed: Vec<Command> = Vec::new();
        let candidates = self
            .listing()
            .filter(|(section, _)| *section == title)
            .flat_map(|(_, listed)| listed.iter().copied())
            .chain(commands.iter().copied());
        for command in candidates {
            if self.files_under(command, title, commands) && !listed.contains(&command) {
                listed.push(command);
            }
        }
        listed
    }

    /// Whether the table lists `command` under `title`.
    fn files_under(&self, command: Command, title: &str, commands: &[Command]) -> bool {
        commands.contains(&command) && self.section_of(command) == title
    }
}

/// The chord command that starts `group`'s steps.
pub(super) fn trigger(group: Group) -> Option<Command> {
    keys::registry().find(|command| command.kind() == CommandKind::Trigger(group))
}
