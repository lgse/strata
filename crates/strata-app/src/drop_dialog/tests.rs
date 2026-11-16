// SPDX-License-Identifier: MIT

use std::path::Path;

use rune::{Key, KeyEvent, Mods};
use strata_core::model::Location;
use strata_fs::services::VolumeRelation;

use super::{cross_volume_drop_description, is_open, prompt};
use crate::{
    host::offscreen::Offscreen,
    test_support::{PATIENCE, fixture, hosted, send_key, view::idle},
};

const MODULE: &str = "drop_dialog::tests";

fn press(host: &mut Offscreen, key: Key, shift: bool) {
    send_key(
        host,
        KeyEvent {
            key,
            mods: Mods {
                shift,
                ..Mods::default()
            },
        },
    );
    host.turn();
    while host.turn() {}
}

fn wait_exists(host: &mut Offscreen, path: &Path, exists: bool) {
    let settled = host.run_until(PATIENCE, |_| path.exists() == exists);
    assert!(settled, "expected {} exists={exists}", path.display());
    idle(host);
}

/// A prompt asked although the volumes were found the same (a forced ask) says what an
/// unknown relation says, as GTK's does; only a confirmed other device says so.
#[test]
fn only_a_confirmed_other_device_is_described_as_one() {
    let unknown = cross_volume_drop_description(VolumeRelation::Unknown);
    assert_eq!(cross_volume_drop_description(VolumeRelation::Same), unknown);
    assert_ne!(
        cross_volume_drop_description(VolumeRelation::Different),
        unknown
    );
}

#[test]
fn enter_activates_copy_move_or_cancel_in_shift_tab_order() {
    hosted(
        MODULE,
        "enter_activates_copy_move_or_cancel_in_shift_tab_order",
        |mut host| {
            for (shift_tabs, copies, moves) in
                [(0, true, false), (1, false, true), (2, false, false)]
            {
                let folder = fixture();
                let root = folder.path();
                let src = root.join("todo.txt");
                let dest = root.join("archive");
                host.app().open(Location::local(root));
                assert!(host.run_until(PATIENCE, |h| {
                    h.app().browser().column_entry_counts(0).is_some()
                }));
                idle(&mut host);

                host.runtime().update(|app, _| {
                    prompt(
                        app,
                        Location::local(&dest),
                        vec![Location::local(&src)],
                        VolumeRelation::Different,
                    );
                });
                host.turn();
                idle(&mut host);
                assert!(is_open(host.app()));

                for _ in 0..shift_tabs {
                    press(&mut host, Key::Tab, true);
                }
                press(&mut host, Key::Enter, false);
                assert!(!is_open(host.app()));

                if copies || moves {
                    wait_exists(&mut host, &dest.join("todo.txt"), true);
                    assert_eq!(src.exists(), copies);
                } else {
                    assert!(!dest.join("todo.txt").exists());
                    assert!(src.exists());
                }
            }
        },
    );
}
