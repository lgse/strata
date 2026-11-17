// SPDX-License-Identifier: MIT

//! An entry moved out of the folder, a selectionless removal: the keyboard goes to the row
//! beside it, unselected, as GTK's list view moves its focus there, and the browser holds no
//! cursor, so a folder there does not open (finding lgse/strata#2546).

use rune::Mods;
use strata_core::model::Location;

use super::{browser_holds, folder_name, hosted, idle, open, press, rest, wait};
use crate::{App, test_support::PATIENCE};

#[test]
fn a_move_out_leaves_the_keyboard_on_the_neighbour_and_opens_no_folder() {
    hosted(
        "moved::a_move_out_leaves_the_keyboard_on_the_neighbour_and_opens_no_folder",
        |mut host| {
            let folder = tempfile::tempdir().expect("folder");
            let root = folder.path().to_owned();
            for name in ["alpha", "beta"] {
                std::fs::create_dir(root.join(name)).expect("folder");
            }
            std::fs::write(root.join("zeta.txt"), "zeta\n").expect("file");
            let name = folder_name(&root);
            open(&mut host, &root);
            press(&mut host, "End");
            wait(&mut host, "zeta.txt under the keyboard", |seen| {
                seen.focused.as_deref() == Some("zeta.txt")
            });

            let (from, into) = (root.join("zeta.txt"), root.join("alpha"));
            host.runtime().update(move |app: &mut App, _| {
                let sources = vec![Location::local(&from)];
                crate::transfer::start_drop_transfer(app, Location::local(&into), sources, true);
            });
            let moved = host.run_until(PATIENCE, |_| root.join("alpha/zeta.txt").exists());
            assert!(moved, "zeta.txt moved into alpha");
            let seen = wait(&mut host, "the keyboard on beta", |seen| {
                seen.focused.as_deref() == Some("beta")
            });
            assert!(
                seen.pane(&name)
                    .is_some_and(|pane| pane.selected().is_empty())
            );
            rest(&mut host);
            let seen = super::seen(&host);
            assert_eq!(
                seen.pane_names(),
                [name.as_str()],
                "beta's column stays closed"
            );
            assert_eq!(browser_holds(&host, 0), (Vec::new(), None));

            host.modifiers(Mods::default());
            press(&mut host, "Up");
            let seen = wait(&mut host, "Up to go on from beta", |seen| {
                seen.focused.as_deref() == Some("alpha")
            });
            assert_eq!(
                seen.pane(&name).map(|pane| pane.selected()),
                Some(vec!["alpha"])
            );
            idle(&mut host);
            let holds = (vec!["alpha".to_owned()], Some("alpha".to_owned()));
            assert_eq!(browser_holds(&host, 0), holds);
        },
    );
}
