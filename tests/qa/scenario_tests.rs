// SPDX-License-Identifier: MIT

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use crate::{
    REPOSITORY,
    fixture::manifest,
    harness,
    scenario::{
        Quality, Scenario, check_folder, check_quality, load_scenario, planned_tests,
        scenario_files, scenario_name,
    },
    steps::run_steps,
    variables::{placeholders, run_name, runs, variables},
};

pub(crate) fn every_scenario_file_is_planned_and_well_formed() {
    let files = scenario_files();
    let mut names: Vec<String> = files.iter().map(|path| scenario_name(path)).collect();
    names.sort();
    if let Some(pair) = names.windows(2).find(|pair| pair[0] == pair[1]) {
        panic!("two scenario files are named {}", pair[0]);
    }
    let scenarios: Vec<(String, &PathBuf, Scenario)> = files
        .iter()
        .map(|path| {
            let scenario =
                load_scenario(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            (scenario_name(path), path, scenario)
        })
        .collect();
    let mut expected: Vec<String> = scenarios
        .iter()
        .flat_map(|(name, _, scenario)| {
            runs(&scenario.header)
                .into_iter()
                .map(move |run| format!("{name}/{}", run.label))
        })
        .collect();
    expected.sort();
    let mut planned: Vec<String> = planned_tests()
        .into_iter()
        .filter(|test| test.group.is_some())
        .map(|test| test.name)
        .collect();
    planned.sort();
    assert_eq!(
        planned, expected,
        "the harness should plan one test per scenario run"
    );
    let manifest = manifest();
    for (name, path, scenario) in &scenarios {
        let folder = path
            .parent()
            .and_then(Path::file_name)
            .and_then(|folder| folder.to_str())
            .unwrap_or_default()
            .to_owned();
        let header = &scenario.header;
        if let Err(error) = check_folder(&folder) {
            panic!("{name}: {error}");
        }
        if let Some(mutation) = &header.mutation {
            assert!(
                Path::new(REPOSITORY)
                    .join(format!("tests/qa/mutations/{mutation}.patch"))
                    .is_file(),
                "{name}: the mutation {mutation} should exist"
            );
        }
        for run in runs(header) {
            let layout = run.variant.layout.as_ref().unwrap_or(&header.layout);
            assert!(
                manifest.layouts.contains_key(layout),
                "{name}/{}: no layout {layout:?}",
                run.label
            );
            let steps = run_steps(scenario, &run, &variables(&run, &placeholders()))
                .unwrap_or_else(|error| panic!("{name}/{}: {error}", run.label));
            assert!(!steps.is_empty(), "{name}/{}: no steps run", run.label);
        }
    }
}

pub(crate) fn a_known_bug_must_name_a_step_line_that_can_fail() {
    let directory = tempfile::tempdir().expect("a scratch directory");
    let path = directory.path().join("example.jsonl");
    let load = |marker: &str| {
        fs::write(
            &path,
            format!(
                "{{\"scenario\":\"example\",\"modes\":[\"columns\"],\"mutation\":null{marker}}}\n\
                 {{\"layout\":\"ru\"}}\n\
                 {{\"wait\":{{\"focused\":\"a\"}}}}\n\
                 {{\"key\":\"Escape\",\"when\":{{\"dialog\":true}}}}\n\
                 {{\"flag\":{{\"dialog\":null}},\"as\":\"closed\"}}\n\
                 {{\"note\":\"focused\",\"as\":\"name\"}}\n\
                 {{\"note\":\"entries\",\"as\":\"names\"}}\n\
                 {{\"slice\":\"names\",\"pick\":0,\"as\":\"first\"}}\n"
            ),
        )
        .expect("the scenario should be written");
        load_scenario(&path).map(|_| ())
    };
    assert!(load(r#","known_bug":"issue-1","known_bug_step":3"#).is_ok());
    for marker in [
        r#","known_bug":"issue-1""#,
        r#","known_bug":null,"known_bug_step":3"#,
        r#","known_bug":"issue-1","known_bug_step":2"#,
        r#","known_bug":"issue-1","known_bug_step":4"#,
        r#","known_bug":"issue-1","known_bug_step":5"#,
        r#","known_bug":"issue-1","known_bug_step":6"#,
        r#","known_bug":"issue-1","known_bug_step":8"#,
        r#","known_bug":"issue-1","known_bug_step":19"#,
    ] {
        assert!(load(marker).is_err(), "{marker} should be rejected");
    }
}

pub(crate) fn a_skipped_scenario_names_a_reason_and_no_mutation_or_known_bug() {
    let directory = tempfile::tempdir().expect("a scratch directory");
    let path = directory.path().join("example.jsonl");
    let load = |extra: &str| {
        fs::write(
            &path,
            format!(
                "{{\"scenario\":\"example\",\"modes\":[\"columns\"],{extra}}}\n\
                 {{\"key\":\"Escape\"}}\n"
            ),
        )
        .expect("the scenario should be written");
        load_scenario(&path).map(|scenario| scenario.header.skip)
    };
    assert_eq!(
        load(r#""mutation":null,"known_bug":null,"skip":"issue 9""#),
        Ok(Some("issue 9".to_owned()))
    );
    for bad in [
        r#""mutation":null,"known_bug":null,"skip":" ""#,
        r#""mutation":"clipboard","known_bug":null,"skip":"x""#,
        r#""mutation":null,"known_bug":"y","known_bug_step":2,"skip":"x""#,
    ] {
        assert!(load(bad).is_err(), "{bad} should be rejected");
    }
}

pub(crate) fn a_cost_gives_each_run_label_a_positive_number() {
    let directory = tempfile::tempdir().expect("a scratch directory");
    let path = directory.path().join("example.jsonl");
    let load = |cost: &str| {
        fs::write(
            &path,
            format!(
                "{{\"scenario\":\"example\",\"modes\":[\"columns\",\"list\"],\
                 \"mutation\":null,\"known_bug\":null{cost}}}\n{{\"key\":\"Escape\"}}\n"
            ),
        )
        .expect("the scenario should be written");
        load_scenario(&path).map(|scenario| scenario.header.cost)
    };
    assert_eq!(load(""), Ok(None));
    assert_eq!(
        load(r#","cost":{"list":0.1,"columns":2.5}"#),
        Ok(Some(BTreeMap::from([
            ("columns".to_owned(), 2.5),
            ("list".to_owned(), 0.1)
        ])))
    );
    for bad in [
        r#","cost":{"columns":1.0}"#,
        r#","cost":{"columns":1.0,"list":1.0,"icons":1.0}"#,
        r#","cost":{"columns":1.0,"list":0}"#,
        r#","cost":{"columns":1.0,"list":-1.5}"#,
        r#","cost":{"columns":1.0,"list":"1.0"}"#,
        r#","cost":[1.0,1.0]"#,
    ] {
        assert!(load(bad).is_err(), "{bad} should be rejected");
    }
}

pub(crate) fn a_scenario_lives_in_a_feature_folder_and_names_its_runs() {
    let directory = tempfile::tempdir().expect("a scratch directory");
    let path = directory.path().join("example.jsonl");
    let load = |header: &str| {
        fs::write(&path, format!("{header}\n{{\"key\":\"Escape\"}}\n"))
            .expect("the scenario should be written");
        load_scenario(&path)
    };
    let names = |scenario: &Scenario| -> Vec<String> {
        runs(&scenario.header)
            .iter()
            .map(|run| run_name("example", run))
            .collect()
    };

    let scenario =
        load(r#"{"scenario":"example","modes":["columns","list"],"variants":[{"id":"us"}]}"#)
            .expect("the scenario should load");
    assert_eq!(names(&scenario), ["example/columns-us", "example/list-us"]);
    assert_eq!(check_folder("hidden_files"), Ok(()));
    assert_eq!(check_folder("tabs"), Ok(()));
    for folder in [
        "test_tabs",
        "Hidden_files",
        "hidden-files",
        "hidden__files",
        "_tabs",
        "",
    ] {
        assert!(
            check_folder(folder).is_err(),
            "{folder:?} should be rejected"
        );
    }

    let renamed = directory.path().join("Example_one.jsonl");
    fs::write(
        &renamed,
        "{\"scenario\":\"Example_one\",\"modes\":[\"columns\"]}\n{\"key\":\"Escape\"}\n",
    )
    .expect("the scenario should be written");
    assert!(
        load_scenario(&renamed).is_err(),
        "a scenario name is lowercase words joined by hyphens"
    );
}

pub(crate) fn a_quality_rates_four_axes_and_its_score_follows_the_rubric() {
    let directory = tempfile::tempdir().expect("a scratch directory");
    let path = directory.path().join("example.jsonl");
    let load = |quality: &str| {
        fs::write(
            &path,
            format!(
                "{{\"scenario\":\"example\",\"modes\":[\"columns\"],\"quality\":{quality}}}\n\
                 {{\"key\":\"Escape\"}}\n"
            ),
        )
        .expect("the scenario should be written");
        load_scenario(&path).map(|scenario| {
            scenario
                .header
                .quality
                .map(|quality| check_quality(&quality))
        })
    };
    let rated = |[impact, coverage, criticality, diagnostic]: [u8; 4], score: u8| {
        format!(
            r#"{{"impact":{impact},"coverage":{coverage},"criticality":{criticality},"diagnostic":{diagnostic},"score":{score},"note":"why it matters"}}"#
        )
    };
    for (ratings, score) in [
        ([1, 1, 1, 1], 0),
        ([5, 1, 5, 5], 0),
        ([1, 5, 1, 1], 0),
        ([5, 5, 5, 5], 100),
        ([3, 3, 3, 3], 25),
    ] {
        let [impact, coverage, criticality, diagnostic] = ratings;
        assert_eq!(
            load(&rated(ratings, score)),
            Ok(Some(Ok(Quality {
                impact,
                coverage,
                criticality,
                diagnostic,
                score,
                note: "why it matters".to_owned(),
            }))),
            "{ratings:?} should score {score}"
        );
        let error = load(&rated(ratings, score + 1)).expect_err("a wrong score is rejected");
        assert!(
            error.contains(&format!("quality score is {score} ")),
            "the error should name the expected score: {error}"
        );
    }
    for bad in [
        r#"{"impact":0,"coverage":3,"criticality":3,"diagnostic":3,"score":25,"note":"x"}"#,
        r#"{"impact":3,"coverage":6,"criticality":3,"diagnostic":3,"score":25,"note":"x"}"#,
        r#"{"impact":3,"coverage":3,"criticality":9,"diagnostic":3,"score":25,"note":"x"}"#,
        r#"{"impact":3,"coverage":3,"criticality":3,"diagnostic":0,"score":25,"note":"x"}"#,
        r#"{"impact":3,"coverage":3,"criticality":3,"diagnostic":3,"score":-1,"note":"x"}"#,
        r#"{"impact":3,"coverage":3,"criticality":3,"diagnostic":3,"score":25.0,"note":"x"}"#,
        r#"{"impact":2.5,"coverage":3,"criticality":3,"diagnostic":3,"score":25,"note":"x"}"#,
        r#"{"impact":"3","coverage":3,"criticality":3,"diagnostic":3,"score":25,"note":"x"}"#,
        r#"{"impact":3,"coverage":3,"criticality":3,"diagnostic":3,"score":25,"note":" "}"#,
        r#"{"impact":3,"coverage":3,"criticality":3,"diagnostic":3,"score":25,"note":"two\nlines"}"#,
        r#"{"impact":3,"coverage":3,"criticality":3,"diagnostic":3,"score":25,"note":"cr\rhere"}"#,
        r#"{"impact":3,"coverage":3,"criticality":3,"diagnostic":3,"score":25,"note":"a\u000bb"}"#,
        r#"{"impact":3,"coverage":3,"criticality":3,"diagnostic":3,"score":25,"note":"a\u000cb"}"#,
        r#"{"impact":3,"coverage":3,"criticality":3,"diagnostic":3,"score":25,"note":"a\u001cb"}"#,
        r#"{"impact":3,"coverage":3,"criticality":3,"diagnostic":3,"score":25,"note":"a\u001db"}"#,
        r#"{"impact":3,"coverage":3,"criticality":3,"diagnostic":3,"score":25,"note":"a\u001eb"}"#,
        r#"{"impact":3,"coverage":3,"criticality":3,"diagnostic":3,"score":25,"note":"a\u0085b"}"#,
        r#"{"impact":3,"coverage":3,"criticality":3,"diagnostic":3,"score":25,"note":"a\u2028b"}"#,
        r#"{"impact":3,"coverage":3,"criticality":3,"diagnostic":3,"score":25,"note":"a\u2029b"}"#,
        r#"{"impact":3,"coverage":3,"criticality":3,"diagnostic":3,"score":25,"note":3}"#,
        r#"{"impact":3,"coverage":3,"criticality":3,"diagnostic":3,"score":25}"#,
        r#"{"impact":3,"coverage":3,"criticality":3,"score":25,"note":"x"}"#,
        r#"{"impact":3,"coverage":3,"criticality":3,"diagnostic":3,"score":25,"note":"x","weight":2}"#,
        r#"[3,3,3,3,25,"x"]"#,
    ] {
        assert!(
            !matches!(load(bad), Ok(Some(Ok(_)))),
            "{bad} should be rejected"
        );
    }
}

pub(crate) fn the_harness_starts_costly_runs_first_and_names_select_whole_scenarios() {
    let test = |name: &str, cost: Option<f64>, exclusive: bool| harness::Test {
        name: name.to_owned(),
        group: name.split_once('/').map(|(group, _)| group.to_owned()),
        ignored: None,
        skip: None,
        cost,
        exclusive,
        plan: None,
        run: Box::new(|| harness::Outcome::Passed(None)),
    };
    let mut tests = vec![
        test("ab/list", Some(3.0), false),
        test("a/columns", Some(1.0), false),
        test("unit_test", None, false),
        test("a/list", Some(3.0), true),
        test("b/columns", None, true),
    ];
    tests.sort_by(harness::starts_before);
    let order: Vec<&str> = tests.iter().map(|test| test.name.as_str()).collect();
    assert_eq!(
        order,
        ["b/columns", "unit_test", "a/list", "ab/list", "a/columns"]
    );
    assert_eq!(harness::next_to_start(&tests, false), Some(0));
    assert_eq!(harness::next_to_start(&tests, true), Some(1));
    assert_eq!(harness::next_to_start(&tests[2..3], true), None);

    let selected = |arguments: &[&str]| -> Vec<&str> {
        let arguments = harness::Arguments::parse(arguments.iter().map(|&text| text.to_owned()))
            .expect("the arguments should parse");
        tests
            .iter()
            .filter(|test| arguments.selects(test))
            .map(|test| test.name.as_str())
            .collect()
    };
    assert_eq!(selected(&["--exact", "a"]), ["a/list", "a/columns"]);
    assert_eq!(
        selected(&["--exact", "a/list", "b"]),
        ["b/columns", "a/list"]
    );
    assert_eq!(
        selected(&["a"]),
        ["a/list", "ab/list", "a/columns"],
        "without --exact a filter is a substring, as libtest's"
    );
    assert_eq!(
        selected(&["--exact", "--skip", "a"]),
        ["b/columns", "unit_test", "ab/list"]
    );
    assert_eq!(
        selected(&["--skip=a/"]),
        ["b/columns", "unit_test", "ab/list"]
    );
    assert!(selected(&["--exact", "unit"]).is_empty());
    assert_eq!(
        selected(&["--skip-exact", "a"]),
        ["b/columns", "unit_test", "ab/list"],
        "--skip-exact leaves out a whole scenario, never one it prefixes, without --exact"
    );
    assert_eq!(
        selected(&["b", "--skip-exact", "a/list"]),
        ["b/columns", "ab/list"]
    );

    assert_eq!(
        harness::Arguments::parse(
            [
                "--test-threads=48",
                "-Zunstable-options",
                "--format",
                "terse",
                "--include-ignored",
                "--show-output",
                "--report-time",
                "--shuffle",
                "--shuffle-seed",
                "7",
                "name",
            ]
            .map(str::to_owned)
        ),
        Ok(harness::Arguments {
            filters: vec!["name".to_owned()],
            threads: Some(48),
            terse: true,
            include_ignored: true,
            ..harness::Arguments::default()
        })
    );
    for bad in [
        &["--bogus"][..],
        &["--test-threads", "0"],
        &["--format", "json"],
        &["--skip"],
        &["--shuffle-seed"],
    ] {
        assert!(
            harness::Arguments::parse(bad.iter().map(|&text| text.to_owned())).is_err(),
            "{bad:?} should be rejected"
        );
    }
}

pub(crate) fn no_test_attribute_hides_a_runner_test() {
    let attributes = [concat!("#[", "test"), concat!("#[", "cfg(test)")];
    let mut sources = vec![Path::new(REPOSITORY).join("tests/qa.rs")];
    for entry in
        fs::read_dir(Path::new(REPOSITORY).join("tests/qa")).expect("tests/qa should be listable")
    {
        let path = entry.expect("the entry should be readable").path();
        if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(path);
        }
    }
    for path in sources {
        let text = fs::read_to_string(&path).expect("the runner source should be readable");
        for (index, line) in text.lines().enumerate() {
            assert!(
                !attributes
                    .iter()
                    .any(|attribute| line.trim_start().starts_with(attribute)),
                "{}:{}: this target has its own harness, so #[test] and #[cfg(test)] code \
                 never runs; list the test in unit_tests! instead",
                path.display(),
                index + 1
            );
        }
    }
}
