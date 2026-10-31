// SPDX-License-Identifier: MIT

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::{
    DEFAULT_LAYOUT, QA_ENVIRONMENT, REPOSITORY, RUNNER_COMMANDS, RUNNER_OWNED_ENVIRONMENT, STEP_IF,
    STEP_MODES, STEP_UNLESS, STEP_VARIANTS, STEP_WHEN, XDG_DIRECTORIES,
    fixture::{expand_tree, fixed_fixture_name, gtk_settings, manifest},
    fs_check::FsStep,
    harness, protocol,
    run::run_scenario,
    unit_tests::UNIT_TESTS,
    variables::{run_name, runs},
};

const KEEPING_STEPS: [&str; 11] = [
    "layout",
    "flag",
    "note",
    "slice",
    "increment",
    "stem",
    "parent",
    "screenshot",
    "restart",
    "http",
    "foreign_stop",
];
const MODES: [(&str, &str); 3] = [("columns", "Columns"), ("icons", "Icons"), ("list", "List")];
/// Set by `./scripts/qa.sh` when scenarios are named, so a `skip` scenario still runs.
const NAMED: &str = "STRATA_QA_NAMED";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Header {
    pub(crate) scenario: String,
    pub(crate) modes: Vec<String>,
    #[serde(default)]
    pub(crate) prefs: Map<String, Value>,
    #[serde(default = "default_layout")]
    pub(crate) layout: String,
    pub(crate) mutation: Option<String>,
    pub(crate) known_bug: Option<String>,
    pub(crate) known_bug_step: Option<usize>,
    #[serde(default)]
    pub(crate) variants: Vec<Variant>,
    pub(crate) tree: Option<Value>,
    #[serde(default)]
    pub(crate) images: BTreeMap<String, Image>,
    #[serde(default)]
    pub(crate) gtk_settings: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) setup: Vec<Value>,
    #[serde(default)]
    pub(crate) teardown: Vec<Value>,
    #[serde(default)]
    pub(crate) env: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) session_bus: bool,
    #[serde(default)]
    pub(crate) portal: bool,
    pub(crate) input_method: Option<String>,
    pub(crate) fixture_root: Option<String>,
    pub(crate) skip: Option<String>,
    pub(crate) cost: Option<BTreeMap<String, f64>>,
    pub(crate) quality: Option<Value>,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Quality {
    pub(crate) impact: u8,
    pub(crate) coverage: u8,
    pub(crate) criticality: u8,
    pub(crate) diagnostic: u8,
    pub(crate) score: u8,
    pub(crate) note: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Image {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgb: [u8; 3],
    #[serde(default)]
    pub(crate) width_step: u32,
    pub(crate) thumbnail: Option<Thumbnail>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Thumbnail {
    pub(crate) cache: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgb: [u8; 3],
    pub(crate) mode: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Variant {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) modes: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) prefs: Map<String, Value>,
    pub(crate) layout: Option<String>,
    #[serde(default)]
    pub(crate) vars: BTreeMap<String, String>,
    pub(crate) tree: Option<Value>,
    #[serde(default)]
    pub(crate) setup: Vec<Value>,
    #[serde(default)]
    pub(crate) teardown: Vec<Value>,
}

fn default_layout() -> String {
    DEFAULT_LAYOUT.to_owned()
}

pub(crate) struct Scenario {
    pub(crate) header: Header,
    pub(crate) steps: Vec<(usize, Value)>,
}

pub(crate) fn scenario_files() -> Vec<PathBuf> {
    let directory = Path::new(REPOSITORY).join("tests/qa/scenarios");
    let mut files = Vec::new();
    for folder in fs::read_dir(&directory).expect("the scenario directory should be listable") {
        let folder = folder
            .expect("the scenario entry should be readable")
            .path();
        assert!(
            folder.is_dir(),
            "{}: scenarios live in a folder of tests/qa/scenarios",
            folder.display()
        );
        for file in fs::read_dir(&folder).expect("the scenario folder should be listable") {
            let file = file.expect("the scenario entry should be readable").path();
            if file
                .extension()
                .is_some_and(|extension| extension == "jsonl")
            {
                files.push(file);
            }
        }
    }
    files.sort();
    files
}

pub(crate) fn scenario_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_owned()
}

