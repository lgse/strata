// SPDX-License-Identifier: MIT

use super::*;
use crate::model::action::examples::ACTION_EXAMPLES;
use std::process::Command;

fn example_action(fixture: &Fixture, index: usize) -> Rc<ActionHandle> {
    let example = &ACTION_EXAMPLES[index];
    let action = fixture.action(
        "example",
        &script(ActionRuntime::Python, "main.py", &example.script()),
        example.mode,
    );
    assert!(action.is_available(), "bundled recipes require Python 3");
    action
}

#[test]
fn documented_starter_reports_inputs_without_modifying_them() {
    let fixture = Fixture::new();
    let input = fixture.path().join("a file.txt");
    fs::write(&input, b"original").expect("input");
    let outcome = run(
        &fixture.runner,
        request(
            example_action(&fixture, 0),
            std::slice::from_ref(&input),
            fixture.path(),
        ),
    );
    let (code, _, log) = outcome.ended();
    assert_eq!(code, Some(0), "{log}");
    assert!(log.contains("a file.txt"), "{log}");
    assert_eq!(fs::read(input).expect("input remains"), b"original");
    assert!(outcome.created().is_empty());
    assert!(
        outcome
            .progress()
            .iter()
            .any(|event| event.completed == 1 && event.total == Some(1))
    );
}

#[test]
fn checksum_example_handles_native_names_and_refuses_existing_files_and_links() {
    use std::os::unix::{ffi::OsStringExt, fs::symlink};
    let fixture = Fixture::new();
    let input = fixture.path().join(OsString::from_vec(
        b"-name\\with\nnewline\r\xff.txt".to_vec(),
    ));
    fs::write(&input, b"original").expect("input");
    let action = example_action(&fixture, 5);
    let invoke = || request(action.clone(), std::slice::from_ref(&input), fixture.path());
    let outcome = run(&fixture.runner, invoke());
    let (code, _, log) = outcome.ended();
    assert_eq!(code, Some(0), "{log}");
    let mut checksum_name = input.as_os_str().to_os_string();
    checksum_name.push(".sha256");
    let checksum = PathBuf::from(checksum_name);
    let contents = fs::read(&checksum).expect("checksum");
    let verify = Command::new("sha256sum")
        .arg("--check")
        .arg(&checksum)
        .current_dir(fixture.path())
        .output()
        .expect("checksum verification");
    assert!(
        verify.status.success(),
        "{}",
        String::from_utf8_lossy(&verify.stderr)
    );
    assert_ne!(run(&fixture.runner, invoke()).ended().0, Some(0));
    assert_eq!(fs::read(&checksum).expect("existing checksum"), contents);
    fs::remove_file(&checksum).expect("remove checksum");
    symlink(&input, &checksum).expect("checksum link");
    assert_ne!(run(&fixture.runner, invoke()).ended().0, Some(0));
    assert_eq!(fs::read(&input).expect("original"), b"original");
    fs::remove_file(&checksum).expect("remove link");
    let missing = fixture.path().join("must-not-create");
    symlink(&missing, &checksum).expect("dangling link");
    assert_ne!(run(&fixture.runner, invoke()).ended().0, Some(0));
    assert!(!missing.exists());
}

#[test]
fn image_examples_make_distinct_copies_without_interpreting_input_names() {
    let fixture = Fixture::new();
    let input = fixture.path().join("-image [0]; $(touch injected).ppm");
    let pixels = b"P6\n2 2\n255\n\xff\0\0\xff\0\0\xff\0\0\xff\0\0";
    fs::write(&input, pixels).expect("image");
    for (index, magic) in [
        (1, b"RIFF".as_slice()),
        (2, b"\x89PNG\r\n\x1a\n".as_slice()),
    ] {
        let action = example_action(&fixture, index);
        let mut previous = None;
        for _ in 0..2 {
            let outcome = run(
                &fixture.runner,
                request(action.clone(), std::slice::from_ref(&input), fixture.path()),
            );
            let (code, _, log) = outcome.ended();
            assert_eq!(code, Some(0), "{}: {log}", ACTION_EXAMPLES[index].name);
            let outputs = outcome.created();
            assert_eq!(outputs.len(), 1, "{log}");
            let output = &outputs[0];
            assert!(
                fs::read(output)
                    .expect("converted image")
                    .starts_with(magic)
            );
            assert_ne!(previous.as_ref(), Some(output));
            previous = Some(output.clone());
            assert_eq!(fs::read(&input).expect("original"), pixels);
        }
    }
    assert!(!fixture.path().join("injected").exists());
}

#[test]
fn ffmpeg_examples_create_playable_outputs_and_preserve_originals() {
    let fixture = Fixture::new();
    let input = fixture.path().join("-clip with [brackets].mkv");
    let generated = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-n",
            "-f",
            "lavfi",
            "-i",
            "color=size=16x16:rate=5",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=8000",
            "-t",
            "0.2",
            "-c:v",
            "ffv1",
            "-c:a",
            "pcm_s16le",
        ])
        .arg(&input)
        .output()
        .expect("FFmpeg fixture");
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let original = fs::read(&input).expect("original video");
    for (index, codec) in [(3, "h264"), (4, "mp3")] {
        let outcome = run(
            &fixture.runner,
            request(
                example_action(&fixture, index),
                std::slice::from_ref(&input),
                fixture.path(),
            ),
        );
        let (code, _, log) = outcome.ended();
        assert_eq!(code, Some(0), "{}: {log}", ACTION_EXAMPLES[index].name);
        let outputs = outcome.created();
        assert_eq!(outputs.len(), 1, "{log}");
        let probe = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_name",
                "-of",
                "csv=p=0",
            ])
            .arg(&outputs[0])
            .output()
            .expect("probe generated media");
        assert!(
            probe.status.success(),
            "{}",
            String::from_utf8_lossy(&probe.stderr)
        );
        assert!(
            String::from_utf8_lossy(&probe.stdout)
                .lines()
                .any(|line| line == codec)
        );
        assert_eq!(fs::read(&input).expect("original video"), original);
    }
}

#[test]
fn converter_examples_report_missing_tools_without_creating_outputs() {
    let fixture = Fixture::new();
    let input = fixture.path().join("input.bin");
    fs::write(&input, b"original").expect("input");
    for example in &ACTION_EXAMPLES[1..5] {
        let source = format!(
            "#!/usr/bin/env python3\nimport strata_actions\nstrata_actions.find_tool = lambda name: None\n{}",
            example.script()
        );
        let action = fixture.action(
            "missing-tool",
            &script(ActionRuntime::Python, "main.py", &source),
            example.mode,
        );
        let outcome = run(
            &fixture.runner,
            request(action, std::slice::from_ref(&input), fixture.path()),
        );
        let (code, _, log) = outcome.ended();
        assert_ne!(code, Some(0));
        assert!(log.contains("was not found on your PATH"), "{log}");
        assert!(outcome.created().is_empty());
        assert_eq!(fs::read(&input).expect("original"), b"original");
    }
    assert!(
        !fs::read_dir(fixture.path())
            .expect("fixture")
            .any(|entry| entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .starts_with("strata-"))
    );
}
