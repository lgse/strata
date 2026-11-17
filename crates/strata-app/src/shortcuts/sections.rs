// SPDX-License-Identifier: MIT

//! The commands each reference section names, for the keyboard settings' tables: GTK's
//! `command_sections` (`src/ui/shortcut_reference.rs`), over the ported tables in
//! `contents.rs` and `tenxer.rs`. The 10xer map's sections are the window's in every view; the
//! portal chooser's scopes (`tenxer_scopes`) join them with [A20]'s chooser rows.
//!
//! **A copy:** `src/ui` does not move, so this is kept beside GTK's by hand. A change to `command_sections` or its helpers
//! on `main` merges clean and changes nothing here: make the same change in the merge
//! (`docs/rune/shared-state.md`, "Merging from `main`").
//!
//! [A20]: ../../../../docs/rune/parity/packages/A20.md

use strata_desktop::keys::{Command, Map};

use super::contents::{self, Mention, Mode, Section, Shortcut};

/// The one title the navigation sections share.
const NAVIGATION: &str = "Navigation";

const NAVIGATION_TITLES: [&str; 3] = ["Columns navigation", "Icons navigation", "List navigation"];

/// `command_sections`: each section's title and the commands its rows show, every view's
/// sections merged in the order their views give them.
pub(crate) fn command_sections(map: Map) -> Vec<(&'static str, Vec<Command>)> {
    let mut merged: Vec<(&'static str, Vec<Command>)> = Vec::new();
    for mode in [Mode::Columns, Mode::Icons, Mode::List] {
        let mut previous = None;
        let sections = match map {
            Map::Tenxer => super::tenxer::tenxer_sections(mode),
            Map::Default => contents::default_sections(mode),
        };
        for section in sections {
            let index = merged_section(&mut merged, merged_title(section.title), previous);
            previous = Some(index);
            add_section_commands(&mut merged[index].1, &section);
        }
    }
    merged
}

fn merged_title(title: &'static str) -> &'static str {
    if NAVIGATION_TITLES.contains(&title) {
        NAVIGATION
    } else {
        title
    }
}

/// The index of `title`'s section, inserted after `previous` when new so sections keep the
/// order their views give them.
fn merged_section(
    merged: &mut Vec<(&'static str, Vec<Command>)>,
    title: &'static str,
    previous: Option<usize>,
) -> usize {
    if let Some(index) = merged.iter().position(|(listed, _)| *listed == title) {
        return index;
    }
    let index = previous.map_or(0, |previous| previous + 1);
    merged.insert(index, (title, Vec::new()));
    index
}

fn add_section_commands(commands: &mut Vec<Command>, section: &Section) {
    for command in section.rows.iter().flat_map(shortcut_commands) {
        if !commands.contains(&command) {
            commands.push(command);
        }
    }
}

/// The commands a row shows: its own and those its action names.
fn shortcut_commands(shortcut: &Shortcut) -> Vec<Command> {
    let mentioned = match shortcut.mention {
        Some(Mention::Keys(keys)) => keys.commands(),
        Some(Mention::Held(_)) | None => Vec::new(),
    };
    shortcut
        .keys
        .commands()
        .into_iter()
        .chain(mentioned)
        .collect()
}
