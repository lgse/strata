// SPDX-License-Identifier: MIT

//! Strata's interface on Rune: the application state, its view, and the hosts that run it
//! beside the GLib context Strata's `Browser` schedules on.
//!
//! `docs/rune/slice/V5-shell-and-loop.md` describes the loop and what each host does.

// strata-core embeds the catalogs; this path holds none, so the macro adds none itself.
rust_i18n::i18n!(
    "src/no-catalogs",
    fallback = "en",
    backend = strata_core::i18n::Catalogs::compiled()
);

mod events;
// The cursor and selection handover every view shares.
mod handover;
mod host;
mod launch;
mod listing;
// The one folder a single-pane view shows: the Icons and List views'.
mod pane;
#[cfg(feature = "qa")]
mod qa;
mod settings;
mod shell;
mod slots;
mod startup;
#[cfg(test)]
mod test_support;

// The parts of the interface (`slots` says what one is). A part is two lines here: its
// module in this list, and its registration in the list below. Each package of the parity
// plan (`docs/rune/parity/`) adds its lines under its own comment, in both lists, so that no
// two branches add at the same place and git merges them without a conflict.

// The slice: the Columns view, the context menus and the rename editor.
mod columns;
mod menus;
mod rename;
// B3 Commands and key dispatch
mod commands;
// A1 Icons view and thumbnails
mod icons;
// A2 List view
mod list;
// A3 Sidebar
// A4 Tabs
// A5 Location bar
// A6 Menus, dialogs and keymap
// A7 Entry management and undo
// A8 View switching and appearance
// A9 Pointer and click modes
// A10 Search, filter and sort
// A11 10xer mode
// A12 Clipboard and conflicts
// A13 Archives and providers
// A14 Open with, terminal and custom actions
// A15 Preview drawer, text and documents
// A16 Media, PDF and model viewers
// A17 Properties
// A18 Settings and themes
// A19 Drag and drop
// A20 Portal chooser
// A21 Window and launch

// The order of these lines is the order of the parts: within a slot, and wherever the shell
// calls each of them.
slots::parts! {
    // The slice. The editor's message shows under a menu, so it comes first.
    columns: columns::Columns,
    rename: rename::Rename,
    menu: menus::Menu,
    // B3 Commands and key dispatch
    // A1 Icons view and thumbnails
    icons: icons::Icons,
    // A2 List view
    list: list::List,
    // A3 Sidebar
    // A4 Tabs
    // A5 Location bar
    // A6 Menus, dialogs and keymap
    // A7 Entry management and undo
    // A8 View switching and appearance
    // A9 Pointer and click modes
    // A10 Search, filter and sort
    // A11 10xer mode
    // A12 Clipboard and conflicts
    // A13 Archives and providers
    // A14 Open with, terminal and custom actions
    // A15 Preview drawer, text and documents
    // A16 Media, PDF and model viewers
    // A17 Properties
    // A18 Settings and themes
    // A19 Drag and drop
    // A20 Portal chooser
    // A21 Window and launch
}

pub use launch::run;
