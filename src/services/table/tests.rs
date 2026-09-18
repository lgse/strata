// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn delimited_tables_preserve_quotes_newlines_ragged_rows_and_literal_markup() {
    let parsed = parse_delimited(
        "name,value\n\"a,b\",\"line 1\nline 2\"\n<script>,2,extra\n",
        b',',
        &Cancellation::default(),
    )
    .expect("quoted CSV");
    let layout = crate::services::layout_document(parsed.document, &Cancellation::default())
        .expect("table layout");
    assert_eq!(
        layout.units[0].copy_text,
        "name\tvalue\na,b\tline 1\nline 2\n<script>\t2\textra\n"
    );
    let parsed = parse_delimited("a\tb\n1\t2\n", b'\t', &Cancellation::default()).expect("TSV");
    assert_eq!(parsed.document.blocks.len(), 2);
}

#[test]
fn tables_are_not_limited_to_200_rows_or_512_cells() {
    let source = format!("a,b,c\n{}", "1,2,3\n".repeat(1000));
    let parsed = parse_delimited(&source, b',', &Cancellation::default()).expect("large table");
    assert_eq!(parsed.document.blocks.len(), 1001);
    assert!(parsed.warnings.is_empty());
}

#[test]
fn cancellation_and_column_budget_are_observable() {
    let cancellation = Cancellation::default();
    cancellation.cancel();
    assert!(parse_delimited("a,b\n1,2", b',', &cancellation).is_err());
    let source = vec!["cell"; TABLE_COLUMN_LIMIT + 1].join(",");
    let parsed =
        parse_delimited(&source, b',', &Cancellation::default()).expect("bounded wide table");
    assert!(!parsed.warnings.is_empty());
}

#[test]
fn reads_first_worksheet_in_each_supported_workbook_format() {
    for extension in ["xls", "xlsx", "ods"] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "tests/fixtures/spreadsheets/any_sheets.{extension}"
        ));
        let table = read_workbook(&path).expect("workbook fixture");
        assert_eq!(table.rows[0], ["1", "2"], "{extension}");
        assert_eq!(table.rows[1], ["3", "4"], "{extension}");
        assert_eq!(table.rows[2], ["5", "6"], "{extension}");
        assert_eq!(table.rows[3], ["", ""], "{extension}");
        assert!(table.rows[4][0].contains("4 sheets"), "{extension}");
        assert_eq!(table.rows.len(), 5, "only the first worksheet is previewed");
        assert!(!table.truncated);
    }
}

#[test]
fn rejects_oversized_and_invalid_workbooks() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let path = directory.path().join("book.xlsx");
    std::fs::write(&path, "not a workbook").expect("invalid fixture");
    assert!(read_workbook(&path).is_err());
    std::fs::File::create(&path)
        .expect("sparse fixture")
        .set_len(WORKBOOK_BYTE_LIMIT + 1)
        .expect("oversized fixture");
    assert!(
        read_workbook(&path)
            .expect_err("oversized workbook")
            .contains("too large")
    );
}

#[test]
fn rejects_untrusted_oversized_table_output() {
    let data = TableData {
        rows: vec![vec!["x".into(); TABLE_COLUMN_LIMIT + 1]],
        truncated: false,
    };
    assert!(TableData::from_json(&serde_json::to_vec(&data).expect("table JSON")).is_err());
    assert!(TableData::from_json(b"not json").is_err());
}
