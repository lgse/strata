// SPDX-License-Identifier: MIT

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use crate::services::MediaPreviewSize;

use super::{MediaPreviewBackend, ParseOperation, PdfRenderSize, sandbox_command, valid_output};

mod command;
mod gpu;
mod lifecycle;
mod output;
mod secrets;
mod validation;

const MEDIA_PREVIEW: ParseOperation = ParseOperation::PreviewMedia(MediaPreviewSize {
    width: 640,
    height: 800,
});
const PDF_PREVIEW: ParseOperation = ParseOperation::PreviewPdf(PdfRenderSize {
    width: 640,
    height: 800,
});

/// The arguments of one `sandbox_command` call, defaulting to a software
/// helper at `/tmp/strata` writing to `/tmp/private-output`.
struct Sandbox<'a> {
    executable: &'a str,
    input: &'a str,
    output: &'a str,
    operation: ParseOperation,
    value: i32,
    backend: MediaPreviewBackend,
    devices: &'a [PathBuf],
}

impl<'a> Sandbox<'a> {
    fn new(input: &'a str, operation: ParseOperation) -> Self {
        Self {
            executable: "/tmp/strata",
            input,
            output: "/tmp/private-output",
            operation,
            value: 0,
            backend: MediaPreviewBackend::Software,
            devices: &[],
        }
    }

    fn installed_at(self, executable: &'a str, output: &'a str) -> Self {
        Self {
            executable,
            output,
            ..self
        }
    }

    fn value(self, value: i32) -> Self {
        Self { value, ..self }
    }

    fn backend(self, backend: MediaPreviewBackend) -> Self {
        Self { backend, ..self }
    }

    fn devices(self, devices: &'a [PathBuf]) -> Self {
        Self { devices, ..self }
    }

    fn command(self) -> Command {
        sandbox_command(
            Path::new("/usr/bin/bwrap"),
            Path::new(self.executable),
            Path::new(self.input),
            Path::new(self.output),
            self.operation,
            self.value,
            self.backend,
            self.devices,
        )
    }

    fn arguments(self) -> Vec<String> {
        arguments_of(&self.command())
    }

    fn joined(self) -> String {
        self.arguments().join(" ")
    }
}

fn arguments_of(command: &Command) -> Vec<String> {
    command
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect()
}

#[track_caller]
fn assert_contains_all<S: AsRef<str>>(joined: &str, needles: &[S]) {
    let missing: Vec<&str> = needles
        .iter()
        .map(AsRef::as_ref)
        .filter(|needle| !joined.contains(needle))
        .collect();
    assert!(missing.is_empty(), "missing {missing:?} in {joined}");
}

#[track_caller]
fn assert_contains_none(joined: &str, needles: &[&str]) {
    let present: Vec<&str> = needles
        .iter()
        .copied()
        .filter(|needle| joined.contains(needle))
        .collect();
    assert!(present.is_empty(), "unexpected {present:?} in {joined}");
}

#[track_caller]
fn assert_no_argument(arguments: &[String], forbidden: &[&str]) {
    let found: Vec<&String> = arguments
        .iter()
        .filter(|argument| forbidden.contains(&argument.as_str()))
        .collect();
    assert!(found.is_empty(), "unexpected arguments {found:?}");
}

#[track_caller]
fn assert_validity(operation: &ParseOperation, cases: &[(&[u8], bool)]) {
    for (data, valid) in cases {
        assert_eq!(
            valid_output(operation.clone(), data),
            *valid,
            "{operation:?}: {}",
            String::from_utf8_lossy(data)
        );
    }
}