fn read_header(path: &Path) -> Result<Header, String> {
    let text = fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let first = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .ok_or("the scenario is empty")?;
    serde_json::from_str(first).map_err(|error| format!("line 1, the header: {error}"))
}

const SCENARIO_IGNORED: &str = "QA scenarios run in the pinned container through ./scripts/qa.sh";

pub(crate) fn planned_tests() -> Vec<harness::Test> {
    let mut tests: Vec<harness::Test> = UNIT_TESTS
        .iter()
        .map(|&(name, test)| harness::Test {
            name: name.to_owned(),
            group: None,
            ignored: None,
            skip: None,
            cost: None,
            exclusive: false,
            plan: None,
            run: Box::new(move || {
                test();
                harness::Outcome::Passed(None)
            }),
        })
        .collect();
    let named = std::env::var_os(NAMED).is_some_and(|named| named == "1");
    for path in scenario_files() {
        let scenario = scenario_name(&path);
        let header = match read_header(&path) {
            Ok(header) => header,
            Err(error) => {
                let error = format!("{}: {error}", path.display());
                tests.push(harness::Test {
                    name: scenario.clone(),
                    group: Some(scenario),
                    ignored: Some(SCENARIO_IGNORED),
                    skip: None,
                    cost: None,
                    exclusive: false,
                    plan: None,
                    run: Box::new(move || harness::Outcome::Failed(error)),
                });
                continue;
            }
        };
        for run in runs(&header) {
            let path = path.clone();
            let label = run.label.clone();
            tests.push(harness::Test {
                name: format!("{scenario}/{label}"),
                group: Some(scenario.clone()),
                ignored: Some(SCENARIO_IGNORED),
                skip: header
                    .skip
                    .as_ref()
                    .filter(|_| !named)
                    .map(|reason| harness::Skip {
                        reason: format!("skipped: {reason}"),
                        notice: format!("SKIPPED {scenario} (run it by name): {reason}"),
                    }),
                cost: header
                    .cost
                    .as_ref()
                    .and_then(|cost| cost.get(&label))
                    .copied(),
                exclusive: header.fixture_root.is_some(),
                plan: Some(format!("RUN {}", run_name(&scenario, &run))),
                run: Box::new(move || run_scenario(&path, &label)),
            });
        }
    }
    tests
}

