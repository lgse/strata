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
/// standard columns, the same opted-in optional columns, and the same widths.
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
            let defaults = [
                "name",
                "mode",
                "size",
                "type",
                "modified",
                "duration",
                "dimensions",
                "created",
            ]
            .map(id);
            let layout = ColumnLayout::normalize(
                &defaults,
                Some(&id("name")),
                ["size", "name", "type"],
                ["mode", "name", "dimensions", "created"],
            );
            // A dragged width is fractional; the file keeps whole points.
            let widths = ColumnWidths::new()
                .with(&id("size"), 120.4)
                .with(&id("duration"), 130.2)
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
                saved_text().contains("list_hidden_columns")
                    && saved_text().contains("list_shown_columns")
                    && saved_text().contains("size = 120")
            });
            assert!(written, "{}", saved_text());

            let read: Preferences = toml::from_str(&saved_text()).expect("the saved settings");
            let list = &host.app().list;
            assert_eq!(columns::layout(&read), list.layout);
            assert_eq!(columns::widths(&read), list.widths);
            assert_eq!(read.list_hidden_columns, [ListColumnId::Mode]);
            assert_eq!(read.list_shown_columns, [ListColumnId::Duration]);
            assert!(!list.layout.is_visible(&id("mode")), "{:?}", list.layout);
            assert!(list.layout.is_visible(&id("duration")), "{:?}", list.layout);
            assert!(
                !list.layout.is_visible(&id("dimensions")),
                "{:?}",
                list.layout
            );
            assert_eq!(list.widths.get(&id("size")), Some(120.0));
            assert_eq!(list.widths.get(&id("duration")), Some(130.0));
        },
    );
}

/// Optional columns saved by GTK round-trip through Rune's layout and widths, and Reset
/// clears `list_column_order`, `list_hidden_columns`, and `list_shown_columns` while hiding
/// optional columns again.
#[test]
fn optional_columns_round_trip_and_reset_clears_opt_ins() {
    let gtk_saved = "list_column_order = [\"name\", \"duration\", \"mode\", \"size\", \"type\", \
                     \"created\", \"modified\", \"dimensions\"]\n\
                     list_hidden_columns = [\"mode\"]\n\
                     list_shown_columns = [\"duration\", \"created\"]\n\
                     [browser_list_columns]\n\
                     mode = 90\nsize = 100\ntype = 120\nmodified = 150\n\
                     duration = 130\ncreated = 140\ndimensions = 125\n"
        .to_owned();
    let name = "saved::optional_columns_round_trip_and_reset_clears_opt_ins";
    hosted_with(MODULE, "list", gtk_saved, name, |mut host| {
        let folder = mixed();
        open(&mut host, LIST, folder.path());
        let id = ColumnId::new;
        {
            let list = &host.app().list;
            assert!(list.layout.is_visible(&id("duration")));
            assert!(list.layout.is_visible(&id("created")));
            assert!(!list.layout.is_visible(&id("dimensions")));
            assert!(!list.layout.is_visible(&id("mode")));
            assert_eq!(list.widths.get(&id("duration")), Some(130.0));
            assert_eq!(list.widths.get(&id("created")), Some(140.0));
            assert_eq!(list.widths.get(&id("dimensions")), Some(125.0));
        }

        host.runtime().update(|app, _| {
            let layout = ColumnLayout::new(app.list.layout.order().iter().cloned());
            columns::keep_layout(
                app,
                LayoutChange {
                    layout,
                    why: LayoutWhy::Reset,
                },
            );
        });
        let cleared = host.run_until(PATIENCE, |_| {
            let text = saved_text();
            !text.contains("list_hidden_columns")
                && !text.contains("list_shown_columns")
                && !text.contains("list_column_order")
        });
        assert!(cleared, "{}", saved_text());

        let list = &host.app().list;
        assert!(list.layout.is_visible(&id("mode")));
        assert!(!list.layout.is_visible(&id("duration")));
        assert!(!list.layout.is_visible(&id("dimensions")));
        assert!(!list.layout.is_visible(&id("created")));
    });
}
