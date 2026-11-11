//! A keystroke's search over a large index, in release builds only:
//! `cargo test -p strata-components --release --lib settings_search::tests::perf -- --nocapture`.

use std::time::Instant;

use super::super::fixture;
use super::super::{IndexRow, SettingsSearchState};

/// Strata's index has about 300 rows with every Keyboard row; this is 2,000 or more.
#[test]
fn a_keystroke_searches_two_thousand_rows_within_a_frame() {
    let mut index = fixture::index();
    let base = index.rows.clone();
    let mut copy = 0;
    while index.rows.len() < 2_000 {
        copy += 1;
        index.rows.extend(base.iter().map(|r| IndexRow {
            key: format!("{}-{copy}", r.key),
            group: r.group.as_ref().map(|g| format!("{g}-{copy}")),
            ..r.clone()
        }));
    }
    let query = "thumbnail size worker";
    let mut st = SettingsSearchState::new();
    let mut times = Vec::new();
    for round in 0..20 {
        for end in 1..=query.len() {
            // Alternate a trailing character so every keystroke is a new query.
            let text = format!("{}{}", &query[..end], if round % 2 == 0 { "" } else { "x" });
            st.set_text(&text);
            let t = Instant::now();
            if let Some(r) = st.plan(&index) {
                st.store(r);
            }
            times.push(t.elapsed().as_secs_f64() * 1e3);
        }
    }
    times.sort_by(f64::total_cmp);
    let p95 = times[times.len() * 95 / 100];
    let max = times[times.len() - 1];
    eprintln!(
        "settings search, {} rows: keystroke p95 {p95:.2} ms, max {max:.2} ms",
        index.rows.len()
    );
    assert!(
        p95 < 16.0,
        "a keystroke's search fits a frame: p95 {p95:.2} ms"
    );
}
