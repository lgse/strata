// SPDX-License-Identifier: MIT

use std::collections::HashSet;

use super::{MAX_PER_TYPE, State, load_from, prune_unknown, record_in, save_to, select_recent};

fn types(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

fn known(values: &[&str]) -> HashSet<String> {
    values.iter().map(|value| value.to_string()).collect()
}

fn record(state: &mut State, mime: &str, app: &str) {
    record_in(state, &types(&[mime]), app);
}

#[test]
fn launches_order_most_recent_first_per_type() {
    let mut state = State::default();
    record(&mut state, "text/plain", "alpha.desktop");
    record(&mut state, "text/plain", "beta.desktop");
    record(&mut state, "text/plain", "alpha.desktop");

    assert_eq!(
        state.recent_ids(&types(&["text/plain"])),
        ["alpha.desktop", "beta.desktop"]
    );
}

#[test]
fn histories_are_independent_across_types() {
    let mut state = State::default();
    record(&mut state, "text/plain", "alpha.desktop");
    record(&mut state, "image/png", "gamma.desktop");

    assert_eq!(state.recent_ids(&types(&["text/plain"])), ["alpha.desktop"]);
    assert_eq!(state.recent_ids(&types(&["image/png"])), ["gamma.desktop"]);
}

#[test]
fn multi_type_merges_keep_global_recency_without_duplicates() {
    let mut state = State::default();
    record(&mut state, "text/plain", "alpha.desktop");
    record(&mut state, "image/png", "beta.desktop");
    record(&mut state, "text/plain", "beta.desktop");

    assert_eq!(
        state.recent_ids(&types(&["text/plain", "image/png"])),
        ["beta.desktop", "alpha.desktop"]
    );
}

#[test]
fn history_caps_each_type_and_evicts_the_oldest_entry() {
    let mut state = State::default();
    for index in 0..MAX_PER_TYPE + 2 {
        record(&mut state, "text/plain", &format!("app{index}.desktop"));
    }

    let ids = state.recent_ids(&types(&["text/plain"]));
    assert_eq!(ids.len(), MAX_PER_TYPE);
    assert_eq!(ids[0], format!("app{}.desktop", MAX_PER_TYPE + 1));
    assert!(
        !ids.iter()
            .any(|id| id == "app0.desktop" || id == "app1.desktop")
    );
}

#[test]
fn history_survives_a_restart_with_order_intact() {
    let directory = tempfile::tempdir().expect("history directory");
    let path = directory.path().join("recent-apps.toml");
    let mut state = State::default();
    record(&mut state, "text/plain", "alpha.desktop");
    record(&mut state, "text/plain", "beta.desktop");
    record(&mut state, "image/png", "gamma.desktop");
    save_to(&path, &mut state);

    let restored = load_from(&path);
    assert_eq!(
        restored.recent_ids(&types(&["text/plain"])),
        ["beta.desktop", "alpha.desktop"]
    );
    assert_eq!(
        restored.recent_ids(&types(&["image/png"])),
        ["gamma.desktop"]
    );

    let mut restored = restored;
    record(&mut restored, "text/plain", "alpha.desktop");
    assert_eq!(
        restored.recent_ids(&types(&["text/plain"])),
        ["alpha.desktop", "beta.desktop"]
    );
}

#[test]
fn missing_history_loads_empty() {
    let directory = tempfile::tempdir().expect("history directory");
    let restored = load_from(&directory.path().join("absent.toml"));
    assert!(restored.recent_ids(&types(&["text/plain"])).is_empty());
}

#[test]
fn malformed_history_loads_empty() {
    let directory = tempfile::tempdir().expect("history directory");
    let path = directory.path().join("recent-apps.toml");
    std::fs::write(&path, "not = [valid toml").expect("malformed history");
    assert!(
        load_from(&path)
            .recent_ids(&types(&["text/plain"]))
            .is_empty()
    );

    std::fs::write(
        &path,
        "version = 999\n[types]\n\"text/plain\" = [{ app = \"a.desktop\", seq = 1 }]\n",
    )
    .expect("future history");
    assert!(
        load_from(&path)
            .recent_ids(&types(&["text/plain"]))
            .is_empty()
    );
}

#[test]
fn prune_drops_uninstalled_apps_but_keeps_installed_ones() {
    let mut state = State::default();
    record(&mut state, "text/plain", "alpha.desktop");
    record(&mut state, "text/plain", "vanished.desktop");

    prune_unknown(&mut state, &known(&["alpha.desktop"]));
    assert_eq!(state.recent_ids(&types(&["text/plain"])), ["alpha.desktop"]);
}

#[test]
fn prune_never_empties_history_on_an_empty_application_database() {
    let mut state = State::default();
    record(&mut state, "text/plain", "alpha.desktop");

    prune_unknown(&mut state, &HashSet::new());
    assert_eq!(state.recent_ids(&types(&["text/plain"])), ["alpha.desktop"]);
}

#[test]
fn prune_removes_types_left_without_entries() {
    let mut state = State::default();
    record(&mut state, "text/plain", "vanished.desktop");

    prune_unknown(&mut state, &known(&["alpha.desktop"]));
    assert!(state.types.is_empty());
}

#[test]
fn selection_keeps_history_order_and_drops_ineligible_ids() {
    let ordered = types(&["beta.desktop", "vanished.desktop", "alpha.desktop"]);
    let eligible = known(&["alpha.desktop", "beta.desktop"]);

    assert_eq!(
        select_recent(&ordered, &eligible),
        ["beta.desktop", "alpha.desktop"]
    );
    assert!(select_recent(&[], &eligible).is_empty());
    assert!(select_recent(&ordered, &HashSet::new()).is_empty());
}

#[test]
fn exhausted_persisted_sequences_preserve_order_and_allow_launches() {
    let directory = tempfile::tempdir().expect("history directory");
    let path = directory.path().join("recent-apps.toml");
    std::fs::write(&path, format!(
        "version = 1\n[types]\n\"text/plain\" = [{{ app = \"beta.desktop\", seq = {} }}, {{ app = \"alpha.desktop\", seq = {} }}]\n\"image/png\" = [{{ app = \"gamma.desktop\", seq = {} }}]\n",
        u64::MAX, u64::MAX - 2, u64::MAX - 1,
    )).expect("exhausted history");
    let mut state = load_from(&path);
    record_in(
        &mut state,
        &types(&["text/plain", "image/png"]),
        "new.desktop",
    );
    save_to(&path, &mut state);
    assert_eq!(
        load_from(&path).recent_ids(&types(&["text/plain", "image/png"])),
        [
            "new.desktop",
            "beta.desktop",
            "gamma.desktop",
            "alpha.desktop"
        ]
    );
}
