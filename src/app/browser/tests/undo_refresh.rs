// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn move_undo_refreshes_both_ends_without_repeating_shared_parents() {
    let items = [
        (Location::local("/source/a"), Location::local("/target/a")),
        (
            Location::local("/source/b"),
            Location::uri("sftp://host/target/b"),
        ),
        (Location::local("/other/c"), Location::local("/target/c")),
    ]
    .into_iter()
    .map(|(original, current)| UndoMoveItem {
        record: MoveRecord { original, current },
        conflict: TransferConflict::FailIfExists,
    })
    .collect::<Vec<_>>();

    assert_eq!(
        undo_move_parents(&items),
        HashSet::from([
            Location::local("/source"),
            Location::local("/target"),
            Location::local("/other"),
            Location::uri("sftp://host/target"),
        ])
    );
}