pub(crate) fn load_scenario(path: &Path) -> Result<Scenario, String> {
    let text = fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut lines = text
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty());
    let (_, first) = lines.next().ok_or("the scenario is empty")?;
    let header: Header =
        serde_json::from_str(first).map_err(|error| format!("line 1, the header: {error}"))?;
    let expected = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    if header.scenario != expected {
        return Err(format!(
            "the header names {:?} but the file is {expected:?}",
            header.scenario
        ));
    }
    // `./scripts/qa.sh` reads `_` in a name as `-`.
    if !is_name(&header.scenario, '-') {
        return Err(format!(
            "a scenario name is lowercase words joined by hyphens, not {:?}",
            header.scenario
        ));
    }
    if let Some(quality) = &header.quality {
        check_quality(quality)?;
    }
    for mode in header.modes.iter().chain(
        header
            .variants
            .iter()
            .flat_map(|variant| variant.modes.iter().flatten()),
    ) {
        view_name(mode)?;
    }
    for tree in header.tree.iter().chain(
        header
            .variants
            .iter()
            .filter_map(|variant| variant.tree.as_ref()),
    ) {
        expand_tree(tree, &manifest())?;
    }
    for (index, step) in header
        .setup
        .iter()
        .chain(header.variants.iter().flat_map(|variant| &variant.setup))
        .enumerate()
    {
        let parsed: FsStep = serde_json::from_value(step.clone())
            .map_err(|error| format!("setup {}: invalid fs step: {error}", index + 1))?;
        if !parsed.fs.acts() || parsed.timeout_ms.is_some() || parsed.message.is_some() {
            return Err(format!(
                "setup {}: setup takes fs actions only, such as write or chmod",
                index + 1
            ));
        }
    }
    for (index, step) in header
        .teardown
        .iter()
        .chain(header.variants.iter().flat_map(|variant| &variant.teardown))
        .enumerate()
    {
        let parsed: FsStep = serde_json::from_value(step.clone())
            .map_err(|error| format!("teardown {}: invalid fs step: {error}", index + 1))?;
        if !parsed.fs.acts() || parsed.timeout_ms.is_some() || parsed.message.is_some() {
            return Err(format!(
                "teardown {}: teardown takes fs actions only, such as chmod",
                index + 1
            ));
        }
    }
    gtk_settings(&manifest().gtk_settings, &header.gtk_settings)?;
    check_cost(&header)?;
    if let Some(reason) = &header.skip
        && (reason.trim().is_empty() || header.mutation.is_some() || header.known_bug.is_some())
    {
        return Err(
            "skip gives a reason and is for a scenario without a mutation or known_bug".to_owned(),
        );
    }
    if header.portal && !header.session_bus {
        return Err("a portal run needs the header's session_bus".to_owned());
    }
    if header.input_method.is_some() && !header.session_bus {
        return Err("an input_method run needs the header's session_bus".to_owned());
    }
    if let Some(root) = &header.fixture_root
        && fixed_fixture_name(root).is_none()
    {
        return Err(format!(
            "fixture_root is /tmp/<name>, a directory directly in /tmp, not {root:?}"
        ));
    }
    if let Some(name) = header.env.keys().find(|name| {
        RUNNER_OWNED_ENVIRONMENT.contains(&name.as_str())
            || XDG_DIRECTORIES.iter().any(|(variable, _)| variable == name)
            || QA_ENVIRONMENT.iter().any(|(variable, _)| variable == name)
    }) {
        return Err(format!(
            "env may not set {name}, which the runner owns for every run"
        ));
    }
    let mut steps = Vec::new();
    for (index, line) in lines {
        let step: Value =
            serde_json::from_str(line).map_err(|error| format!("line {}: {error}", index + 1))?;
        let Some(object) = step.as_object() else {
            return Err(format!("line {}: a step must be an object", index + 1));
        };
        let commands: Vec<&str> = protocol::COMMANDS
            .iter()
            .chain(RUNNER_COMMANDS.iter())
            .copied()
            .filter(|name| object.contains_key(*name))
            .collect();
        if commands.len() != 1 {
            return Err(format!(
                "line {}: a step needs exactly one command, found {commands:?}",
                index + 1
            ));
        }
        if let Some(modes) = object.get(STEP_MODES) {
            let modes: Vec<String> = serde_json::from_value(modes.clone())
                .map_err(|error| format!("line {}: modes: {error}", index + 1))?;
            for mode in &modes {
                view_name(mode).map_err(|error| format!("line {}: {error}", index + 1))?;
            }
        }
        if ["dbus_send", "launch", "foreign"]
            .iter()
            .any(|command| object.contains_key(*command))
            && !header.session_bus
        {
            return Err(format!(
                "line {}: dbus_send, launch, and foreign need the header's session_bus",
                index + 1
            ));
        }
        if ["portal_open", "portal_response"]
            .iter()
            .any(|command| object.contains_key(*command))
            && !header.portal
        {
            return Err(format!(
                "line {}: portal_open and portal_response need the header's portal",
                index + 1
            ));
        }
        if let Some(variants) = object.get(STEP_VARIANTS) {
            let variants: Vec<String> = serde_json::from_value(variants.clone())
                .map_err(|error| format!("line {}: variants: {error}", index + 1))?;
            for variant in &variants {
                if !header.variants.iter().any(|known| known.id == *variant) {
                    return Err(format!("line {}: no variant {variant:?}", index + 1));
                }
            }
        }
        steps.push((index + 1, step));
    }
    match (&header.known_bug, header.known_bug_step) {
        (None, None) => {}
        (Some(_), Some(line)) => {
            let step = steps
                .iter()
                .find(|(number, _)| *number == line)
                .map(|(_, step)| step)
                .ok_or_else(|| format!("known_bug_step {line} is not a step line"))?;
            if let Some(kind) = KEEPING_STEPS
                .into_iter()
                .find(|kind| step.get(*kind).is_some())
            {
                return Err(format!(
                    "known_bug_step {line} is a {kind} step, which only keeps or switches state \
                     and cannot fail as the scenario's expected failure"
                ));
            }
            if [STEP_WHEN, STEP_IF, STEP_UNLESS]
                .iter()
                .any(|key| step.get(*key).is_some())
            {
                return Err(format!(
                    "known_bug_step {line} has a when, if, or unless, so it may not run at all"
                ));
            }
        }
        (Some(_), None) => {
            return Err("known_bug needs known_bug_step: the line expected to fail".to_owned());
        }
        (None, Some(_)) => return Err("known_bug_step needs known_bug".to_owned()),
    }
    Ok(Scenario { header, steps })
}

