//! The page registry holds together: every page has golden tests, scene names follow the
//! `<id>-…-light|dark` rule and open their own page, and every golden or script folder
//! belongs to a page. These guard what parallel work packages add.

use std::collections::BTreeSet;

use strata_gallery::Gallery;
use strata_gallery::scenes::{self, Page};

use crate::support;

#[test]
fn every_page_has_golden_tests() {
    let ids: Vec<&str> = scenes::all().map(|p| p.id).collect();
    assert_eq!(
        crate::IDS,
        ids.as_slice(),
        "the pages! list in main.rs must match the registry"
    );
}

#[test]
fn page_ids_are_unique_and_lowercase() {
    let mut seen = BTreeSet::new();
    for p in scenes::all() {
        assert!(seen.insert(p.id), "page id {:?} appears twice", p.id);
        let lower =
            p.id.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
        assert!(
            lower && p.id.starts_with(|c: char| c.is_ascii_lowercase()),
            "bad id {:?}",
            p.id
        );
    }
}

#[test]
fn page_docs_exist() {
    for p in scenes::PAGES {
        let doc = p
            .doc
            .unwrap_or_else(|| panic!("page {} names no work package doc", p.id));
        assert!(
            support::repo().join(doc).is_file(),
            "page {}: no file {doc}",
            p.id
        );
    }
}

/// Every work package has landed, so every page is built: it has golden scenes, and the index
/// marks it so. A page added without them fails here.
#[test]
fn every_page_is_built() {
    let unbuilt: Vec<&str> = scenes::PAGES
        .iter()
        .filter(|p| !p.implemented())
        .map(|p| p.id)
        .collect();
    assert!(
        unbuilt.is_empty(),
        "pages with no golden scenes: {unbuilt:?}"
    );
}

fn every_scene() -> impl Iterator<Item = (&'static Page, &'static str, fn(&mut Gallery))> {
    scenes::all().flat_map(|p| {
        p.scenes
            .iter()
            .chain(p.review)
            .map(move |&(n, s)| (p, n, s))
    })
}

#[test]
fn scene_names_are_unique_prefixed_and_come_in_light_and_dark() {
    let names: Vec<&str> = every_scene().map(|(_, n, _)| n).collect();
    let unique: BTreeSet<&str> = names.iter().copied().collect();
    assert_eq!(
        unique.len(),
        names.len(),
        "scene names must be unique across the gallery"
    );
    for (page, name, _) in every_scene() {
        assert!(
            name.starts_with(&format!("{}-", page.id)),
            "{name:?} must start with `{}-`",
            page.id
        );
        let twin = if let Some(stem) = name.strip_suffix("-light") {
            format!("{stem}-dark")
        } else if let Some(stem) = name.strip_suffix("-dark") {
            format!("{stem}-light")
        } else {
            panic!("{name:?} must end in -light or -dark");
        };
        assert!(unique.contains(twin.as_str()), "{name:?} has no {twin:?}");
    }
}

#[test]
fn a_scene_opens_its_own_page_in_its_own_theme() {
    for (page, name, scene) in every_scene() {
        let mut g = Gallery::default();
        scene(&mut g);
        let open = scenes::open_page(&g).id;
        assert_eq!(open, page.id, "scene {name:?} opens page {open:?}");
        assert_eq!(
            g.dark,
            name.ends_with("-dark"),
            "scene {name:?} sets the wrong theme"
        );
    }
}

#[test]
fn golden_and_script_folders_belong_to_pages() {
    let ids: BTreeSet<&str> = scenes::all().map(|p| p.id).collect();
    let golden = support::repo().join("tests/golden");
    for kind in std::fs::read_dir(&golden).expect("tests/golden") {
        let kind = kind.expect("entry").path();
        let name = kind
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_owned();
        assert!(
            ["a11y", "png", "scripts"].contains(&name.as_str()),
            "tests/golden/{name}: goldens go under a11y/, png/ or scripts/"
        );
        assert_page_folders(&kind, &ids);
    }
    let scripts = support::repo().join("tests/scripts");
    if scripts.exists() {
        assert_page_folders(&scripts, &ids);
    }
}

fn assert_page_folders(dir: &std::path::Path, ids: &BTreeSet<&str>) {
    for entry in std::fs::read_dir(dir).expect("folder") {
        let path = entry.expect("entry").path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        assert!(
            path.is_dir() && ids.contains(name),
            "{}: not a page folder",
            path.display()
        );
    }
}

#[test]
fn bless_filters_name_a_page_a_scene_or_a_script() {
    let on = Some("1");
    let bless = |filter, id, name| support::blessing(on, filter, id, name);
    assert!(bless(None, "c1", "c1-empty-dark"));
    assert!(bless(Some(""), "c10", "c10-x-dark"));
    assert!(bless(Some("c1"), "c1", "c1-empty-dark"));
    assert!(!bless(Some("c1"), "c10", "c10-empty-dark"));
    assert!(!bless(Some("c1"), "c14a", "c14a-fit-dark"));
    assert!(bless(Some("c1-empty-dark"), "c1", "c1-empty-dark"));
    assert!(!bless(Some("c1-empty-dark"), "c1", "c1-empty-light"));
    assert!(bless(Some("c5/stem"), "c5", "c5/stem"));
    assert!(!bless(Some("c5/stem"), "c5", "c5/commit"));
    assert!(!support::blessing(None, None, "c1", "c1-empty-dark"));
    assert!(!support::blessing(Some("0"), None, "c1", "c1-empty-dark"));
}

/// `mise run bless <filter>` always runs this, so a filter that names no page, golden scene or
/// script fails instead of blessing nothing.
#[test]
fn bless_filter_names_a_page_scene_or_script() {
    let blessing = std::env::var("RUNE_BLESS").is_ok_and(|v| v != "0");
    let filter = std::env::var("STRATA_BLESS").unwrap_or_default();
    if blessing && !filter.trim().is_empty() {
        support::filter_target(filter.trim(), support::script_stems)
            .unwrap_or_else(|e| panic!("{e}"));
    }
}

#[test]
fn a_bless_filter_must_name_something_that_exists() {
    let scripts = |id: &str| match id {
        "c5" => vec!["stem-selection".to_owned()],
        _ => Vec::new(),
    };
    assert!(support::filter_target("gallery", scripts).is_ok());
    assert!(support::filter_target("c1", scripts).is_ok());
    assert!(support::filter_target("gallery-index-dark", scripts).is_ok());
    assert!(support::filter_target("c5/stem-selection", scripts).is_ok());
    assert!(support::filter_target("c99", scripts).is_err());
    assert!(support::filter_target("gallery-index-dim", scripts).is_err());
    assert!(support::filter_target("c1-empty-dark", scripts).is_err());
    assert!(support::filter_target("c5/commit", scripts).is_err());
}
