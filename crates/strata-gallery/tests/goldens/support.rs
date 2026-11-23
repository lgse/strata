// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use rune::{Env, Runtime};
use serde_json::Value;
use strata_gallery::Gallery;
use strata_gallery::scenes::{self, Page};

/// The repository root.
pub fn repo() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    root.canonicalize().unwrap_or(root)
}

/// `tests/golden/<kind>/<id>/`.
pub fn golden_dir(kind: &str, id: &str) -> PathBuf {
    repo().join("tests/golden").join(kind).join(id)
}

/// `tests/scripts/<id>/`.
pub fn scripts_dir(id: &str) -> PathBuf {
    repo().join("tests/scripts").join(id)
}

/// Where fresh output goes for review when a golden differs.
pub fn fresh_dir(kind: &str, id: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("goldens")
        .join(kind)
        .join(id)
}

pub fn page(id: &str) -> &'static Page {
    scenes::find(id).unwrap_or_else(|| panic!("no gallery page {id:?}"))
}

pub fn blessing(rune_bless: Option<&str>, filter: Option<&str>, id: &str, name: &str) -> bool {
    if !bless_on(rune_bless) {
        return false;
    }
    match filter.map(str::trim) {
        None | Some("") => true,
        Some(f) => f == id || f == name,
    }
}

/// What a bless filter names, or why it names nothing. `scripts` lists a page's script stems.
pub fn filter_target(filter: &str, scripts: impl Fn(&str) -> Vec<String>) -> Result<(), String> {
    let id_of = |f: &str| f.split(['-', '/']).next().unwrap_or_default().to_owned();
    let id = id_of(filter);
    let Some(page) = scenes::find(&id) else {
        return Err(format!("bless filter {filter:?}: no gallery page {id:?}"));
    };
    if filter == page.id {
        return Ok(());
    }
    if let Some(stem) = filter.strip_prefix(&format!("{id}/")) {
        return match scripts(&id).iter().any(|s| s == stem) {
            true => Ok(()),
            false => Err(format!(
                "bless filter {filter:?}: page {id} has no script {stem:?}"
            )),
        };
    }
    match page.scenes.iter().any(|(n, _)| *n == filter) {
        true => Ok(()),
        false => Err(format!(
            "bless filter {filter:?}: page {id} has no golden scene by that name (review scenes \
             have no goldens)"
        )),
    }
}

/// The stems of `tests/scripts/<id>/*.json`.
pub fn script_stems(id: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(scripts_dir(id)) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(str::to_owned))
        .collect()
}

fn bless_on(rune_bless: Option<&str>) -> bool {
    rune_bless.is_some_and(|v| v != "0")
}

/// What to do with one golden item in this run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Compare,
    Bless,
    /// Blessing is on and the filter names something else: neither compared nor written.
    Skip,
}

pub fn mode(id: &str, name: &str) -> Mode {
    let rune_bless = std::env::var("RUNE_BLESS").ok();
    let filter = std::env::var("STRATA_BLESS").ok();
    if !bless_on(rune_bless.as_deref()) {
        Mode::Compare
    } else if blessing(rune_bless.as_deref(), filter.as_deref(), id, name) {
        Mode::Bless
    } else {
        Mode::Skip
    }
}

/// Bounds and scroll offsets are layout, not semantics; shortcuts are written per platform.
pub fn strip(v: &mut Value) {
    if let Some(o) = v.as_object_mut() {
        for k in ["bounds", "scroll", "selection", "shortcut"] {
            o.remove(k);
        }
        if let Some(Value::Array(children)) = o.get_mut("children") {
            children.iter_mut().for_each(strip);
        }
    }
}

pub fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).expect("JSON serializes") + "\n"
}

pub const AT: f64 = 1.0;

/// Frames per second on the virtual clock, Rune's headless default.
const FPS: f64 = 60.0;

/// `AT` as `--at` takes it.
pub fn at_arg() -> String {
    format!("{AT}")
}

pub fn mount(scene: fn(&mut Gallery)) -> Runtime<Gallery> {
    let mut gallery = Gallery::default();
    scene(&mut gallery);
    let env = Env::new(gallery.dark, 1.0);
    let mut rt = strata_gallery::runtime(gallery, env);
    rt.frame(0.0);
    for _ in 0..(AT * FPS).round() as usize {
        rt.frame(1.0 / FPS);
    }
    rt
}

/// `light` or `dark`, from a scene name's suffix, for the binary's `--theme`.
pub fn theme_of(scene: &str) -> &'static str {
    if scene.ends_with("-dark") {
        "dark"
    } else {
        "light"
    }
}

