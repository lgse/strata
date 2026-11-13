// SPDX-License-Identifier: MIT

//! A Shift range goes on from where it started after F5 reads the folder again, as GTK's
//! does from the anchor the shared browser keeps by location (finding lgse/strata#2354).

use rune::{Key, Mods};

use super::{MODULE, act, listed, press};
use crate::{
    host::offscreen::Offscreen,
    shell,
    test_support::{PATIENCE, view::hosted_in},
};

/// The keys that move to the next and the previous entry: a row in Columns and List, a tile
/// in the Icons grid, whose rows the fixture's five entries fit in one of.
fn steps(mode: &str) -> (Key, Key) {
    if mode == "icons" {
        (Key::Right, Key::Left)
    } else {
        (Key::Down, Key::Up)
    }
}

fn shift() -> Mods {
    Mods {
        shift: true,
        ..Mods::default()
    }
}

/// The names the browser holds selected in its active column.
fn selected(host: &Offscreen) -> Vec<String> {
    let browser = host.app().browser();
    let depth = browser.active_depth().unwrap_or_default();
    let positions = browser.selected_positions(depth);
    browser
        .with_column_entries(depth, |entries| {
            positions
                .iter()
                .filter_map(|position| entries.get(*position))
                .map(|entry| entry.display_name.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn keeps_the_anchor_across_a_refresh(mode: &'static str, name: &str) {
    hosted_in(
        MODULE,
        mode,
        &format!("refresh_anchor::{name}"),
        |mut host| {
            let _folder = listed(&mut host);
            act(&mut host, |app, tx| {
                let view = shell::view_focus(app).expect("a shown view");
                tx.focus(view, true);
            });
            let (next, previous) = steps(mode);
            press(&mut host, Key::Home, Mods::default());
            press(&mut host, next.clone(), Mods::default());
            press(&mut host, previous, shift());
            assert_eq!(
                selected(&host),
                ["archive", "documents"],
                "{mode}: the range"
            );

            let copied = host.app().rows_copied();
            press(&mut host, Key::Function(5), Mods::default());
            let read_again = host.run_until(PATIENCE, |host| host.app().rows_copied() > copied);
            assert!(read_again, "{mode}: F5 read nothing");
            while host.turn() {}

            // From the anchor, `documents`, back to it: the range is the anchor alone.
            press(&mut host, next, shift());
            assert_eq!(
                selected(&host),
                ["documents"],
                "{mode}: the range after F5 starts where it started before"
            );
        },
    );
}

in_each_view! {
    keeps_the_anchor_across_a_refresh:
    "columns" => the_columns_keep_the_shift_anchor_across_a_refresh,
    "icons" => the_grid_keeps_the_shift_anchor_across_a_refresh,
    "list" => the_table_keeps_the_shift_anchor_across_a_refresh,
}
