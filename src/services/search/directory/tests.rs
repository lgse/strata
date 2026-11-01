// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn directory_index_reports_budgets_and_unreadable_roots() {
    let fixture = tempfile::tempdir().expect("fixture");
    let folder = fixture.path().join("folder");
    std::fs::create_dir(&folder).expect("folder");
    std::fs::write(folder.join("one"), "fixture").expect("file");
    std::fs::write(folder.join("two"), "fixture").expect("file");
    let alias = fixture.path().join("alias");
    std::os::unix::fs::symlink(&folder, &alias).expect("alias");
    let real = folder.canonicalize().expect("real folder");
    let unreadable = SearchCoverage {
        unreadable: true,
        ..Default::default()
    };
    for (root, fenced, max_entries, time_budget, expected) in [
        (
            folder.clone(),
            None,
            1,
            Duration::from_secs(10),
            SearchCoverage {
                entry_limit: true,
                ..Default::default()
            },
        ),
        (
            folder.clone(),
            None,
            10,
            Duration::ZERO,
            SearchCoverage {
                time_limit: true,
                ..Default::default()
            },
        ),
        (
            folder.join("missing"),
            None,
            10,
            Duration::from_secs(10),
            unreadable,
        ),
        (
            folder.clone(),
            Some(folder.clone()),
            10,
            Duration::from_secs(10),
            unreadable,
        ),
        (alias, Some(real), 10, Duration::from_secs(10), unreadable),
    ] {
        let operation = file_activity::begin(file_activity::OperationScope::default());
        let _fence = fenced.map(|folder| operation.fence([folder]));
        let index = SharedIndex::new();
        build_index(
            &index,
            vec![root.clone()],
            false,
            max_entries,
            time_budget,
            &SearchExclusions::default(),
        );
        let data = index.state.read().expect("index data");
        assert!(!data.indexing);
        assert_eq!(data.coverage, expected, "{}", root.display());
        assert!(data.items.len() <= max_entries);
    }
}

#[test]
fn cancelled_directory_index_never_publishes_results() {
    let fixture = tempfile::tempdir().expect("fixture");
    std::fs::write(fixture.path().join("needle"), "fixture").expect("file");
    let index = SharedIndex::new();
    index.release();
    build_index(
        &index,
        vec![fixture.path().to_path_buf()],
        false,
        10,
        Duration::from_secs(10),
        &SearchExclusions::default(),
    );
    assert!(index.state.read().expect("index data").items.is_empty());
}

#[test]
fn directory_index_follows_symlinks_to_directories() {
    let fixture = tempfile::tempdir().expect("fixture");
    let real_dir = fixture.path().join("real-folder");
    std::fs::create_dir(&real_dir).expect("real directory");
    std::os::unix::fs::symlink(&real_dir, fixture.path().join("alias-folder"))
        .expect("symlink to a directory");

    let index = SharedIndex::new();
    build_index(
        &index,
        vec![fixture.path().to_path_buf()],
        false,
        usize::MAX,
        Duration::from_secs(10),
        &SearchExclusions::default(),
    );
    let data = index.state.read().expect("index data");
    let alias = data
        .items
        .iter()
        .find(|item| item.name == "alias-folder")
        .expect("the symlink should be indexed");
    assert!(
        alias.is_directory,
        "a symlink to a directory should be indexed as a directory, matching the GIO listing"
    );
}

#[test]
fn directory_index_honours_gio_hidden_file_entries() {
    let fixture = tempfile::tempdir().expect("fixture");
    std::fs::write(fixture.path().join("secret.txt"), "fixture").expect("file");
    std::fs::write(fixture.path().join(".hidden"), "secret.txt\n").expect(".hidden file");

    let index = SharedIndex::new();
    build_index(
        &index,
        vec![fixture.path().to_path_buf()],
        false,
        usize::MAX,
        Duration::from_secs(10),
        &SearchExclusions::default(),
    );
    assert!(
        !index
            .state
            .read()
            .expect("index data")
            .items
            .iter()
            .any(|item| item.name == "secret.txt"),
        "a name listed in .hidden should stay out while hidden files are off, matching the GIO listing"
    );

    let index = SharedIndex::new();
    build_index(
        &index,
        vec![fixture.path().to_path_buf()],
        true,
        usize::MAX,
        Duration::from_secs(10),
        &SearchExclusions::default(),
    );
    assert!(
        index
            .state
            .read()
            .expect("index data")
            .items
            .iter()
            .any(|item| item.name == "secret.txt"),
        "a GIO-hidden name should still be indexed once hidden files are shown"
    );
}
