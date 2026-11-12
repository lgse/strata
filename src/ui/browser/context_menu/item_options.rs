// SPDX-License-Identifier: MIT

use super::ITEM_CONTEXT_SUMMARY_MAX_CHARS;
use super::options::{item_context_danger_option, item_context_option, submenu_row};
use crate::assets::icons;
use crate::ui::shortcut_reference::ContextHint;
use gtk::prelude::*;

fn separator() -> gtk::Separator {
    gtk::Separator::new(gtk::Orientation::Horizontal)
}

fn danger_option(icon: &str, label: &str, hint: ContextHint) -> gtk::Button {
    let option = item_context_danger_option(icon, label, hint);
    option.add_css_class("danger");
    option
}

fn append_all(container: &gtk::Box, widgets: &[&gtk::Widget]) {
    for widget in widgets {
        container.append(*widget);
    }
}

/// Plain creation rows; the new item is a sibling of the clicked entry.
pub(super) struct CreationOptions {
    pub(super) container: gtk::Box,
    pub(super) new_folder: gtk::Button,
    pub(super) new_file: gtk::Button,
}

impl CreationOptions {
    fn build() -> Self {
        let options = Self {
            container: gtk::Box::new(gtk::Orientation::Vertical, 0),
            new_folder: item_context_option(
                icons::FOLDER_PLUS,
                "New Folder",
                ContextHint::NewFolder,
            ),
            new_file: item_context_option(icons::FILE_PLUS, "New File", ContextHint::None),
        };
        append_all(
            &options.container,
            &[
                options.new_folder.upcast_ref(),
                options.new_file.upcast_ref(),
                separator().upcast_ref(),
            ],
        );
        options
    }
}

/// "Open in…" with its New Tab and New Window entries.
pub(super) struct OpenInRow {
    pub(super) row: gtk::Box,
    pub(super) tab: gtk::Button,
    pub(super) window: gtk::Button,
}

impl OpenInRow {
    fn build() -> Self {
        let open_in = item_context_option(icons::EXTERNAL_LINK, "Open in…", ContextHint::None);
        let tab = item_context_option(icons::PLUS, "New Tab", ContextHint::OpenInNewTab);
        let window = item_context_option(
            icons::APP_WINDOW,
            "New Window",
            ContextHint::OpenInNewWindow,
        );
        let row = submenu_row(&open_in, [&tab, &window]);
        Self { row, tab, window }
    }
}

/// The trailing delete rows; in Trash the first one already deletes permanently.
pub(super) struct DeleteOptions {
    pub(super) trash: gtk::Button,
    pub(super) permanent: gtk::Button,
    pub(super) without_trash: gtk::Button,
}

impl DeleteOptions {
    fn build(in_trash: bool) -> Self {
        let trash = if in_trash {
            danger_option(icons::CIRCLE_X, "Permanently delete", ContextHint::Trash)
        } else {
            item_context_option(icons::TRASH, "Move to Trash", ContextHint::Trash)
        };
        Self {
            trash,
            permanent: danger_option(
                icons::CIRCLE_X,
                "Permanently delete",
                ContextHint::PermanentDelete,
            ),
            without_trash: danger_option(
                icons::CIRCLE_X,
                "Delete without Trash",
                ContextHint::PermanentDelete,
            ),
        }
    }

    fn append_to(&self, container: &gtk::Box) {
        append_all(
            container,
            &[
                self.trash.upcast_ref(),
                self.permanent.upcast_ref(),
                self.without_trash.upcast_ref(),
            ],
        );
    }
}

pub(super) struct ItemHeader {
    pub(super) header: gtk::Box,
    pub(super) heading: gtk::Label,
    pub(super) summary: gtk::Label,
}

fn header_label(class: &str) -> gtk::Label {
    let label = gtk::Label::new(None);
    label.add_css_class(class);
    label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    label.set_max_width_chars(ITEM_CONTEXT_SUMMARY_MAX_CHARS);
    label.set_xalign(0.0);
    label
}

impl ItemHeader {
    fn build() -> Self {
        let header = gtk::Box::new(gtk::Orientation::Vertical, 2);
        header.add_css_class("item-context-header");
        let heading = header_label("item-context-title");
        let summary = header_label("item-context-summary");
        header.append(&heading);
        header.append(&summary);
        Self {
            header,
            heading,
            summary,
        }
    }
}

