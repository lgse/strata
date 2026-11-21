// SPDX-License-Identifier: MIT

//! What the List view saves of its columns, in the `settings.toml` the Rune window shares
//! with the installed GTK application.

use std::{path::PathBuf, time::Duration};

use strata_components::list_table::{
    ColumnId, ColumnLayout, ColumnWidths, LayoutChange, LayoutWhy,
};
use strata_desktop::settings::{ListColumnId, Preferences};

use super::{LIST, MODULE, mixed, settle};
use crate::{
    list::columns,
    test_support::{
        PATIENCE,
        view::{hosted_in, hosted_with, open, wheel},
    },
};

fn settings_file() -> PathBuf {
    strata_core::storage::config_directory().join("settings.toml")
}

fn saved_text() -> String {
    std::fs::read_to_string(settings_file()).expect("settings")
}

/// Opening a folder, resizing the window, scrolling and leaving rewrites nothing: neither the
/// columns, which the person did not touch, nor anything else the GTK application reads from
/// the same file.
#[test]
fn an_untouched_list_window_writes_nothing() {
    hosted_in(
        MODULE,
        "list",
        "saved::an_untouched_list_window_writes_nothing",
        |mut host| {
            let before = saved_text();
            let folder = mixed();
            open(&mut host, LIST, folder.path());
            settle(&mut host);
            host.runtime().set_viewport((640.0, 360.0), 1.0);
            settle(&mut host);
            wheel(&mut host, -60.0, 3);
            host.run_until(Duration::from_millis(500), |_| false);

            assert_eq!(saved_text(), before);
        },
    );
}

/// A layout and widths the person made are saved through the settings' rules, and a window
/// that starts over the file reads back what this one keeps: the same order, the same hidden
/// columns, the same widths.
#[test]
fn the_columns_saved_read_back_as_the_window_keeps_them() {
    hosted_in(
        MODULE,
        "list",
        "saved::the_columns_saved_read_back_as_the_window_keeps_them",
        |mut host| {
            let folder = mixed();
            open(&mut host, LIST, folder.path());
            let id = ColumnId::new;
            let defaults = ["name", "mode", "size", "type", "modified"].map(id);
            let layout = ColumnLayout::normalize(
                &defaults,
                Some(&id("name")),
                ["size", "name", "type"],
                ["mode", "name"],
            );
            // A dragged width is fractional; the file keeps whole points.
            let widths = ColumnWidths::new()
                .with(&id("size"), 120.4)
                .with(&id("name"), 240.0);
            host.runtime().update(|app, _| {
                columns::keep_layout(
                    app,
                    LayoutChange {
                        layout,
                        why: LayoutWhy::Reorder,
                    },
                );
                columns::keep_widths(app, widths);
            });
            let written = host.run_until(PATIENCE, |_| {
                saved_text().contains("list_hidden_columns") && saved_text().contains("size = 120")
            });
            assert!(written, "{}", saved_text());

            let read: Preferences = toml::from_str(&saved_text()).expect("the saved settings");
            let list = &host.app().list;
            assert_eq!(columns::layout(&read), list.layout);
            assert_eq!(columns::widths(&read), list.widths);
            assert!(!list.layout.is_visible(&id("mode")), "{:?}", list.layout);
            assert_eq!(list.widths.get(&id("size")), Some(120.0));
        },
    );
}

/// GTK's optional columns, which this table does not build, keep what GTK saved for them
/// through a reorder and a resize here: their places in the order, their hidden state, their
/// widths, and the set GTK shows.
#[test]
fn a_rune_save_keeps_the_columns_gtk_alone_shows() {
    let gtk_saved = "list_column_order = [\"name\", \"duration\", \"mode\", \"size\", \"type\", \
                     \"created\", \"modified\", \"dimensions\"]\n\
                     list_hidden_columns = [\"dimensions\"]\n\
                     list_shown_columns = [\"duration\", \"created\"]\n\
                     [browser_list_columns]\n\
                     mode = 90\nsize = 100\ntype = 120\nmodified = 150\n\
                     duration = 130\ncreated = 140\ndimensions = 125\n"
        .to_owned();
    let name = "saved::a_rune_save_keeps_the_columns_gtk_alone_shows";
    hosted_with(MODULE, "list", gtk_saved, name, |mut host| {
        let folder = mixed();
        open(&mut host, LIST, folder.path());
        let id = ColumnId::new;
        let defaults = ["name", "mode", "size", "type", "modified"].map(id);
        let layout = ColumnLayout::normalize(
            &defaults,
            Some(&id("name")),
            ["size", "name", "type"],
            ["mode"],
        );
        let widths = ColumnWidths::new().with(&id("size"), 110.0);
        host.runtime().update(|app, _| {
            columns::keep_layout(
                app,
                LayoutChange {
                    layout,
                    why: LayoutWhy::Reorder,
                },
            );
            columns::keep_widths(app, widths);
        });
        let written = host.run_until(PATIENCE, |_| saved_text().contains("size = 110"));
        assert!(written, "{}", saved_text());

        let read: Preferences = toml::from_str(&saved_text()).expect("the saved settings");
        let order = &read.list_column_order;
        let place = |column| order.iter().position(|saved| *saved == column);
        assert_eq!(place(ListColumnId::Duration), Some(1), "{order:?}");
        assert_eq!(place(ListColumnId::Created), Some(5), "{order:?}");
        assert_eq!(place(ListColumnId::Dimensions), Some(7), "{order:?}");
        assert!(
            place(ListColumnId::Size) < place(ListColumnId::Name),
            "{order:?}"
        );
        assert!(read.list_hidden_columns.contains(&ListColumnId::Dimensions));
        assert!(read.list_hidden_columns.contains(&ListColumnId::Mode));
        assert_eq!(
            read.list_shown_columns,
            [ListColumnId::Duration, ListColumnId::Created]
        );
        let saved = read.browser_list_columns.expect("the saved widths");
        assert_eq!(saved.width(ListColumnId::Duration), Some(130));
        assert_eq!(saved.width(ListColumnId::Created), Some(140));
        assert_eq!(saved.width(ListColumnId::Dimensions), Some(125));
        assert_eq!(saved.width(ListColumnId::Size), Some(110));
    });
}