fn check_cost(header: &Header) -> Result<(), String> {
    let mut labels: Vec<String> = runs(header).into_iter().map(|run| run.label).collect();
    labels.sort();
    if let Some(pair) = labels.windows(2).find(|pair| pair[0] == pair[1]) {
        return Err(format!("two runs share the label {:?}", pair[0]));
    }
    let Some(cost) = &header.cost else {
        return Ok(());
    };
    let keys: Vec<&String> = cost.keys().collect();
    if keys != labels.iter().collect::<Vec<_>>() {
        return Err(format!(
            "cost names exactly the runs {labels:?}, not {keys:?}; refresh it with scripts/qa_cost.py"
        ));
    }
    if let Some((label, value)) = cost
        .iter()
        .find(|(_, value)| !(value.is_finite() && **value > 0.0))
    {
        return Err(format!(
            "cost {value} for {label:?} is not a positive number"
        ));
    }
    Ok(())
}

fn is_name(name: &str, separator: char) -> bool {
    !name.is_empty()
        && name.split(separator).all(|word| {
            !word.is_empty()
                && word
                    .chars()
                    .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
        })
}

pub(crate) fn check_folder(folder: &str) -> Result<(), String> {
    if !is_name(folder, '_') || folder.starts_with("test_") {
        return Err(format!(
            "a scenario lives in a feature-area folder named in snake_case \
             without a test_ prefix, not {folder:?}"
        ));
    }
    Ok(())
}

pub(crate) fn check_quality(quality: &Value) -> Result<Quality, String> {
    if !quality.is_object() {
        return Err(format!("quality is an object, not {quality}"));
    }
    let quality: Quality =
        serde_json::from_value(quality.clone()).map_err(|error| format!("quality: {error}"))?;
    for (name, value) in [
        ("impact", quality.impact),
        ("coverage", quality.coverage),
        ("criticality", quality.criticality),
        ("diagnostic", quality.diagnostic),
    ] {
        if !(1..=5).contains(&value) {
            return Err(format!("quality {name} is 1 to 5, not {value}"));
        }
    }
    let expected = quality_score(&quality);
    if quality.score != expected {
        return Err(format!(
            "quality score is {expected} for these ratings, not {}",
            quality.score
        ));
    }
    // Every break Python's `str.splitlines` splits on, as the scripts read headers.
    let breaks = [
        '\n', '\r', '\u{b}', '\u{c}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2028}',
        '\u{2029}',
    ];
    if quality.note.trim().is_empty() || quality.note.contains(breaks) {
        return Err("quality note is one non-empty line".to_owned());
    }
    Ok(quality)
}

/// The `qa-quality` rubric's score, 0 to 100: `100 * (coverage - 1) / 4 * P / 32`
/// rounded half up, where `P = 3(impact - 1) + 3(criticality - 1) + 2(diagnostic - 1)`.
fn quality_score(quality: &Quality) -> u8 {
    let rating = |value: u8| u32::from(value) - 1;
    let points = 3 * rating(quality.impact)
        + 3 * rating(quality.criticality)
        + 2 * rating(quality.diagnostic);
    u8::try_from((50 * rating(quality.coverage) * points + 32) / 64)
        .expect("ratings from 1 to 5 score at most 100")
}

pub(crate) fn view_name(mode: &str) -> Result<&'static str, String> {
    MODES
        .iter()
        .find(|(name, _)| *name == mode)
        .map(|(_, view)| *view)
        .ok_or_else(|| format!("unknown mode {mode:?}"))
}