/// The opening rows shown above the clipboard rows for a single item.
pub(super) struct SingleOpenOptions {
    pub(super) container: gtk::Box,
    pub(super) group: gtk::Button,
    pub(super) group_separator: gtk::Separator,
    pub(super) open: gtk::Button,
    pub(super) open_with: gtk::Button,
    pub(super) preview: gtk::Button,
    pub(super) open_file_location: gtk::Button,
    pub(super) run: gtk::Button,
    pub(super) open_terminal: gtk::Button,
    pub(super) open_in: OpenInRow,
    pub(super) restore: gtk::Button,
    pub(super) remove_from_recent: gtk::Button,
    pub(super) extract: gtk::Button,
    pub(super) extract_to: gtk::Button,
}

fn group_option() -> gtk::Button {
    item_context_option(
        icons::FOLDER_PLUS,
        "New Folder with Selection",
        ContextHint::None,
    )
}

fn remove_from_recent_option(in_recent: bool) -> gtk::Button {
    let option = item_context_option(
        icons::CLOCK,
        "Remove from Recent",
        ContextHint::RemoveFromRecent,
    );
    option.set_visible(in_recent);
    option
}

fn restore_option(label: &str, in_trash: bool) -> gtk::Button {
    let option = item_context_option(icons::UNDO_2, label, ContextHint::Restore);
    option.set_visible(in_trash);
    option
}

impl SingleOpenOptions {
    fn build(in_trash: bool, in_recent: bool) -> Self {
        let options = Self {
            container: gtk::Box::new(gtk::Orientation::Vertical, 0),
            group: group_option(),
            group_separator: separator(),
            open: item_context_option(icons::EXTERNAL_LINK, "Open", ContextHint::Open),
            open_with: item_context_option(icons::APP_WINDOW, "Open With…", ContextHint::None),
            preview: item_context_option(icons::EYE, "Quick preview", ContextHint::Preview),
            open_file_location: item_context_option(
                icons::FOLDER_OPEN,
                "Open file location",
                ContextHint::None,
            ),
            run: item_context_option(icons::PLAY, "Run", ContextHint::None),
            open_terminal: item_context_option(
                icons::TERMINAL,
                "Open in Terminal",
                ContextHint::Terminal,
            ),
            open_in: OpenInRow::build(),
            restore: restore_option("Restore", in_trash),
            remove_from_recent: remove_from_recent_option(in_recent),
            extract: item_context_option(icons::PACKAGE_OPEN, "Extract here", ContextHint::None),
            extract_to: item_context_option(
                icons::FOLDER_ARCHIVE,
                "Extract to…",
                ContextHint::None,
            ),
        };
        append_all(
            &options.container,
            &[
                options.group.upcast_ref(),
                options.group_separator.upcast_ref(),
                options.open.upcast_ref(),
                options.open_with.upcast_ref(),
                options.preview.upcast_ref(),
                options.open_file_location.upcast_ref(),
                options.run.upcast_ref(),
                options.open_terminal.upcast_ref(),
                options.open_in.row.upcast_ref(),
                options.restore.upcast_ref(),
                options.remove_from_recent.upcast_ref(),
                options.extract.upcast_ref(),
                options.extract_to.upcast_ref(),
            ],
        );
        options
    }
}

/// Clipboard, transfer, and item rows for a single item.
pub(super) struct SingleEditOptions {
    pub(super) container: gtk::Box,
    pub(super) cut: gtk::Button,
    pub(super) copy: gtk::Button,
    pub(super) duplicate: gtk::Button,
    pub(super) rename: gtk::Button,
    pub(super) move_to: gtk::Button,
    pub(super) copy_to: gtk::Button,
    pub(super) other_pane: OtherPaneOptions,
    pub(super) compress: gtk::Button,
    pub(super) pin: gtk::Button,
    pub(super) customize: gtk::Button,
    pub(super) copy_path: gtk::Button,
    pub(super) copy_name: gtk::Button,
    pub(super) properties: gtk::Button,
    pub(super) delete: DeleteOptions,
}

fn cut_option() -> gtk::Button {
    item_context_option(icons::SCISSORS, "Cut", ContextHint::Cut)
}

