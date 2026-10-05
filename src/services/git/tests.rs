// SPDX-License-Identifier: MIT

use super::*;
use std::{fs, os::unix::ffi::OsStrExt, process::Command};
use tempfile::tempdir;

fn git(path: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(arguments)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        arguments.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn porcelain_parser_preserves_statuses_renames_and_non_utf8_paths() {
    let output = b" M src/main.rs\0?? untracked/\0!! target/\0R  new.txt\0old.txt\0UU conflict.rs\0?? bad\xffname\0";
    let parsed = parse_porcelain_v1_z(output);

    assert_eq!(
        parsed.status_map.get(Path::new("src/main.rs")),
        Some(&GitStatus::Modified)
    );
    assert_eq!(
        parsed.status_map.get(Path::new("untracked")),
        Some(&GitStatus::Untracked)
    );
    assert_eq!(
        parsed.status_map.get(Path::new("target")),
        Some(&GitStatus::Ignored)
    );
    assert_eq!(
        parsed.status_map.get(Path::new("new.txt")),
        Some(&GitStatus::Modified)
    );
    assert_eq!(
        parsed.status_map.get(Path::new("conflict.rs")),
        Some(&GitStatus::Modified)
    );
    assert!(!parsed.status_map.contains_key(Path::new("old.txt")));
    assert_eq!(
        parsed
            .status_map
            .get(Path::new(std::ffi::OsStr::from_bytes(b"bad\xffname"))),
        Some(&GitStatus::Untracked)
    );
    assert_eq!(
        parsed.collapsed_dirs,
        vec![
            (PathBuf::from("untracked"), GitStatus::Untracked),
            (PathBuf::from("target"), GitStatus::Ignored),
        ]
    );
}

#[test]
fn directory_status_uses_descendants_and_collapsed_directories() {
    let status = RepoGitStatus {
        root: PathBuf::from("/repo"),
        head: Some(GitHead::Branch("main".to_owned())),
        status_map: HashMap::from([
            (PathBuf::from("src/ui/browser.rs"), GitStatus::Modified),
            (PathBuf::from("src/model/mod.rs"), GitStatus::Untracked),
            (PathBuf::from("docs/old.md"), GitStatus::Ignored),
            (PathBuf::from("scratch"), GitStatus::Untracked),
            (PathBuf::from("scratch/generated"), GitStatus::Ignored),
            (PathBuf::from("target"), GitStatus::Ignored),
        ]),
        collapsed_dirs: vec![
            (PathBuf::from("scratch"), GitStatus::Untracked),
            (PathBuf::from("scratch/generated"), GitStatus::Ignored),
            (PathBuf::from("target"), GitStatus::Ignored),
        ],
    };

    assert_eq!(
        status.status_for_child(Path::new("src"), true),
        Some(GitStatus::Modified)
    );
    assert_eq!(
        status.status_for_child(Path::new("src/model"), true),
        Some(GitStatus::Untracked)
    );
    assert_eq!(
        status.status_for_child(Path::new("docs"), true),
        Some(GitStatus::Ignored)
    );
    assert_eq!(
        status.status_for_child(Path::new("scratch/nested/file.txt"), false),
        Some(GitStatus::Untracked)
    );
    assert_eq!(
        status.status_for_child(Path::new("scratch/generated/output.js"), false),
        Some(GitStatus::Ignored)
    );
    assert_eq!(
        status.status_for_child(Path::new("target/debug/app"), false),
        Some(GitStatus::Ignored)
    );
    assert_eq!(status.status_for_child(Path::new("clean.txt"), false), None);
}

#[test]
fn repository_discovery_handles_worktree_files_and_nested_repository_rows() {
    let dir = tempdir().expect("tempdir");
    let repo = dir.path().join("repo");
    let nested = repo.join("vendor/nested");
    fs::create_dir_all(&nested).expect("create nested path");
    fs::create_dir(repo.join(".git")).expect("create repository marker");
    fs::write(nested.join(".git"), "gitdir: /tmp/example").expect("create worktree marker");

    assert_eq!(find_repo_root(&nested), Some(nested.clone()));
    assert_eq!(
        find_status_repo_root(&nested, true),
        Some(repo.clone()),
        "a nested repository row belongs to its parent repository"
    );
    assert_eq!(
        find_status_repo_root(&nested.join("file.rs"), false),
        Some(nested),
        "children inside the nested repository use the nested repository"
    );
    assert_eq!(find_repo_root(dir.path()), None);
    assert_eq!(find_status_repo_root(&repo, true), None);
}

#[test]
fn query_reads_the_branch_and_changes_from_a_linked_worktree() {
    let dir = tempdir().expect("tempdir");
    let repo = dir.path().join("main");
    let worktree = dir.path().join("linked");
    fs::create_dir(&repo).expect("create repository directory");
    git(&repo, &["init", "--quiet"]);
    git(&repo, &["config", "user.name", "Strata Tests"]);
    git(
        &repo,
        &["config", "user.email", "strata-tests@example.invalid"],
    );
    fs::write(repo.join("tracked.txt"), "initial").expect("write tracked file");
    git(&repo, &["add", "tracked.txt"]);
    git(&repo, &["commit", "--quiet", "-m", "initial"]);
    git(&repo, &["branch", "feature/worktree"]);
    git(
        &repo,
        &[
            "worktree",
            "add",
            "--quiet",
            worktree.to_str().expect("utf8 worktree path"),
            "feature/worktree",
        ],
    );
    fs::write(worktree.join("tracked.txt"), "changed").expect("modify worktree file");

    let status = query_git_status(&worktree).expect("query linked worktree");
    assert_eq!(status.root, worktree);
    assert_eq!(
        status.head,
        Some(GitHead::Branch("feature/worktree".to_owned()))
    );
    assert_eq!(
        status.status_for_child(Path::new("tracked.txt"), false),
        Some(GitStatus::Modified)
    );
}

#[test]
fn query_reports_unborn_and_detached_heads_with_collapsed_children() {
    let dir = tempdir().expect("tempdir");
    git(dir.path(), &["init", "--quiet"]);
    git(
        dir.path(),
        &[
            "symbolic-ref",
            "HEAD",
            "refs/heads/feature/a-very-long-branch-name-for-the-header",
        ],
    );
    fs::write(dir.path().join(".gitignore"), "ignored/\n").expect("write gitignore");
    git(dir.path(), &["add", ".gitignore"]);
    fs::create_dir_all(dir.path().join("untracked/nested")).expect("create untracked directory");
    fs::write(dir.path().join("untracked/nested/file.txt"), "new").expect("write untracked file");
    fs::create_dir_all(dir.path().join("ignored/cache")).expect("create ignored directory");
    fs::write(dir.path().join("ignored/cache/data"), "ignored").expect("write ignored file");

    let unborn = query_git_status(dir.path()).expect("query unborn repository");
    assert_eq!(
        unborn.head,
        Some(GitHead::Branch(
            "feature/a-very-long-branch-name-for-the-header".to_owned()
        ))
    );
    assert_eq!(
        unborn.status_for_child(Path::new("untracked/nested/file.txt"), false),
        Some(GitStatus::Untracked)
    );
    assert_eq!(
        unborn.status_for_child(Path::new("ignored/cache/data"), false),
        Some(GitStatus::Ignored)
    );

    git(dir.path(), &["config", "user.name", "Strata Tests"]);
    git(
        dir.path(),
        &["config", "user.email", "strata-tests@example.invalid"],
    );
    git(dir.path(), &["commit", "--quiet", "-m", "initial"]);
    git(dir.path(), &["checkout", "--quiet", "--detach"]);

    let detached = query_git_status(dir.path()).expect("query detached repository");
    let Some(GitHead::Detached(commit)) = detached.head else {
        panic!("expected detached Git head");
    };
    assert_eq!(commit.len(), 8);
    assert!(commit.bytes().all(|byte| byte.is_ascii_hexdigit()));
}
