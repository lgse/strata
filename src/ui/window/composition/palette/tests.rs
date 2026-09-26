// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn command_search_supports_aliases_fuzzy_input_and_no_matches() {
    for (query, expected) in [
        ("preferences", Command::Settings),
        ("  MKDIR  ", Command::File(FileCommand::NewFolder)),
        ("trmnl", Command::Terminal),
        ("copy path", Command::File(FileCommand::CopyPaths)),
        ("hide sidebar", Command::Sidebar),
    ] {
        let matches = catalogue::matches(query);
        assert_eq!(COMMANDS[matches[0]].command, expected, "{query}");
    }
    assert!(catalogue::matches("zzzzzzzzz").is_empty());
    let matches = catalogue::matches("refresh");
    assert_eq!(COMMANDS[matches[0]].command, Command::Refresh);
}

#[test]
fn recent_commands_move_repeats_to_front_and_evict_oldest() {
    let mut recent = Vec::new();
    for index in 0..7 {
        record_recent(&mut recent, index);
    }
    assert_eq!(recent, [6, 5, 4, 3, 2]);
    record_recent(&mut recent, 4);
    assert_eq!(recent, [4, 6, 5, 3, 2]);
}