fn copy_option() -> gtk::Button {
    item_context_option(icons::COPY, "Copy", ContextHint::Copy)
}

fn duplicate_option() -> gtk::Button {
    item_context_option(icons::COPY_PLUS, "Duplicate", ContextHint::Duplicate)
}

fn move_to_option() -> gtk::Button {
    item_context_option(icons::FOLDER_INPUT, "Move to…", ContextHint::MoveTo)
}

fn copy_to_option() -> gtk::Button {
    item_context_option(icons::FOLDER_OUTPUT, "Copy to…", ContextHint::CopyTo)
}

/// Copy and Move to Other Pane, shown while the tab shows two panes.
pub(super) struct OtherPaneOptions {
    pub(super) copy: gtk::Button,
    pub(super) moving: gtk::Button,
}

impl OtherPaneOptions {
    fn build() -> Self {
        Self {
            copy: item_context_option(
                icons::COLUMNS,
                "Copy to Other Pane",
                ContextHint::CopyToOtherPane,
            ),
            moving: item_context_option(
                icons::ARROW_RIGHT,
                "Move to Other Pane",
                ContextHint::MoveToOtherPane,
            ),
        }
    }
}

fn compress_option() -> gtk::Button {
    item_context_option(icons::PACKAGE_PLUS, "Compress…", ContextHint::None)
}

fn properties_option() -> gtk::Button {
    item_context_option(icons::INFO, "Properties", ContextHint::Properties)
}

impl SingleEditOptions {
    fn build(in_trash: bool) -> Self {
        let options = Self {
            container: gtk::Box::new(gtk::Orientation::Vertical, 0),
            cut: cut_option(),
            copy: copy_option(),
            duplicate: duplicate_option(),
            rename: item_context_option(icons::PENCIL, "Rename", ContextHint::Rename),
            move_to: move_to_option(),
            copy_to: copy_to_option(),
            other_pane: OtherPaneOptions::build(),
            compress: compress_option(),
            pin: item_context_option(icons::PIN, "Pin to sidebar", ContextHint::Pin),
            customize: item_context_option(icons::PALETTE, "Customize…", ContextHint::None),
            copy_path: item_context_option(icons::ROUTE, "Copy path", ContextHint::CopyPath),
            copy_name: item_context_option(icons::FILE_TYPE, "Copy name", ContextHint::None),
            properties: properties_option(),
            delete: DeleteOptions::build(in_trash),
        };
        append_all(
            &options.container,
            &[
                options.cut.upcast_ref(),
                options.copy.upcast_ref(),
                options.duplicate.upcast_ref(),
                options.rename.upcast_ref(),
                separator().upcast_ref(),
                options.move_to.upcast_ref(),
                options.copy_to.upcast_ref(),
                options.other_pane.copy.upcast_ref(),
                options.other_pane.moving.upcast_ref(),
                separator().upcast_ref(),
                options.compress.upcast_ref(),
                separator().upcast_ref(),
                options.pin.upcast_ref(),
                options.customize.upcast_ref(),
                options.copy_path.upcast_ref(),
                options.copy_name.upcast_ref(),
                options.properties.upcast_ref(),
                separator().upcast_ref(),
            ],
        );
        options.delete.append_to(&options.container);
        options
    }
}

/// The opening rows shown above the clipboard rows for several items.
pub(super) struct MultipleOpenOptions {
    pub(super) container: gtk::Box,
    pub(super) group: gtk::Button,
    pub(super) group_separator: gtk::Separator,
    pub(super) open: gtk::Button,
    pub(super) open_with: gtk::Button,
    pub(super) open_in: OpenInRow,
    pub(super) restore: gtk::Button,
    pub(super) remove_from_recent: gtk::Button,
}

impl MultipleOpenOptions {
    fn build(in_trash: bool, in_recent: bool) -> Self {
        let options = Self {
            container: gtk::Box::new(gtk::Orientation::Vertical, 0),
            group: group_option(),
            group_separator: separator(),
            open: item_context_option(icons::EXTERNAL_LINK, "Open", ContextHint::Open),
            open_with: item_context_option(icons::APP_WINDOW, "Open With…", ContextHint::None),
            open_in: OpenInRow::build(),
            restore: restore_option("Restore items", in_trash),
            remove_from_recent: remove_from_recent_option(in_recent),
        };
        append_all(
            &options.container,
            &[
                options.group.upcast_ref(),
                options.group_separator.upcast_ref(),
                options.open.upcast_ref(),
                options.open_with.upcast_ref(),
                options.open_in.row.upcast_ref(),
                options.restore.upcast_ref(),
                options.remove_from_recent.upcast_ref(),
            ],
        );
        options
    }
}

