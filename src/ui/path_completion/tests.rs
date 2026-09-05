// SPDX-License-Identifier: GPL-3.0-or-later

use std::{fs, path::Path};

use tempfile::tempdir;

use super::{CompletionCandidate, longest_common_prefix, suggest_completions};

#[test]
fn suggest_completions_for_home_shorthand() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path();

    fs::create_dir_all(home.join("Documents")).expect("create Documents");
    fs::create_dir_all(home.join("Downloads")).expect("create Downloads");
    fs::create_dir_all(home.join("Desktop")).expect("create Desktop");
    fs::create_dir_all(home.join(".config")).expect("create .config");
    fs::write(home.join("notes.txt"), b"hello").expect("write file");

    let tilde = suggest_completions("~", None, home, false);
    assert_eq!(
        tilde,
        vec![CompletionCandidate {
            display_name: "~/".to_owned(),
            replacement: "~/".to_owned(),
            is_dir: true,
        }]
    );

    let home_slash = suggest_completions("~/", None, home, false);
    let names: Vec<_> = home_slash.iter().map(|c| c.display_name.as_str()).collect();
    assert!(names.contains(&"Desktop/"));
    assert!(names.contains(&"Documents/"));
    assert!(names.contains(&"Downloads/"));
    assert!(names.contains(&"notes.txt"));
    assert!(!names.contains(&".config/"));

    let with_hidden = suggest_completions("~/", None, home, true);
    let hidden_names: Vec<_> = with_hidden
        .iter()
        .map(|c| c.display_name.as_str())
        .collect();
    assert!(hidden_names.contains(&".config/"));

    let prefix_match = suggest_completions("~/Doc", None, home, false);
    assert_eq!(
        prefix_match,
        vec![CompletionCandidate {
            display_name: "Documents/".to_owned(),
            replacement: "~/Documents/".to_owned(),
            is_dir: true,
        }]
    );

    let hidden_prefix = suggest_completions("~/.co", None, home, false);
    assert_eq!(
        hidden_prefix,
        vec![CompletionCandidate {
            display_name: ".config/".to_owned(),
            replacement: "~/.config/".to_owned(),
            is_dir: true,
        }]
    );
}

#[test]
fn suggest_completions_for_absolute_paths() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path();

    fs::create_dir_all(root.join("sub/alpha")).expect("create alpha");
    fs::create_dir_all(root.join("sub/beta")).expect("create beta");
    fs::write(root.join("sub/albatross.txt"), b"bird").expect("write albatross");

    let sub_str = root.join("sub").to_string_lossy().into_owned();
    let sub_slash = format!("{sub_str}/");
    let completions = suggest_completions(&sub_slash, None, Path::new("/dummy"), false);
    let names: Vec<_> = completions
        .iter()
        .map(|c| c.display_name.as_str())
        .collect();
    assert!(names.contains(&"alpha/"));
    assert!(names.contains(&"beta/"));
    assert!(names.contains(&"albatross.txt"));

    let prefix_query = format!("{sub_str}/al");
    let al_completions = suggest_completions(&prefix_query, None, Path::new("/dummy"), false);
    assert_eq!(al_completions.len(), 2);
    assert_eq!(al_completions[0].display_name, "alpha/");
    assert_eq!(al_completions[0].replacement, format!("{sub_str}/alpha/"));
    assert!(al_completions[0].is_dir);
    assert_eq!(al_completions[1].display_name, "albatross.txt");
    assert_eq!(
        al_completions[1].replacement,
        format!("{sub_str}/albatross.txt")
    );
    assert!(!al_completions[1].is_dir);
}

#[test]
fn suggest_completions_for_relative_paths() {
    let temp = tempdir().expect("tempdir");
    let current = temp.path();

    fs::create_dir_all(current.join("photos/vacation")).expect("create vacation");
    fs::create_dir_all(current.join("photos/work")).expect("create work");

    let relative_slash = suggest_completions("photos/", Some(current), Path::new("/dummy"), false);
    let names: Vec<_> = relative_slash
        .iter()
        .map(|c| c.display_name.as_str())
        .collect();
    assert_eq!(names, vec!["vacation/", "work/"]);
    assert_eq!(relative_slash[0].replacement, "photos/vacation/");

    let relative_prefix =
        suggest_completions("photos/va", Some(current), Path::new("/dummy"), false);
    assert_eq!(
        relative_prefix,
        vec![CompletionCandidate {
            display_name: "vacation/".to_owned(),
            replacement: "photos/vacation/".to_owned(),
            is_dir: true,
        }]
    );
}

#[test]
fn suggest_completions_handles_empty_and_nonexistent_gracefully() {
    let temp = tempdir().expect("tempdir");
    let current = temp.path();

    let empty = suggest_completions("", None, Path::new("/dummy"), false);
    assert!(empty.is_empty());

    let nonexistent = suggest_completions(
        "/definitely/not/a/real/path",
        None,
        Path::new("/dummy"),
        false,
    );
    assert!(nonexistent.is_empty());

    let empty_with_current = suggest_completions("", Some(current), Path::new("/dummy"), false);
    assert!(empty_with_current.is_empty());
}

#[test]
fn longest_common_prefix_computes_shared_start() {
    assert_eq!(longest_common_prefix(&[]), None);
    assert_eq!(
        longest_common_prefix(&["~/Documents/".to_owned()]),
        Some("~/Documents/".to_owned())
    );
    assert_eq!(
        longest_common_prefix(&[
            "~/Documents/notes1.txt".to_owned(),
            "~/Documents/notes2.txt".to_owned(),
        ]),
        Some("~/Documents/notes".to_owned())
    );
    assert_eq!(
        longest_common_prefix(&["/var/log/".to_owned(), "/usr/bin/".to_owned(),]),
        Some("/".to_owned())
    );
    assert_eq!(
        longest_common_prefix(&["abc".to_owned(), "xyz".to_owned(),]),
        None
    );
}
