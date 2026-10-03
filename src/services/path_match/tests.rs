// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

use super::{Frecency, PathMatcher, PathQuery, TextScore, rank};
use crate::services::search::fold_for_search;

fn score(query: &str, path: &str) -> Option<TextScore> {
    let folded = fold_for_search(path);
    let name_start = folded.rfind('/').map_or(0, |slash| slash + 1);
    PathMatcher::new(&PathQuery::parse(query)).score(&folded, name_start)
}

fn ranked<'a>(query: &str, paths: &[&'a str]) -> Vec<&'a str> {
    let mut scored: Vec<_> = paths
        .iter()
        .filter_map(|path| score(query, path).map(|score| (rank(score, 0), *path)))
        .collect();
    scored.sort_by_key(|(rank, _)| std::cmp::Reverse(*rank));
    scored.into_iter().map(|(_, path)| path).collect()
}

fn highlighted<'a>(query: &str, path: &'a str) -> Vec<&'a str> {
    let name_start = path.rfind('/').map_or(0, |slash| slash + 1);
    PathMatcher::new(&PathQuery::parse(query))
        .highlight(path, name_start)
        .into_iter()
        .map(|range| &path[range])
        .collect()
}

#[test]
fn folder_terms_narrow_matches_in_any_order() {
    let paths = [
        "git/trading/README.md",
        "git/notes/README.md",
        "archive/trading/README.md",
        "git/trading/src/main.rs",
    ];
    for query in ["git trading readme", "readme trading git"] {
        assert_eq!(ranked(query, &paths), ["git/trading/README.md"], "{query}");
    }
    assert_eq!(
        ranked("GIT Trad READ", &paths),
        ["git/trading/README.md", "git/notes/README.md"]
    );
}

#[test]
fn terms_matching_the_name_outrank_terms_matching_only_folders() {
    let paths = [
        "readme-drafts/notes.txt",
        "docs/a/b/c/rdme.txt",
        "docs/README.md",
    ];
    assert_eq!(
        ranked("readme", &paths),
        ["docs/README.md", "readme-drafts/notes.txt"]
    );
    assert_eq!(
        ranked("docs rdme", &paths),
        ["docs/a/b/c/rdme.txt", "docs/README.md"]
    );
}

#[test]
fn closer_matches_rank_above_scattered_ones() {
    let paths = ["photos/report-final.pdf", "reports/photo.pdf"];
    assert_eq!(
        ranked("report", &paths),
        ["photos/report-final.pdf", "reports/photo.pdf"]
    );
    let scattered = score("report", "r/e/p/o/r/t.txt").expect("scattered match");
    let close = score("report", "a/report.txt").expect("close match");
    assert!(scattered < close);
}

#[test]
fn fzf_operators_constrain_terms() {
    assert!(score("^src", "src/lib.rs").is_some());
    assert!(score("^src", "app/src/lib.rs").is_none());
    assert!(score(".rs$", "src/lib.rs").is_some());
    assert!(score(".rs$", "src/lib.rs.bak").is_none());
    assert!(score("'lib", "src/lib.rs").is_some());
    assert!(score("'lib", "src/l_i_b.rs").is_none());
    assert!(score("lib !test", "src/lib.rs").is_some());
    assert!(score("lib !test", "tests/lib.rs").is_none());
    assert!(score("!test", "src/lib.rs").is_some());
}

#[test]
fn queries_without_terms_match_nothing() {
    assert!(score("  ", "src/lib.rs").is_none());
    assert!(score("! ^ $", "src/lib.rs").is_none());
}

#[test]
fn highlights_prefer_the_name_and_cover_folder_terms() {
    assert_eq!(
        highlighted("git trading readme", "git/trading/README.md"),
        ["git", "trading", "README"]
    );
    assert_eq!(highlighted("read", "readers/notes/README.md"), ["READ"]);
    assert_eq!(highlighted("lib !test", "src/lib.rs"), ["lib"]);
    assert!(highlighted("zzz", "src/lib.rs").is_empty());
}

#[test]
fn highlights_map_folded_text_back_to_displayed_characters() {
    assert_eq!(highlighted("ärger", "Über/ÄRGER.txt"), ["ÄRGER"]);
    assert_eq!(highlighted("über ä", "Über/Ärger.txt"), ["Über", "Ä"]);
    assert_eq!(highlighted("i̇stanbul", "İstanbul.txt"), ["İstanbul"]);
}

fn frecency(root: &str, folders: &[(&str, f64)]) -> Frecency {
    Frecency::within(
        Path::new(root),
        folders
            .iter()
            .map(|(folder, frecency)| (PathBuf::from(folder), *frecency)),
    )
}

#[test]
fn folders_use_their_own_frecency_and_files_their_containing_folder() {
    let history = frecency("/home/me", &[("/home/me/git/trading", 40.0)]);
    let folder = history.bias(Path::new("/home/me/git/trading"), true);
    let file = history.bias(Path::new("/home/me/git/trading/README.md"), false);
    assert!(folder > 0);
    assert_eq!(folder, file);
    assert_eq!(history.bias(Path::new("/home/me/git/notes"), true), 0);
}

#[test]
fn nearest_visited_ancestor_decays_with_distance() {
    let history = frecency(
        "/home/me",
        &[("/home/me/git", 40.0), ("/home/me/git/a/b", 1.0)],
    );
    let direct = history.bias(Path::new("/home/me/git/file"), false);
    let one_level = history.bias(Path::new("/home/me/git/a/file"), false);
    let two_levels = history.bias(Path::new("/home/me/git/a/c/file"), false);
    assert!(direct > one_level && one_level > two_levels && two_levels > 0);
    let nearer_but_rarer = history.bias(Path::new("/home/me/git/a/b/file"), false);
    assert!(
        nearer_but_rarer < one_level,
        "the nearest visit counts, not the largest"
    );
}

#[test]
fn only_folders_below_the_root_bias_results() {
    let history = frecency(
        "/home/me/git",
        &[
            ("/home/me", 1_000.0),
            ("/home/me/git", 1_000.0),
            ("/srv/git/x", 1_000.0),
        ],
    );
    assert_eq!(history.bias(Path::new("/home/me/git/x/file"), false), 0);
    assert_eq!(history.bias(Path::new("/home/me/git/x"), true), 0);
}

#[test]
fn frecency_orders_similar_matches_without_crossing_the_name_tier() {
    let history = frecency(
        "/home/me",
        &[
            ("/home/me/old/trading", 0.25),
            ("/home/me/git/trading", 10_000.0),
        ],
    );
    let ranked_with = |path: &str| {
        rank(
            score("trading readme", path).expect("both terms match"),
            history.bias(&Path::new("/home/me").join(path), false),
        )
    };
    assert!(ranked_with("git/trading/README.md") > ranked_with("old/trading/README.md"));
    assert!(
        ranked_with("old/trading/README.md") > ranked_with("git/trading/readme-notes/plan.txt"),
        "a name match beats any frecency"
    );
}

#[test]
fn name_highlights_index_the_displayed_file_name() {
    let mut matcher = PathMatcher::new(&PathQuery::parse("trading read"));
    let root = Path::new("/home/me/git");
    assert_eq!(
        matcher.name_highlight(root, Path::new("/home/me/git/trading/README.md")),
        vec![0..4]
    );
    assert!(
        matcher
            .name_highlight(root, Path::new("/home/me/git/trading/notes"))
            .is_empty()
    );
}
