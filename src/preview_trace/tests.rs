// SPDX-License-Identifier: MIT

use std::process::Command;

use gtk::{glib, prelude::*};

#[test]
fn trace_is_opt_in_and_does_not_retain_objects() {
    if std::env::var_os("STRATA_TRACE_TEST_CHILD").is_some() {
        let object = glib::Object::new::<glib::Object>();
        let weak = object.downgrade();
        super::watch_object(&object, "test", 42);
        super::event!("escaped", "value" => "line\n\"quote\"");
        drop(object);
        assert!(weak.upgrade().is_none());
        return;
    }
    for enabled in [false, true] {
        let output = Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "preview_trace::tests::trace_is_opt_in_and_does_not_retain_objects",
                "--nocapture",
            ])
            .env("STRATA_TRACE_TEST_CHILD", "1")
            .env("STRATA_PREVIEW_TRACE", if enabled { "1" } else { "0" })
            .output()
            .expect("trace probe starts");
        assert!(output.status.success());
        let stderr = String::from_utf8(output.stderr).expect("UTF-8 trace output");
        let records: Vec<serde_json::Value> = stderr
            .lines()
            .filter_map(|line| line.strip_prefix("STRATA_PREVIEW_TRACE "))
            .map(|line| serde_json::from_str(line).expect("JSON trace record"))
            .collect();
        if enabled {
            assert_eq!(records.len(), 3);
            assert_eq!(records[0]["event"], "object_created");
            assert_eq!(records[1]["fields"]["value"], "line\n\"quote\"");
            assert_eq!(records[2]["event"], "object_finalized");
            assert_eq!(records[2]["fields"]["object"], 42);
            assert!(records[0]["unix_ms"].as_u64().expect("numeric timestamp") > 0);
        } else {
            assert!(records.is_empty());
        }
    }
}

#[test]
fn media_observers_do_not_retain_the_media_file() {
    const TEST: &str = "preview_trace::tests::media_observers_do_not_retain_the_media_file";
    if std::env::var("STRATA_PREVIEW_TRACE").as_deref() != Ok("1") {
        let output = Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", TEST, "--nocapture"])
            .env("STRATA_PREVIEW_TRACE", "1")
            .output()
            .expect("media observer probe starts");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stderr).contains("object_finalized"));
        return;
    }
    crate::test_support::gtk_test(TEST, || {
        let media = gtk::MediaFile::new();
        let weak = media.downgrade();
        super::watch_media(&media, 123);
        drop(media);
        assert!(weak.upgrade().is_none());
    });
}

#[test]
fn helper_trace_relay_filters_diagnostics_and_caps_input() {
    let valid = b"STRATA_PREVIEW_TRACE {\"event\":\"backend_attempt\"}\n";
    let mut input = b"untrusted renderer error\nSTRATA_PREVIEW_TRACE invalid-json\n".to_vec();
    input.extend_from_slice(valid);
    let mut output = Vec::new();
    super::relay_helper_trace(input.as_slice(), &mut output).expect("relay trace");
    assert_eq!(output, valid);

    let mut oversized = std::io::Cursor::new(vec![b'x'; 128 * 1024]);
    output.clear();
    super::relay_helper_trace(&mut oversized, &mut output).expect("bounded relay");
    assert_eq!(oversized.position(), 64 * 1024);
    assert!(output.is_empty());
}

#[test]
fn file_tokens_correlate_within_a_process() {
    let path = std::path::Path::new("/private/video.mp4");
    assert_eq!(super::file_id(&path), super::file_id(&path.to_path_buf()));
    assert_ne!(
        super::file_id(&path),
        super::file_id(&std::path::Path::new("/private/other.mp4"))
    );
}
