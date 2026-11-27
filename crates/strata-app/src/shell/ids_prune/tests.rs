// SPDX-License-Identifier: MIT

use strata_components::contract::EntrySource;
use strata_core::model::Location;

use super::prune_now;
use crate::{
    columns::Columns,
    events::Change,
    icons::Icons,
    shell,
    slots::{Part, Place},
    test_support::{PATIENCE, fixture, hosted},
};

#[test]
fn pending_rename_target_id_survives_pruning() {
    hosted(
        "shell::ids_prune::tests",
        "pending_rename_target_id_survives_pruning",
        |mut host| {
            let folder = fixture();
            let root = Location::local(folder.path());
            host.app().browser().navigate_location(root, false);
            let listed = host.run_until(PATIENCE, |host| host.app().columns.rows_shown(0) > 0);
            assert!(listed, "fixture folder should load in columns");

            host.runtime().update(|app, tx| {
                assert!(shell::show_view(app, "icons", tx));
            });
            assert!(
                host.app()
                    .icons
                    .single
                    .pane
                    .listing()
                    .is_some_and(|col| !col.is_empty()),
                "icons pane should hold the loaded folder"
            );

            let from_loc = Location::local(folder.path().join("readme.md"));
            let col_to_loc = Location::local(folder.path().join("readme-col-renamed.md"));
            let pane_to_loc = Location::local(folder.path().join("readme-pane-renamed.md"));
            let stale_loc = Location::local(folder.path().join("never-referenced.txt"));

            let (from_id, col_to_id, pane_to_id, stale_id) = {
                let ids = &host.app().window.ids;
                (
                    ids.of(&from_loc),
                    ids.of(&col_to_loc),
                    ids.of(&pane_to_loc),
                    ids.of(&stale_loc),
                )
            };

            let Place::View(columns_view) = Columns::PLACE else {
                unreachable!("Columns is a View part");
            };
            let Place::View(icons_view) = Icons::PLACE else {
                unreachable!("Icons is a View part");
            };

            host.runtime().update(|app, tx| {
                (columns_view.apply)(
                    app,
                    Change::Moved {
                        from: from_id,
                        to: col_to_id,
                    },
                    tx,
                );
                (icons_view.apply)(
                    app,
                    Change::Moved {
                        from: from_id,
                        to: pane_to_id,
                    },
                    tx,
                );
            });

            prune_now(host.app());

            let ids = &host.app().window.ids;
            assert_eq!(
                ids.of(&col_to_loc),
                col_to_id,
                "Columns pending rename target id must survive pruning"
            );
            assert_eq!(
                ids.of(&pane_to_loc),
                pane_to_id,
                "Pane pending rename target id must survive pruning"
            );
            assert_ne!(
                ids.of(&stale_loc),
                stale_id,
                "unreferenced stale id must be evicted by pruning"
            );
        },
    );
}
