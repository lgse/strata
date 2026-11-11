//! Strata's interface components on Rune and rune-kit.
//!
//! Components take state by accessor (`fn(&S) -> &X`, `fn(&mut S) -> &mut X`) and own no
//! I/O: listings, thumbnails and search results arrive as data.
//!
//! Every module is declared here once (W0). A work package edits only inside its own module
//! folder, so parallel branches never touch this file.

pub mod code_view;
pub mod collection;
pub mod columns;
pub mod context_menu;
pub mod contract;
pub mod controls;
pub mod dialog;
pub mod document_view;
pub mod filter_bar;
pub mod folder_peek;
pub mod footer;
pub mod header;
pub mod icons_grid;
pub mod list_table;
pub mod location_bar;
pub mod media_player;
pub mod motion;
pub mod mount_prompts;
pub mod pdf_view;
pub mod preview_drawer;
pub mod progress_dock;
pub mod rename;
pub mod scroll;
pub mod search_palette;
pub mod sheet_grid;
pub mod sidebar;
pub mod tabs;
pub mod thumbnails;
pub mod viewers;
