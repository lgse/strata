// SPDX-License-Identifier: MIT

use strata_core::util::{DateDisplay, DateFormat, modified_date_for_seconds, set_date_format};

use super::LIST;
use crate::{
    host::offscreen::Offscreen,
    shell,
    test_support::{
        PATIENCE,
        view::{Shape, hosted_with, open},
    },
};

const MODULE: &str = "list::tests::tooltips";
const LONG_NAME: &str =
    "extremely-long-filename-that-will-certainly-truncate-in-the-list-name-column.txt";

const COLUMNS: Shape = Shape {
    described: "Columns view",
    list: "list-box",
    entry: "list-box-option",
};

const ICONS: Shape = Shape {
    described: "Icons view",
    list: "grid",
    entry: "grid-cell",
};

fn hover_rect(host: &mut Offscreen, rect: [f32; 4]) -> Option<String> {
    let rt = host.runtime();
    rt.pointer_move(rect[0] + rect[2] * 0.5, rect[1] + rect[3] * 0.5);
    rt.frame(1.0 / 60.0);
    let delay = rt.help_delay;
    rt.frame(delay + 0.1);
    rt.help_tooltip().map(|(text, _)| text.to_owned())
}

fn find_rect(host: &Offscreen, pred: impl Fn(&rune::ElementInfo) -> bool) -> [f32; 4] {
    host.elements()
        .into_iter()
        .find(pred)
        .expect("mounted element")
        .rect
}

fn set_live_date_format(host: &mut Offscreen, format: DateFormat) {
    let mut tx = rune::Tx::default();
    shell::saved(host.app_mut()).change(|s| s.date_format = format.as_str().to_owned());
    shell::sync_live_preferences(host.app_mut(), &mut tx);
    let _ = host.turn();
    host.runtime().frame(1.0 / 60.0);
}

#[test]
fn columns_rows_show_name_and_modified_date_tooltip_and_update_on_date_format_change() {
    hosted_with(
        MODULE,
        "columns",
        "date_format = \"iso\"\n".to_owned(),
        "columns_rows_show_name_and_modified_date_tooltip_and_update_on_date_format_change",
        |mut host| {
            let folder = tempfile::tempdir().expect("tempdir");
            std::fs::write(folder.path().join("alpha.txt"), "hello\n").expect("write");
            open(&mut host, COLUMNS, folder.path());
            assert!(host.run_until(PATIENCE, |h| {
                h.elements().iter().any(|e| {
                    e.help
                        .as_deref()
                        .is_some_and(|tip| tip.starts_with("alpha.txt\n"))
                })
            }));

            let row_rect = find_rect(&host, |e| {
                e.help
                    .as_deref()
                    .is_some_and(|tip| tip.starts_with("alpha.txt\n"))
            });
            let iso_tip = hover_rect(&mut host, row_rect).expect("iso tooltip");
            let mtime = host
                .app()
                .columns
                .listing(0)
                .and_then(|col| col.row_at(0))
                .and_then(|row| row.facts().modified)
                .expect("known modified timestamp");
            set_date_format(DateFormat::Iso8601);
            let expected_iso = format!(
                "alpha.txt\n{}",
                modified_date_for_seconds(mtime, DateDisplay::Preferred)
            );
            assert_eq!(iso_tip, expected_iso);

            set_live_date_format(&mut host, DateFormat::Long);
            let long_tip = hover_rect(&mut host, row_rect).expect("long tooltip");
            let expected_long = format!(
                "alpha.txt\n{}",
                modified_date_for_seconds(mtime, DateDisplay::Preferred)
            );
            assert_eq!(long_tip, expected_long);
            assert_ne!(long_tip, iso_tip);
        },
    );
}

