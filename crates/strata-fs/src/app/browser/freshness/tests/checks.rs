// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn an_unchanged_stamp_never_relists_while_a_changed_one_does() {
    let source = Rc::new(Folders::default());
    let share = source.add("/net/share", network(1), Some(Watching::Live));
    let sub = source.add("/net/share/sub", network(2), Some(Watching::Live));
    let (browser, clock) = browser_on(&source);
    browser.navigate(share.clone());
    browser.show_child(0, sub.clone());
    source.change(&sub, &["listed", "added elsewhere"]);

    clock.tick_at(15);

    assert_eq!((source.checks_of(&share), source.checks_of(&sub)), (1, 1));
    assert_eq!((source.loads_of(&share), source.loads_of(&sub)), (1, 2));
    assert_eq!(names_at(&browser, 1), ["added elsewhere", "listed"]);
}

#[test]
fn folders_changed_on_one_mount_relist_one_after_the_other() {
    let source = Rc::new(Folders::default());
    let share = source.add("/net/share", network(1), Some(Watching::Live));
    let sub = source.add("/net/share/sub", network(2), Some(Watching::Live));
    let (browser, clock) = browser_on(&source);
    browser.navigate(share.clone());
    browser.show_child(0, sub.clone());
    source.change(&share, &["listed", "added elsewhere"]);
    source.change(&sub, &["listed", "added elsewhere"]);
    source.hold_loads.set(true);

    clock.tick_at(15);
    let while_the_share_lists = (source.loads_of(&share), source.loads_of(&sub));
    source.finish_held_loads();
    source.finish_held_loads();

    assert_eq!(while_the_share_lists, (2, 1));
    assert_eq!(names_at(&browser, 0), ["added elsewhere", "listed"]);
    assert_eq!(names_at(&browser, 1), ["added elsewhere", "listed"]);
}

#[test]
fn a_folder_only_this_kernel_writes_is_never_checked_while_a_network_folder_is() {
    let source = Rc::new(Folders::default());
    let home = source.add("/home/me", local(1), Some(Watching::Live));
    let nas = source.add("/home/me/nas", network(2), Some(Watching::Live));
    let (browser, clock) = browser_on(&source);
    browser.navigate(home.clone());
    let armed_for_home_alone = clock.armed();
    browser.show_child(0, nas.clone());
    let armed_with_the_share = clock.armed();

    clock.tick_at(15);

    assert_eq!(armed_for_home_alone, None);
    assert_eq!(armed_with_the_share, Some(Duration::from_secs(15)));
    assert_eq!((source.checks_of(&home), source.checks_of(&nas)), (0, 1));
}

#[test]
fn a_folder_without_a_monitor_is_checked() {
    assert_rows([None, Some(Watching::Unavailable)], |watching| {
        let source = Rc::new(Folders::default());
        let folder = source.add("/home/me/docs", local(1), watching);
        let (browser, clock) = browser_on(&source);
        browser.navigate(folder.clone());
        let armed = clock.armed().is_some();

        clock.tick_at(15);

        let got = (armed, source.checks_of(&folder));
        (got != (true, 1)).then(|| format!("{watching:?}: got {got:?}"))
    });
}

#[test]
fn with_network_folders_unwatched_only_local_folders_without_a_monitor_are_checked() {
    let source = Rc::new(Folders::default());
    let docs = source.add("/home/me/docs", local(1), None);
    let nas = source.add("/home/me/docs/nas", network(2), None);
    let (browser, clock) = browser_on(&source);
    let watch = |watch_network_folders| {
        browser.set_freshness(FreshnessInput {
            presence: Presence::Focused,
            watch_network_folders,
            ..browser.freshness_input()
        });
    };
    watch(false);
    browser.navigate(docs.clone());
    browser.show_child(0, nas.clone());
    source.change(&nas, &["listed", "added elsewhere"]);

    clock.tick_at(3);
    let unwatched = (source.checks_of(&docs), source.checks_of(&nas));
    watch(true);
    clock.tick_at(6);

    assert_eq!(unwatched, (1, 0));
    assert_eq!((source.checks_of(&docs), source.checks_of(&nas)), (2, 1));
    assert_eq!(names_at(&browser, 1), ["added elsewhere", "listed"]);
}

#[test]
fn an_unchanged_folder_is_checked_less_often_until_it_changes() {
    let source = Rc::new(Folders::default());
    let share = source.add("/net/share", network(1), Some(Watching::Live));
    let (browser, clock) = browser_on(&source);
    set_presence(&browser, Presence::Focused);
    browser.navigate(share.clone());

    for second in (3..=60).step_by(3) {
        clock.tick_at(second);
    }
    clock.tick_at(63);
    let after_twenty_unchanged = source.checks_of(&share);
    clock.tick_at(66);
    let at_twice_the_interval = source.checks_of(&share);
    source.change(&share, &["listed", "added elsewhere"]);
    clock.tick_at(72);
    clock.tick_at(75);

    assert_eq!((after_twenty_unchanged, at_twice_the_interval), (20, 21));
    assert_eq!(source.loads_of(&share), 2);
    assert_eq!(source.checks_of(&share), 23);
}

#[test]
fn an_answer_after_navigation_is_ignored_while_the_current_column_s_answer_applies() {
    let source = Rc::new(Folders::default());
    let share = source.add("/net/share", network(1), Some(Watching::Live));
    let sub = source.add("/net/share/sub", network(2), Some(Watching::Live));
    let other = source.add("/net/share/other", network(3), Some(Watching::Live));
    let (browser, clock) = browser_on(&source);
    browser.navigate(share.clone());
    browser.show_child(0, sub.clone());
    source.hold_checks.set(true);

    clock.tick_at(15);
    browser.show_child(0, other.clone());
    source.change(&share, &["listed", "added elsewhere"]);
    source.change(&sub, &["listed", "added elsewhere"]);
    source.answer_held_checks();

    assert_eq!(
        (
            source.loads_of(&share),
            source.loads_of(&sub),
            source.loads_of(&other)
        ),
        (2, 1, 1)
    );
    assert_eq!(names_at(&browser, 0), ["added elsewhere", "listed"]);
}

fn operation_scope(row: &str, folder: std::path::PathBuf) -> OperationScope {
    match row {
        "renaming the folder" => OperationScope {
            roots: vec![folder],
            folders: Vec::new(),
        },
        _ => OperationScope {
            roots: vec![folder.join("listed")],
            folders: vec![folder],
        },
    }
}

#[test]
fn checks_skip_folders_an_operation_changes_and_resume_after_it() {
    let rows = [
        ("renaming the folder", "/freshness/renamed"),
        ("renaming inside the folder", "/freshness/written"),
    ];
    assert_rows(rows, |(row, path)| {
        let source = Rc::new(Folders::default());
        let share = source.add(path, network(1), Some(Watching::Live));
        let (browser, clock) = browser_on(&source);
        browser.navigate(share.clone());
        source.change(&share, &["listed", "added elsewhere"]);
        let folder = share.native_path().expect("a native folder").to_path_buf();
        let activity = file_activity::begin(operation_scope(row, folder));

        clock.tick_at(15);
        let during = (source.checks_of(&share), source.loads_of(&share));
        drop(activity);
        clock.tick_at(30);

        let got = (during, source.checks_of(&share), source.loads_of(&share));
        (got != ((0, 1), 1, 2)).then(|| format!("{row}: got {got:?}"))
    });
}
