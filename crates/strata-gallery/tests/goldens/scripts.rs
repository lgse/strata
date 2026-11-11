//! The script runner: each `tests/scripts/<id>/<name>.json` runs headless through the gallery
//! (`--scene <scene> --script <file> --tree <out>`). It fails when the script is malformed or
//! names no feature-map bullets, when Rune rejects it or reports key warnings, when an
//! `expect` entry doesn't hold, or when the accessibility tree it ends on differs from
//! `tests/golden/scripts/<id>/<name>.json`. `docs/testing.md` describes the format.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use strata_gallery::scenes::Page;

use crate::support::{self, Mode};

const SCRIPT_KEYS: &[&str] = &[
    "bullets", "scene", "events", "expect", "displays", "pointer",
];
const EXPECT_KEYS: &[&str] = &[
    "key", "role", "name", "value", "states", "focused", "absent",
];

/// A script's checked header.
#[derive(Debug)]
pub struct Manifest {
    pub bullets: Vec<String>,
    pub scene: String,
    pub expect: Vec<Value>,
}

/// Checks a script's shape against the page whose folder holds it.
pub fn parse(script: &Value, page: &Page) -> Result<Manifest, String> {
    let obj = script
        .as_object()
        .ok_or("a script is an object with `bullets`, `scene` and `events`")?;
    if let Some(k) = obj.keys().find(|k| !SCRIPT_KEYS.contains(&k.as_str())) {
        return Err(format!("unknown key {k:?}; a script takes {SCRIPT_KEYS:?}"));
    }
    let bullets = obj
        .get("bullets")
        .and_then(Value::as_array)
        .ok_or("`bullets` must list the feature-map Behavior bullets the script checks")?;
    if bullets.is_empty() {
        return Err("`bullets` is empty: name the feature-map Behavior bullets this checks".into());
    }
    let bullets = bullets
        .iter()
        .map(|b| match b.as_str() {
            Some(s) if is_bullet(s) => Ok(s.to_owned()),
            _ => Err(format!(
                "bullet {b} is not `<node-id>: <bullet text or prefix>`"
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let scene = obj
        .get("scene")
        .and_then(Value::as_str)
        .ok_or("`scene` must name a scene")?;
    let known = page
        .scenes
        .iter()
        .chain(page.review)
        .any(|(n, _)| *n == scene);
    if !known {
        return Err(format!(
            "scene {scene:?} is not one of page {}'s scenes",
            page.id
        ));
    }
    match obj.get("events").and_then(Value::as_array) {
        Some(events) if !events.is_empty() => {}
        _ => return Err("`events` must be a non-empty list of Rune script events".into()),
    }
    let expect = match obj.get("expect") {
        None => Vec::new(),
        Some(Value::Array(list)) => list.clone(),
        Some(_) => return Err("`expect` must be a list".into()),
    };
    for e in &expect {
        check_expect_shape(e)?;
    }
    Ok(Manifest {
        bullets,
        scene: scene.to_owned(),
        expect,
    })
}

/// `<node-id>: <text>`, where a node id is `/`-separated segments of `a-z`, `0-9` and `-`.
fn is_bullet(s: &str) -> bool {
    let Some((node, text)) = s.split_once(": ") else {
        return false;
    };
    let segment = |seg: &str| {
        !seg.is_empty()
            && seg
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    };
    node.split('/').all(segment) && text.chars().next().is_some_and(|c| !c.is_whitespace())
}

fn check_expect_shape(e: &Value) -> Result<(), String> {
    let obj = e
        .as_object()
        .ok_or_else(|| format!("expect entry {e} is not an object"))?;
    if let Some(k) = obj.keys().find(|k| !EXPECT_KEYS.contains(&k.as_str())) {
        return Err(format!(
            "expect entry {e}: unknown key {k:?}; it takes {EXPECT_KEYS:?}"
        ));
    }
    if !obj.get("key").is_some_and(Value::is_string) {
        return Err(format!("expect entry {e} needs a `key`"));
    }
    Ok(())
}

/// Runs `script` from `scene` and returns the accessibility tree it ends on, stripped.
pub fn run(script: &Path, scene: &str, tree_out: &Path) -> Result<Value, String> {
    std::fs::create_dir_all(tree_out.parent().expect("tree directory")).expect("tree directory");
    let mut cmd = support::gallery();
    // Scripts run at scale 2, Rune's headless default, named so a change of default can't
    // move what a script's expectations (a zoom level, a drawn size) see.
    cmd.args(["--scene", scene, "--theme", support::theme_of(scene)])
        .args(["--scale", "2"])
        .arg("--script")
        .arg(script)
        .arg("--tree")
        .arg(tree_out);
    support::run(cmd)?;
    let text = std::fs::read_to_string(tree_out).map_err(|e| format!("no tree written: {e}"))?;
    let tree: Value = serde_json::from_str(&text).map_err(|e| format!("tree: {e}"))?;
    match tree.get("warnings").and_then(Value::as_array) {
        Some(w) if !w.is_empty() => return Err(format!("Rune reported key warnings: {w:?}")),
        _ => {}
    }
    let mut a11y = tree
        .get("accessibility")
        .cloned()
        .ok_or("the tree has no accessibility")?;
    support::strip(&mut a11y);
    Ok(a11y)
}

/// Every `expect` entry that doesn't hold in `tree`.
pub fn unmet(tree: &Value, expect: &[Value]) -> Vec<String> {
    expect.iter().filter_map(|e| holds(tree, e).err()).collect()
}

fn holds(tree: &Value, e: &Value) -> Result<(), String> {
    let key = e["key"].as_str().unwrap_or_default();
    let node = find(tree, key);
    if e["absent"].as_bool() == Some(true) {
        return match node {
            None => Ok(()),
            Some(_) => Err(format!("{key:?} should be absent, but it is in the tree")),
        };
    }
    let node = node.ok_or_else(|| format!("no node keyed {key:?} in the final tree"))?;
    for field in ["role", "name", "value"] {
        if let Some(want) = e.get(field)
            && node.get(field) != Some(want)
        {
            return Err(format!(
                "{key:?}: {field} is {}, expected {want}",
                node[field]
            ));
        }
    }
    let states: Vec<&str> = node["states"]
        .as_array()
        .map_or(Vec::new(), |s| s.iter().filter_map(Value::as_str).collect());
    for want in e["states"].as_array().into_iter().flatten() {
        let want = want.as_str().unwrap_or_default();
        if !states.contains(&want) {
            return Err(format!("{key:?} lacks state {want:?}; it has {states:?}"));
        }
    }
    if let Some(focused) = e["focused"].as_bool()
        && states.contains(&"focused") != focused
    {
        return Err(format!(
            "{key:?} focused is {}, expected {focused}",
            !focused
        ));
    }
    Ok(())
}

fn find<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    if v["key"] == key {
        return Some(v);
    }
    v["children"].as_array()?.iter().find_map(|c| find(c, key))
}

fn read(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

pub fn check(id: &str) {
    let page = support::page(id);
    let mut failures = Vec::new();
    let mut stems = BTreeSet::new();
    for path in script_files(id, &mut failures) {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_owned();
        stems.insert(stem.clone());
        let bless_name = format!("{id}/{stem}");
        if support::mode(id, &bless_name) == Mode::Skip {
            continue;
        }
        let tree_out = support::fresh_dir("scripts-run", id).join(format!("{stem}.tree.json"));
        let result = read(&path)
            .and_then(|script| parse(&script, page))
            .and_then(|m| run(&path, &m.scene, &tree_out).map(|tree| (m, tree)));
        match result {
            Err(e) => failures.push(format!("script {}: {e}", path.display())),
            Ok((m, tree)) => {
                let misses = unmet(&tree, &m.expect);
                let blessing = support::mode(id, &bless_name) == Mode::Bless;
                for miss in &misses {
                    failures.push(format!("script {}: expect: {miss}", path.display()));
                }
                // Blessing never records an end state its own expectations reject.
                if blessing && !misses.is_empty() {
                    continue;
                }
                let actual = support::pretty(&tree);
                support::text_golden("scripts", id, &stem, &bless_name, &actual, &mut failures);
            }
        }
    }
    support::orphans("scripts", id, "json", &stems, &mut failures);
    support::report(id, "scripts", failures);
}

/// The page's scripts; anything else in its folder is a failure.
fn script_files(id: &str, failures: &mut Vec<String>) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(support::scripts_dir(id)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries {
        let path = entry.expect("script directory entry").path();
        if path.is_file() && path.extension().is_some_and(|e| e == "json") {
            out.push(path);
        } else {
            failures.push(format!(
                "{}: only `<name>.json` scripts belong here",
                path.display()
            ));
        }
    }
    out.sort();
    out
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/goldens/fixtures")
        .join(name)
}

/// The runner end to end, on the gallery's own navigation: pressing an index row opens its
/// page, and Escape returns to the index.
#[test]
fn a_script_drives_the_gallery_and_its_expectations_are_checked() {
    let tree_out = support::fresh_dir("scripts-run", "fixtures").join("index-escape.tree.json");
    let path = fixture("index-escape.json");
    let script = read(&path).expect("fixture parses");
    let tree = run(&path, "gallery-index-light", &tree_out).expect("fixture runs");
    let expect = script["expect"].as_array().expect("fixture expectations");
    assert_eq!(unmet(&tree, expect), Vec::<String>::new());
}

#[test]
fn a_script_opens_the_page_it_taps() {
    let tree_out = support::fresh_dir("scripts-run", "fixtures").join("index-open.tree.json");
    let path = fixture("index-open.json");
    let tree = run(&path, "gallery-index-light", &tree_out).expect("fixture runs");
    let opened = [
        json!({"key": "w1-title", "name": "Matched text"}),
        json!({"key": "index", "absent": true}),
    ];
    assert_eq!(unmet(&tree, &opened), Vec::<String>::new());
}

fn index() -> &'static Page {
    support::page("gallery")
}

fn manifest(bullets: Value) -> Value {
    json!({
        "bullets": bullets,
        "scene": "gallery-index-light",
        "events": [{"t": 0.1, "type": "tap", "key": "title"}],
    })
}

#[test]
fn a_script_without_bullets_is_rejected() {
    let empty = parse(&manifest(json!([])), index()).expect_err("empty bullets fail");
    assert!(empty.contains("empty"), "{empty}");
    let mut missing = manifest(json!([]));
    missing.as_object_mut().expect("object").remove("bullets");
    assert!(parse(&missing, index()).is_err());
}

#[test]
fn bullets_name_a_node_and_its_text() {
    let ok = manifest(json!([
        "browser/selection/click-modes: Shift+click extends the range"
    ]));
    let m = parse(&ok, index()).expect("a well-formed bullet passes");
    assert_eq!(m.bullets.len(), 1);
    for bad in [
        "Shift+click extends",
        "browser/Selection: x",
        "browser//x: y",
        "browser/x:  y",
        "browser/x:",
    ] {
        assert!(
            parse(&manifest(json!([bad])), index()).is_err(),
            "{bad:?} should fail"
        );
    }
}

#[test]
fn a_script_must_start_from_its_own_pages_scene_and_have_events() {
    let mut other = manifest(json!(["a/b: c"]));
    other["scene"] = json!("c1-empty-light");
    assert!(parse(&other, index()).is_err());
    let mut no_events = manifest(json!(["a/b: c"]));
    no_events["events"] = json!([]);
    assert!(parse(&no_events, index()).is_err());
    let mut typo = manifest(json!(["a/b: c"]));
    typo["expects"] = json!([]);
    assert!(parse(&typo, index()).is_err());
    let mut bad_expect = manifest(json!(["a/b: c"]));
    bad_expect["expect"] = json!([{"role": "button"}]);
    assert!(parse(&bad_expect, index()).is_err());
}
