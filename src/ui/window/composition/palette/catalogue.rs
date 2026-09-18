// SPDX-License-Identifier: MIT

use crate::ui::{browser::palette::FileCommand, browser_modes::BrowserMode};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Command {
    Search,
    RecentFolders,
    Filter,
    Location,
    Terminal,
    Refresh,
    Settings,
    Shortcuts,
    View(BrowserMode),
    Hidden,
    Sidebar,
    File(FileCommand),
}

pub(super) struct CommandSpec {
    pub command: Command,
    pub group: &'static str,
    pub label: &'static str,
    pub aliases: &'static [&'static str],
    pub shortcut: &'static str,
}

pub(super) const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        command: Command::Search,
        group: "Find and navigate",
        label: "Search files and folders",
        aliases: &["find", "global search"],
        shortcut: "Ctrl+K",
    },
    CommandSpec {
        command: Command::RecentFolders,
        group: "Find and navigate",
        label: "Jump to a recent folder",
        aliases: &["history"],
        shortcut: "Ctrl+Shift+K",
    },
    CommandSpec {
        command: Command::Filter,
        group: "Find and navigate",
        label: "Filter current pane",
        aliases: &["find here"],
        shortcut: "Ctrl+F",
    },
    CommandSpec {
        command: Command::Location,
        group: "Find and navigate",
        label: "Edit location",
        aliases: &["go to folder", "address", "path"],
        shortcut: "Ctrl+L",
    },
    CommandSpec {
        command: Command::Terminal,
        group: "Tools",
        label: "Open terminal here",
        aliases: &["shell", "console"],
        shortcut: "Ctrl+T",
    },
    CommandSpec {
        command: Command::Refresh,
        group: "Tools",
        label: "Refresh",
        aliases: &["reload"],
        shortcut: "F5",
    },
    CommandSpec {
        command: Command::Settings,
        group: "Tools",
        label: "Open Settings",
        aliases: &["preferences", "configuration"],
        shortcut: "Ctrl+,",
    },
    CommandSpec {
        command: Command::Shortcuts,
        group: "Tools",
        label: "Show keyboard shortcuts",
        aliases: &["keybindings", "help"],
        shortcut: "F1",
    },
    CommandSpec {
        command: Command::View(BrowserMode::Columns),
        group: "View",
        label: "Switch to Columns",
        aliases: &["column view"],
        shortcut: "Ctrl+1",
    },
    CommandSpec {
        command: Command::View(BrowserMode::Icons),
        group: "View",
        label: "Switch to Icons",
        aliases: &["grid view"],
        shortcut: "Ctrl+2",
    },
    CommandSpec {
        command: Command::View(BrowserMode::List),
        group: "View",
        label: "Switch to List",
        aliases: &["table view"],
        shortcut: "Ctrl+3",
    },
    CommandSpec {
        command: Command::Hidden,
        group: "View",
        label: "Show hidden files",
        aliases: &["hide hidden files", "dotfiles"],
        shortcut: "Ctrl+H",
    },
    CommandSpec {
        command: Command::Sidebar,
        group: "View",
        label: "Show sidebar",
        aliases: &["hide sidebar", "places"],
        shortcut: "Ctrl+B",
    },
    CommandSpec {
        command: Command::File(FileCommand::NewFolder),
        group: "Files and folders",
        label: "New folder",
        aliases: &["mkdir", "create directory"],
        shortcut: "Ctrl+Shift+N",
    },
    CommandSpec {
        command: Command::File(FileCommand::Rename),
        group: "Files and folders",
        label: "Rename selected item",
        aliases: &["change name"],
        shortcut: "F2 / Ctrl+R",
    },
    CommandSpec {
        command: Command::File(FileCommand::Duplicate),
        group: "Files and folders",
        label: "Duplicate selected items",
        aliases: &["make a copy"],
        shortcut: "Ctrl+D",
    },
    CommandSpec {
        command: Command::File(FileCommand::CopyPaths),
        group: "Files and folders",
        label: "Copy selected paths",
        aliases: &["copy path", "clipboard", "address"],
        shortcut: "",
    },
    CommandSpec {
        command: Command::File(FileCommand::Properties),
        group: "Files and folders",
        label: "Show properties",
        aliases: &["info", "details", "permissions"],
        shortcut: "Alt+Enter",
    },
    CommandSpec {
        command: Command::File(FileCommand::Pin),
        group: "Files and folders",
        label: "Pin folder",
        aliases: &["unpin folder", "bookmark", "favourite"],
        shortcut: "",
    },
    CommandSpec {
        command: Command::File(FileCommand::Undo),
        group: "Files and folders",
        label: "Undo last file operation",
        aliases: &["revert"],
        shortcut: "Ctrl+Z",
    },
];

pub(super) fn matches(query: &str) -> Vec<usize> {
    let query = query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let mut matches: Vec<_> = COMMANDS
        .iter()
        .enumerate()
        .filter_map(|(index, spec)| {
            std::iter::once(spec.label)
                .chain(spec.aliases.iter().copied())
                .filter_map(|text| score(&query, &text.to_lowercase()))
                .min()
                .map(|score| (score, index))
        })
        .collect();
    matches.sort_unstable();
    matches.into_iter().map(|(_, index)| index).collect()
}

fn score(query: &str, text: &str) -> Option<usize> {
    if query.is_empty() || query == text {
        return Some(0);
    }
    if text.starts_with(query) {
        return Some(1);
    }
    if text.contains(query) {
        return Some(2);
    }
    let mut characters = text.chars();
    let mut gaps = 0;
    for wanted in query.chars().filter(|c| !c.is_whitespace()) {
        gaps += characters.by_ref().position(|c| c == wanted)?;
    }
    Some(3 + gaps)
}
