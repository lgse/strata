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
mod entry_icon;
mod events;
// The per-folder views the process keeps, `folder-views.toml`, and when they are written.
mod folder_views;
// Metadata fills for the rows a view shows, once per poll.
mod fills;
// GPU shader compositor for `El::effect` slots backed by `rune-fx`.
mod fx;
// The cursor and selection handover every view shares.
mod handover;
mod host;
mod launch;
mod listing;
// What a listing that shows no entry says, in every view.
mod surface;
// The one folder a single-pane view shows: the Icons and List views'.
mod pane;
mod portal;
#[cfg(feature = "qa")]
mod qa;
mod settings;
mod shell;
// What the single-pane views, Icons and List, share beside their pane.
mod single;
mod slots;
mod startup;
mod theme;
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
mod mount_prompts;
mod sidebar;
// A4 Tabs
mod tab_strip;
// A5 Location bar
// Return presses a focused button, as on GTK (Rune packet entry 103).
mod buttons;
mod error_dialog;
mod location;
mod open_failed;
mod pane_header;
mod popover;
// A6 Menus, dialogs and keymap
mod compress;
mod connect_server;
mod customize;
mod footer;
mod run;
mod shortcuts;
// The keys the GTK window reads after a view's or a field's own.
mod window_keys;
// A7 Entry management and undo
mod new_item;
mod new_window;
mod rename_failed;
mod trash;
mod undo;
// A8 View switching and appearance
mod appearance;
// A9 Pointer and click modes
mod clicks;
mod pointer;
// A10 Search, filter and sort
mod filter;
mod palette;
mod sort;
// A11 10xer mode
mod peek;
mod tenxer;
mod tenxer_fx;
// A12 Clipboard and conflicts
mod clipboard;
mod conflict_dialog;
mod transfer;
// A13 Archives and providers
mod archive;
mod file_providers;
// A14 Open with, terminal and custom actions
mod custom_actions;
mod open_with;
mod terminal;
// A15 Preview drawer, text and documents
mod preview;
// A16 Media, PDF and model viewers
// A17 Properties
mod properties;
mod raw_details;
// A18 Settings and themes
mod settings_dialog;
// A19 Drag and drop
mod dnd;
mod drop_dialog;
// A20 Portal chooser
mod chooser;
// A21 Window and launch
// The process behind the windows: its one instance on the session bus, and the windows its
// launches ask for.
mod application;

// The order of these lines is the order of the parts: within a slot, and wherever the shell
// calls each of them.
slots::parts! {
    // The slice. The editor's message shows under a menu, so it comes first.
    columns: columns::Columns,
    rename: rename::Rename,
    // A21's Shift+Enter (`open-in-window`) comes before `menu`'s `activate`, which takes Enter
    // with any Shift, as GTK asks `opens_focused_in_new` before it activates.
    new_window: new_window::NewWindow,
    menu: menus::Menu,
    // B3 Commands and key dispatch
    // A1 Icons view and thumbnails
    icons: icons::Icons,
    // A2 List view
    list: list::List,
    // A3 Sidebar
    mount_prompts: mount_prompts::MountPromptsPart,
    sidebar: sidebar::Sidebar,
    // A4 Tabs
    tab_strip: tab_strip::TabStrip,
    // A5 Location bar
    location: location::Bar,
    open_failed: open_failed::OpenFailed,
    pane_header: pane_header::PaneHeader,
    // A6 Menus, dialogs and keymap
    run: run::Run,
    footer: footer::Footer,
    shortcuts: shortcuts::Shortcuts,
    customize: customize::Customize,
    compress: compress::Compress,
    connect_server: connect_server::ConnectServer,
    // A7 Entry management and undo
    undo: undo::Undo,
    new_item: new_item::NewItem,
    trash: trash::Trash,
    // A8 View switching and appearance
    appearance: appearance::Appearance,
    // A9 Pointer and click modes
    pointer: pointer::Pointer,
    // A10 Search, filter and sort
    filter: filter::Filter,
    palette: palette::Palette,
    palette_menu: palette::menu::PaletteMenu,
    sort: sort::Sort,
    // A11 10xer mode
    tenxer: tenxer::Tenxer,
    peek: peek::Peek,
    // A12 Clipboard and conflicts
    clipboard: clipboard::Clipboard,
    transfer: transfer::Transfer,
    transfer_dialog: transfer::TransferDialog,
    conflict_dialog: conflict_dialog::ConflictDialog,
    // A13 Archives and providers
    archive: archive::Archive,
    file_providers: file_providers::FileProviders,
    // A14 Open with, terminal and custom actions
    custom_actions: custom_actions::CustomActions,
    open_with: open_with::OpenWith,
    terminal: terminal::Terminal,
    // A15 Preview drawer, text and documents
    preview: preview::Preview,
    // A16 Media, PDF and model viewers
    // A17 Properties
    properties: properties::Properties,
    // A18 Settings and themes
    settings_dialog: settings_dialog::SettingsDialog,
    // A19 Drag and drop
    dnd: dnd::Dnd,
    drop_dialog: drop_dialog::DropDialog,
    // A20 Portal chooser
    chooser: chooser::Chooser,
    chooser_dialog: chooser::ChooserDialog,
    // Error and informational dialogs paint above Settings and every other modal.
    error_dialog: error_dialog::ErrorDialog,
    // 10x mode activation visual FX overlay: paints above dialogs (including Settings live previews)
    // while staying completely inert and pointer-transparent.
    tenxer_fx: tenxer_fx::TenxerFxOverlay,
    // A21 Window and launch: `new_window` is listed before `menu`.
}

pub use launch::run;
