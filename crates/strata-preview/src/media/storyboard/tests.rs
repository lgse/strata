// SPDX-License-Identifier: MIT

use std::io::Cursor;

use super::*;

fn sheet() -> Sheet {
    Sheet {
        width: 2,
        height: 1,
        count: 8,
        duration_us: 16_000_000,
    }
}

#[test]
fn subdivision_covers_every_cell_once_from_the_middle_outwards() {
    assert_eq!(subdivision_order(8), [4, 2, 6, 1, 3, 5, 7, 0]);
    for count in [1, 2, 3, 8, 47, 48] {
        let mut order = subdivision_order(count);
        assert_eq!(order.len(), count as usize, "{count}");
        order.sort_unstable();
        assert!(order.iter().copied().eq(0..count), "{count}");
    }
    assert_eq!(cell_count(1_000_000), MIN_CELLS);
    assert_eq!(cell_count(60_000_000), 30);
    assert_eq!(cell_count(7_200_000_000), MAX_CELLS);
    let sheet = sheet();
    assert_eq!(sheet.cell_time_us(0), 1_000_000);
    assert_eq!(sheet.cell_time_us(7), 15_000_000);
    assert_eq!(sheet.cell_at(0), 0);
    assert_eq!(sheet.cell_at(2_999_999), 1);
    assert_eq!(sheet.cell_at(16_000_000), 7);
    assert_eq!(sheet.cell_at(u64::MAX), 7);
}

#[test]
fn readers_accept_missing_cells_but_reject_bad_sheets_cells_and_trailing_data() {
    let sheet = sheet();
    let mut bytes = Vec::new();
    sheet.write(&mut bytes).expect("sheet");
    write_cell(&mut bytes, 4, &[1; 8]).expect("cell");
    write_cell(&mut bytes, 0, &[2; 8]).expect("cell");
    write_end(&mut bytes, sheet).expect("end");
    let mut reader = Cursor::new(bytes.clone());
    let read = Sheet::read(&mut reader).expect("sheet");
    assert_eq!(read, sheet);
    let mut cells = CellReader::new(read);
    assert_eq!(
        cells.read(&mut reader).expect("cell"),
        Some((4, vec![1; 8]))
    );
    assert_eq!(
        cells.read(&mut reader).expect("cell"),
        Some((0, vec![2; 8]))
    );
    assert_eq!(cells.read(&mut reader).expect("end"), None);
    assert!(cells.read(&mut reader).is_err(), "nothing follows the end");

    for (name, bad) in [
        ("duplicate cell", {
            let mut bytes = Vec::new();
            sheet.write(&mut bytes).expect("sheet");
            write_cell(&mut bytes, 4, &[1; 8]).expect("cell");
            write_cell(&mut bytes, 4, &[1; 8]).expect("cell");
            bytes
        }),
        ("wrong length", {
            let mut bytes = Vec::new();
            sheet.write(&mut bytes).expect("sheet");
            write_cell(&mut bytes, 1, &[1; 7]).expect("cell");
            bytes
        }),
        ("index past the sheet", {
            let mut bytes = Vec::new();
            sheet.write(&mut bytes).expect("sheet");
            write_cell(&mut bytes, 9, &[1; 8]).expect("cell");
            bytes
        }),
    ] {
        let mut reader = Cursor::new(bad);
        let mut cells = CellReader::new(Sheet::read(&mut reader).expect("sheet"));
        loop {
            match cells.read(&mut reader) {
                Ok(Some(_)) => continue,
                Ok(None) => panic!("{name} was accepted"),
                Err(_) => break,
            }
        }
    }

    for bad in [
        Sheet { width: 0, ..sheet },
        Sheet {
            height: MAX_CELL_EDGE + 1,
            ..sheet
        },
        Sheet { count: 0, ..sheet },
        Sheet {
            count: MAX_CELLS + 1,
            ..sheet
        },
        Sheet {
            duration_us: 0,
            ..sheet
        },
        Sheet {
            duration_us: MAX_DURATION_US,
            ..sheet
        },
    ] {
        let mut bytes = Vec::new();
        bad.write(&mut bytes).expect("sheet");
        assert!(Sheet::read(&mut Cursor::new(bytes)).is_err(), "{bad:?}");
    }
    assert!(Sheet::read(&mut Cursor::new(b"STRPEAK1".repeat(4))).is_err());
}