/// Whether `scene` leaves the gallery dark.
pub fn is_dark(scene: fn(&mut Gallery)) -> bool {
    let mut g = Gallery::default();
    scene(&mut g);
    g.dark
}

pub const NO_WAYLAND: &str = "strata-test-no-wayland";

pub fn gallery() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_strata-gallery"));
    cmd.env_remove("DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .env("WAYLAND_DISPLAY", NO_WAYLAND)
        .env("RUNE_GPU_LOG", "1");
    if let Some(icd) = vulkan_icd() {
        cmd.env("VK_ICD_FILENAMES", icd);
    }
    cmd
}

fn vulkan_icd() -> Option<OsString> {
    if let Some(icd) = std::env::var_os("STRATA_VK_ICD").filter(|v| !v.is_empty()) {
        return Some(icd);
    }
    let dir = Path::new("/usr/share/vulkan/icd.d");
    let preferred = [
        format!("lvp_icd.{}.json", std::env::consts::ARCH),
        "lvp_icd.json".to_owned(),
    ];
    if let Some(icd) = preferred.iter().map(|n| dir.join(n)).find(|p| p.is_file()) {
        return Some(icd.into_os_string());
    }
    let mut lavapipe: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("lvp_icd") && n.ends_with(".json"))
        })
        .collect();
    lavapipe.sort();
    std::env::join_paths(lavapipe)
        .ok()
        .filter(|joined| !joined.is_empty())
}

/// Runs `cmd`; its stderr on success, or a description of the failure.
pub fn run(mut cmd: Command) -> Result<String, String> {
    let out = cmd
        .output()
        .map_err(|e| format!("could not start the gallery: {e}"))?;
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        Ok(stderr)
    } else {
        Err(format!(
            "the gallery exited with {}:\n{}",
            out.status,
            tail(&stderr, 20)
        ))
    }
}

/// The adapter Rune reported, for failure messages.
pub fn gpu_line(stderr: &str) -> &str {
    stderr
        .lines()
        .find(|l| l.starts_with("rune: GPU"))
        .unwrap_or("rune: GPU (not reported)")
}

fn tail(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

pub fn text_golden(
    kind: &str,
    id: &str,
    name: &str,
    bless_name: &str,
    actual: &str,
    failures: &mut Vec<String>,
) {
    let path = golden_dir(kind, id).join(format!("{name}.json"));
    match mode(id, bless_name) {
        Mode::Skip => {}
        Mode::Bless => write(&path, actual.as_bytes()),
        Mode::Compare => {
            let Ok(want) = std::fs::read_to_string(&path) else {
                failures.push(format!(
                    "{kind} {name}: missing golden; run `mise run bless {id}`"
                ));
                return;
            };
            if want.replace("\r\n", "\n") != actual {
                let fresh = fresh_dir(kind, id).join(format!("{name}.json"));
                write(&fresh, actual.as_bytes());
                let line = want
                    .lines()
                    .zip(actual.lines())
                    .position(|(a, b)| a != b)
                    .unwrap_or(want.lines().count().min(actual.lines().count()));
                failures.push(format!(
                    "{kind} {name}: differs from its golden at line {}\n  golden: {}\n  fresh:  {}",
                    line + 1,
                    path.display(),
                    fresh.display(),
                ));
            }
        }
    }
}

pub fn write(path: &Path, bytes: &[u8]) {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).expect("output directory");
    }
    std::fs::write(path, bytes).expect("output written");
}

pub fn orphans(
    kind: &str,
    id: &str,
    ext: &str,
    expected: &BTreeSet<String>,
    failures: &mut Vec<String>,
) {
    let dir = golden_dir(kind, id);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let page_mode = mode(id, id);
    for entry in entries {
        let path = entry.expect("golden directory entry").path();
        if page_mode == Mode::Skip {
            continue;
        }
        if path.is_dir() {
            failures.push(format!(
                "{kind}: {} is a directory; goldens are files directly in {}",
                path.display(),
                dir.display()
            ));
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_owned();
        if path.extension().is_some_and(|e| e == ext) && expected.contains(&stem) {
            continue;
        }
        if page_mode == Mode::Bless {
            std::fs::remove_file(&path).expect("stale golden removed");
        } else {
            failures.push(format!(
                "{kind}: {} has no scene or script behind it; remove it, or run \
                 `mise run bless {id}`",
                path.display()
            ));
        }
    }
}

/// Panics with every failure, so one run reports them all.
pub fn report(id: &str, kind: &str, failures: Vec<String>) {
    assert!(
        failures.is_empty(),
        "{} {kind} golden failure(s) on page {id}:\n\n{}\n\nAccept intended changes with \
         `mise run bless {id}`, then review every golden diff.",
        failures.len(),
        failures.join("\n\n"),
    );
}
