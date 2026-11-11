// SPDX-License-Identifier: MIT

use std::io::{self, Write};

use super::PipeSafeWriter;

struct FailingWriter {
    error_kind: io::ErrorKind,
}

impl Write for FailingWriter {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(self.error_kind, "write failed"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::new(self.error_kind, "flush failed"))
    }
}

#[test]
fn pipe_safe_writer_swallows_broken_pipe() {
    let mut writer = PipeSafeWriter(FailingWriter {
        error_kind: io::ErrorKind::BrokenPipe,
    });
    let data = b"hello logging";
    assert_eq!(
        writer.write(data).expect("broken pipe is swallowed"),
        data.len()
    );
    assert!(writer.flush().is_ok());
}

#[test]
fn pipe_safe_writer_forwards_other_errors() {
    let mut writer = PipeSafeWriter(FailingWriter {
        error_kind: io::ErrorKind::PermissionDenied,
    });
    let data = b"hello logging";
    assert_eq!(
        writer
            .write(data)
            .expect_err("permission denied should return error")
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        writer
            .flush()
            .expect_err("flush permission denied should return error")
            .kind(),
        io::ErrorKind::PermissionDenied
    );
}

#[test]
fn pipe_safe_writer_forwards_successful_writes() {
    let mut sink = Vec::new();
    let mut writer = PipeSafeWriter(&mut sink);
    let data = b"success payload";
    assert_eq!(
        writer.write(data).expect("successful write should succeed"),
        data.len()
    );
    assert!(writer.flush().is_ok());
    assert_eq!(sink, data);
}
