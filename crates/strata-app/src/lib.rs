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

// The text of a row's facts, as the Icons details line and the List size column show it.
mod details;
mod events;
// The per-folder views the process keeps, `folder-views.toml`, and when they are written.
mod folder_views;
// Metadata fills for the rows a view shows, once per poll.
mod fills;
// The cursor and selection handover every view shares.
mod handover;
mod host;
mod launch;
mod listing;
// What a listing that shows no entry says, in every view.
mod surface;
// The one folder a single-pane view shows: the Icons and List views'.
mod pane;
#[cfg(feature = "qa")]
mod qa;
mod settings;
mod shell;
// What the single-pane views, Icons and List, share beside their pane.
mod single;
mod slots;
mod startup;
// Thumbnails for the views: the render pool, one per process, and a view's viewer.
#[cfg(test)]
mod test_support;
mod thumbnails;

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
mod sidebar;
// A4 Tabs
// A5 Location bar
// Return presses a focused button, as on GTK (Rune packet entry 103).
mod buttons;
mod error_dialog;
mod location;
mod open_failed;
mod pane_header;
// A6 Menus, dialogs and keymap
// A7 Entry management and undo
mod new_item;
mod rename_failed;
mod trash;
mod undo;
// A8 View switching and appearance
mod appearance;
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
    sidebar: sidebar::Sidebar,
    // A4 Tabs
    // A5 Location bar
    location: location::Bar,
    error_dialog: error_dialog::ErrorDialog,
    open_failed: open_failed::OpenFailed,
    pane_header: pane_header::PaneHeader,
    // A6 Menus, dialogs and keymap
    // A7 Entry management and undo
    undo: undo::Undo,
    new_item: new_item::NewItem,
    trash: trash::Trash,
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
