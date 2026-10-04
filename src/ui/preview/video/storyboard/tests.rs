// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn nearest_cell_falls_back_outwards_while_the_board_is_partial() {
    let board = Storyboard::new(Sheet {
        width: 1,
        height: 1,
        count: 8,
        duration_us: 8_000_000,
    });
    assert!(board.nearest(3_500_000).is_none());
    board.set_cell(4, vec![0; 4]);
    board.set_cell(1, vec![0; 4]);
    board.set_cell(9, vec![0; 4]);
    board.set_cell(2, vec![0; 3]);
    assert_eq!(board.loaded_cells(), 2);
    let four = board.nearest(4_500_000).expect("exact cell");
    let one = board.nearest(1_500_000).expect("exact cell");
    assert!(!four.eq(&one));
    assert!(
        board
            .nearest(2_500_000)
            .expect("cell 2 falls back")
            .eq(&one)
    );
    assert!(
        board
            .nearest(3_500_000)
            .expect("cell 3 falls back")
            .eq(&four)
    );
    assert!(
        board
            .nearest(7_999_999)
            .expect("the end falls back")
            .eq(&four)
    );
    assert!(!board.is_complete());
}
