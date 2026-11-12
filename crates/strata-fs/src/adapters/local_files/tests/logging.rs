// SPDX-License-Identifier: MIT

use super::*;

fn capture_directory_start_logs(locations: &[(RequestId, &Location)]) -> String {
    capture_logs(|| {
        for (request_id, location) in locations {
            log_directory_load_started(*request_id, location);
        }
    })
}

fn captured_event<'a>(output: &'a str, request_id: RequestId, message: &str) -> &'a str {
    let request_id = format!("request_id={}", request_id.0);
    output
        .lines()
        .find(|line| line.contains(message) && line.contains(&request_id))
        .unwrap_or_else(|| panic!("missing {message:?} event for {request_id}"))
}

struct LoggedLine<'a> {
    request_id: RequestId,
    message: &'a str,
    level: &'a str,
    shows: &'a [&'a str],
    hides: &'a [&'a str],
}

impl LoggedLine<'_> {
    fn assert_in(&self, output: &str) {
        let line = captured_event(output, self.request_id, self.message);
        assert_eq!(line.split_whitespace().next(), Some(self.level), "{line}");
        assert_contains_all(line, self.shows);
        assert_contains_none(line, self.hides);
    }
}

#[test]
fn directory_logging_respects_default_and_diagnostic_privacy() {
    let native_path = "/home/alice/sentinel-private-directory";
    let native = Location::local(native_path);
    let remote = Location::uri(
        "sftp://alice:password;key=secret@example.com/private?token=secret#private-fragment",
    );
    let output =
        capture_directory_start_logs(&[(RequestId(42), &native), (RequestId(43), &remote)]);
    let started = "directory load started";
    let located = "directory load location";

    for line in [
        LoggedLine {
            request_id: RequestId(42),
            message: started,
            level: "INFO",
            shows: &["backend=native"],
            hides: &[native_path],
        },
        LoggedLine {
            request_id: RequestId(42),
            message: located,
            level: "DEBUG",
            shows: &[native_path],
            hides: &[],
        },
        LoggedLine {
            request_id: RequestId(43),
            message: started,
            level: "INFO",
            shows: &["backend=sftp"],
            hides: &["example.com"],
        },
        LoggedLine {
            request_id: RequestId(43),
            message: located,
            level: "DEBUG",
            shows: &["sftp://example.com/private"],
            hides: &[
                "alice",
                "password",
                "key=secret",
                "token=secret",
                "private-fragment",
            ],
        },
    ] {
        line.assert_in(&output);
    }
}

#[test]
fn remote_backend_failures_hide_uri_user_info_in_views_and_diagnostic_logs() {
    let location = Location::uri("sftp://alice@host.example/private");
    let error = glib::Error::new(
        gio::IOErrorEnum::Failed,
        "Unable to read sftp://alice:secret@host.example/private?token=hidden#fragment",
    );
    let validation = uri_validation_result(&location, Err(error.clone()));
    let Err(LocationValidationError::Unavailable(validation_message)) = validation else {
        panic!("remote validation should report a sanitized failure");
    };
    let DirectoryEvent::Failed { message, .. } = remote_directory_failure(RequestId(42), &error)
    else {
        panic!("remote enumeration should report a sanitized failure");
    };
    let diagnostic = capture_logs(|| log_monitor_metadata_error(&location, &error));

    assert_eq!(
        (validation_message.as_str(), message.as_str()),
        (
            crate::services::UNEXPECTED_SYSTEM_ERROR,
            crate::services::UNEXPECTED_SYSTEM_ERROR
        )
    );
    assert!(diagnostic.contains("sftp://host.example/private"));
    for text in [&validation_message, &message, &diagnostic] {
        assert_contains_none(text, &["alice", "secret", "token", "hidden", "fragment"]);
    }
}