#[test]
fn icons_tiles_show_name_and_modified_date_tooltip_and_update_on_date_format_change() {
    hosted_with(
        MODULE,
        "icons",
        "date_format = \"iso\"\n".to_owned(),
        "icons_tiles_show_name_and_modified_date_tooltip_and_update_on_date_format_change",
        |mut host| {
            let folder = tempfile::tempdir().expect("tempdir");
            std::fs::write(folder.path().join("beta.txt"), "hello\n").expect("write");
            open(&mut host, ICONS, folder.path());
            assert!(host.run_until(PATIENCE, |h| {
                h.elements().iter().any(|e| {
                    e.help
                        .as_deref()
                        .is_some_and(|tip| tip.starts_with("beta.txt\n"))
                })
            }));

            let icon_rect = find_rect(&host, |e| {
                e.key.as_deref() == Some("icon")
                    && e.help
                        .as_deref()
                        .is_some_and(|tip| tip.starts_with("beta.txt\n"))
            });
            let iso_tip = hover_rect(&mut host, icon_rect).expect("iso tooltip");
            let mtime = host
                .app()
                .icons
                .single
                .pane
                .listing()
                .and_then(|col| col.row_at(0))
                .and_then(|row| row.facts().modified)
                .expect("known modified timestamp");
            set_date_format(DateFormat::Iso8601);
            let expected_iso = format!(
                "beta.txt\n{}",
                modified_date_for_seconds(mtime, DateDisplay::Preferred)
            );
            assert_eq!(iso_tip, expected_iso);

            set_live_date_format(&mut host, DateFormat::Long);
            let long_tip = hover_rect(&mut host, icon_rect).expect("long tooltip");
            let expected_long = format!(
                "beta.txt\n{}",
                modified_date_for_seconds(mtime, DateDisplay::Preferred)
            );
            assert_eq!(long_tip, expected_long);
            assert_ne!(long_tip, iso_tip);
        },
    );
}

#[test]
fn list_rows_omit_date_when_modified_column_is_visible() {
    hosted_with(
        MODULE,
        "list",
        "date_format = \"iso\"\nbrowser_list_columns = { name = 160, mode = 160, size = 90, type = 120, modified = 150 }\n"
            .to_owned(),
        "list_rows_omit_date_when_modified_column_is_visible",
        |mut host| {
            let folder = tempfile::tempdir().expect("tempdir");
            std::fs::write(folder.path().join("a.txt"), "short\n").expect("write");
            std::fs::write(folder.path().join(LONG_NAME), "long\n").expect("write");
            open(&mut host, LIST, folder.path());
            assert!(host.run_until(PATIENCE, |h| {
                h.app()
                    .list
                    .single
                    .pane
                    .listing()
                    .and_then(|col| col.row_at(0))
                    .and_then(|row| row.facts().modified)
                    .is_some()
            }));

            let short_cell = find_rect(&host, |e| {
                e.key.as_deref() == Some("cell-name") && e.label.as_deref() == Some("a.txt")
            });
            let long_cell = find_rect(&host, |e| {
                e.key.as_deref() == Some("cell-name") && e.label.as_deref() == Some(LONG_NAME)
            });
            assert_eq!(
                hover_rect(&mut host, short_cell),
                None,
                "short name needs no tooltip when Modified column is visible"
            );
            assert_eq!(
                hover_rect(&mut host, long_cell).as_deref(),
                Some(LONG_NAME),
                "truncated name shows full name without date when Modified column is visible"
            );
        },
    );
}

#[test]
fn list_rows_include_date_when_modified_column_is_hidden() {
    hosted_with(
        MODULE,
        "list",
        "date_format = \"iso\"\nlist_hidden_columns = [\"modified\"]\n".to_owned(),
        "list_rows_include_date_when_modified_column_is_hidden",
        |mut host| {
            let folder = tempfile::tempdir().expect("tempdir");
            std::fs::write(folder.path().join("a.txt"), "short\n").expect("write");
            open(&mut host, LIST, folder.path());
            assert!(host.run_until(PATIENCE, |h| {
                h.elements().iter().any(|e| {
                    e.key.as_deref() == Some("cell-name")
                        && e.help
                            .as_deref()
                            .is_some_and(|tip| tip.starts_with("a.txt\n"))
                })
            }));

            let short_cell = find_rect(&host, |e| {
                e.key.as_deref() == Some("cell-name") && e.label.as_deref() == Some("a.txt")
            });
            let iso_tip = hover_rect(&mut host, short_cell).expect("iso tooltip");
            let mtime = host
                .app()
                .list
                .single
                .pane
                .listing()
                .and_then(|col| col.row_at(0))
                .and_then(|row| row.facts().modified)
                .expect("known modified timestamp");
            set_date_format(DateFormat::Iso8601);
            let expected_iso = format!(
                "a.txt\n{}",
                modified_date_for_seconds(mtime, DateDisplay::Preferred)
            );
            assert_eq!(iso_tip, expected_iso);

            set_live_date_format(&mut host, DateFormat::Long);
            let long_tip = hover_rect(&mut host, short_cell).expect("long tooltip");
            let expected_long = format!(
                "a.txt\n{}",
                modified_date_for_seconds(mtime, DateDisplay::Preferred)
            );
            assert_eq!(long_tip, expected_long);
            assert_ne!(long_tip, iso_tip);
        },
    );
}
