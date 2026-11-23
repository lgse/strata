// SPDX-License-Identifier: MIT

// strata-core embeds the catalogs; this path holds none, so the macro adds none itself.
rust_i18n::i18n!(
    "src/no-catalogs",
    fallback = "en",
    backend = strata_core::i18n::Catalogs::compiled()
);

mod details;
mod entry_icon;
mod events;
mod fills;
mod folder_views;
mod fx;
mod handover;
mod host;
mod launch;
mod listing;
mod pane;
mod portal;
#[cfg(feature = "qa")]
mod qa;
mod settings;
mod shell;
mod single;
mod slots;
mod startup;
mod surface;
#[cfg(test)]
mod test_support;
mod theme;
mod thumbnails;

mod appearance;
mod application;
mod archive;
mod buttons;
mod chooser;
mod clicks;
mod clipboard;
mod columns;
mod commands;
mod compress;
mod conflict_dialog;
mod connect_server;
mod custom_actions;
mod customize;
mod dnd;
mod drop_dialog;
mod error_dialog;
mod file_providers;
mod filter;
mod footer;
mod icons;
mod list;
mod location;
mod menus;
mod mount_prompts;
mod new_item;
mod new_window;
mod open_failed;
mod open_with;
mod palette;
mod pane_header;
mod peek;
mod pointer;
mod popover;
mod preview;
mod properties;
mod raw_details;
mod rename;
mod rename_failed;
mod run;
mod settings_dialog;
mod shortcuts;
mod sidebar;
mod sort;
mod tab_strip;
mod tenxer;
mod tenxer_fx;
mod terminal;
mod transfer;
mod trash;
mod undo;
mod window_keys;

slots::parts! {
    // Registration order matters: rename message sits under menus; `new_window` precedes `menu`.
    columns: columns::Columns,
    rename: rename::Rename,
    new_window: new_window::NewWindow,
    menu: menus::Menu,
    icons: icons::Icons,
    list: list::List,
    mount_prompts: mount_prompts::MountPromptsPart,
    sidebar: sidebar::Sidebar,
    tab_strip: tab_strip::TabStrip,
    location: location::Bar,
    open_failed: open_failed::OpenFailed,
    pane_header: pane_header::PaneHeader,
    run: run::Run,
    footer: footer::Footer,
    shortcuts: shortcuts::Shortcuts,
    customize: customize::Customize,
    compress: compress::Compress,
    connect_server: connect_server::ConnectServer,
    undo: undo::Undo,
    new_item: new_item::NewItem,
    trash: trash::Trash,
    appearance: appearance::Appearance,
    pointer: pointer::Pointer,
    filter: filter::Filter,
    palette: palette::Palette,
    palette_menu: palette::menu::PaletteMenu,
    sort: sort::Sort,
    tenxer: tenxer::Tenxer,
    peek: peek::Peek,
    clipboard: clipboard::Clipboard,
    transfer: transfer::Transfer,
    transfer_dialog: transfer::TransferDialog,
    conflict_dialog: conflict_dialog::ConflictDialog,
    archive: archive::Archive,
    file_providers: file_providers::FileProviders,
    custom_actions: custom_actions::CustomActions,
    open_with: open_with::OpenWith,
    terminal: terminal::Terminal,
    preview: preview::Preview,
    properties: properties::Properties,
    settings_dialog: settings_dialog::SettingsDialog,
    dnd: dnd::Dnd,
    drop_dialog: drop_dialog::DropDialog,
    chooser: chooser::Chooser,
    chooser_dialog: chooser::ChooserDialog,
    // Error and informational dialogs paint above Settings and every other modal.
    error_dialog: error_dialog::ErrorDialog,
    tenxer_fx: tenxer_fx::TenxerFxOverlay,
}

pub use launch::run;