/// Clipboard, transfer, and item rows for several items.
pub(super) struct MultipleEditOptions {
    pub(super) container: gtk::Box,
    pub(super) cut: gtk::Button,
    pub(super) copy: gtk::Button,
    pub(super) duplicate: gtk::Button,
    pub(super) move_to: gtk::Button,
    pub(super) copy_to: gtk::Button,
    pub(super) other_pane: OtherPaneOptions,
    pub(super) compress: gtk::Button,
    pub(super) copy_paths: gtk::Button,
    pub(super) copy_names: gtk::Button,
    pub(super) properties: gtk::Button,
    pub(super) delete: DeleteOptions,
}

impl MultipleEditOptions {
    fn build(in_trash: bool) -> Self {
        let options = Self {
            container: gtk::Box::new(gtk::Orientation::Vertical, 0),
            cut: cut_option(),
            copy: copy_option(),
            duplicate: duplicate_option(),
            move_to: move_to_option(),
            copy_to: copy_to_option(),
            other_pane: OtherPaneOptions::build(),
            compress: compress_option(),
            copy_paths: item_context_option(icons::ROUTE, "Copy paths", ContextHint::CopyPaths),
            copy_names: item_context_option(icons::FILE_TYPE, "Copy names", ContextHint::None),
            properties: properties_option(),
            delete: DeleteOptions::build(in_trash),
        };
        append_all(
            &options.container,
            &[
                options.cut.upcast_ref(),
                options.copy.upcast_ref(),
                options.duplicate.upcast_ref(),
                separator().upcast_ref(),
                options.move_to.upcast_ref(),
                options.copy_to.upcast_ref(),
                options.other_pane.copy.upcast_ref(),
                options.other_pane.moving.upcast_ref(),
                separator().upcast_ref(),
                options.compress.upcast_ref(),
                separator().upcast_ref(),
                options.copy_paths.upcast_ref(),
                options.copy_names.upcast_ref(),
                options.properties.upcast_ref(),
                separator().upcast_ref(),
            ],
        );
        options.delete.append_to(&options.container);
        options.container.set_visible(false);
        options
    }
}

/// Every row of the item context menu, grouped as the menu shows them.
pub(super) struct ItemMenuOptions {
    pub(super) content: gtk::Box,
    pub(super) remaining: gtk::Box,
    pub(super) header: ItemHeader,
    pub(super) creation: CreationOptions,
    pub(super) single_open: SingleOpenOptions,
    pub(super) single: SingleEditOptions,
    pub(super) multiple_open: MultipleOpenOptions,
    pub(super) multiple: MultipleEditOptions,
    pub(super) in_trash: bool,
    pub(super) in_recent: bool,
}

fn item_menu_box() -> gtk::Box {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
    container.add_css_class("item-context-menu");
    container
}

impl ItemMenuOptions {
    pub(super) fn build(in_trash: bool, in_recent: bool) -> Self {
        let options = Self {
            content: item_menu_box(),
            remaining: item_menu_box(),
            header: ItemHeader::build(),
            creation: CreationOptions::build(),
            single_open: SingleOpenOptions::build(in_trash, in_recent),
            single: SingleEditOptions::build(in_trash),
            multiple_open: MultipleOpenOptions::build(in_trash, in_recent),
            multiple: MultipleEditOptions::build(in_trash),
            in_trash,
            in_recent,
        };
        // Each edit group's visibility drives its open group above.
        for (edit, open) in [
            (&options.single.container, &options.single_open.container),
            (
                &options.multiple.container,
                &options.multiple_open.container,
            ),
        ] {
            edit.bind_property("visible", open, "visible")
                .sync_create()
                .build();
        }
        options.content.append(&options.creation.container);
        options.content.append(&options.single_open.container);
        options.remaining.append(&options.single.container);
        options.content.append(&options.multiple_open.container);
        options.remaining.append(&options.multiple.container);
        options
    }
}
