// SPDX-License-Identifier: MIT

#![cfg(test)]

pub(crate) mod operations;

use std::{
    io::Write,
    sync::{Arc, LockResult, Mutex, MutexGuard, Once},
};

use tracing_subscriber::fmt::MakeWriter;

pub(crate) struct TestMutex(Mutex<()>);

impl TestMutex {
    pub(crate) const fn new() -> Self {
        Self(Mutex::new(()))
    }

    pub(crate) fn lock(&self) -> LockResult<MutexGuard<'_, ()>> {
        match self.0.lock() {
            Ok(guard) => Ok(guard),
            Err(error) => {
                self.0.clear_poison();
                Ok(error.into_inner())
            }
        }
    }
}

/// Serializes tests that drive `glib::MainContext::default()` directly, since it is a
/// process-wide singleton and concurrent access from the test harness's per-test threads panics
/// with a GLib thread-affinity error. A single shared lock, not one static per module: two
/// separate locks each covering only their own module's tests do not prevent a test in one
/// module from racing a test in another, since neither knows about the other's lock. Poisoning is
/// cleared because the mutex protects no state and should not turn one failure into a cascade.
pub(crate) static ASYNC_MAIN_CONTEXT_DEFAULT: TestMutex = TestMutex::new();

#[derive(Clone, Default)]
pub(crate) struct LogWriter(Arc<Mutex<Vec<u8>>>);

pub(crate) struct LogWriterGuard<'a>(MutexGuard<'a, Vec<u8>>);

impl Write for LogWriterGuard<'_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.0.write(buffer)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

impl<'a> MakeWriter<'a> for LogWriter {
    type Writer = LogWriterGuard<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        LogWriterGuard(self.0.lock().unwrap_or_else(|error| error.into_inner()))
    }
}

impl LogWriter {
    fn output(&self) -> String {
        let output = self.0.lock().unwrap_or_else(|error| error.into_inner());
        String::from_utf8_lossy(&output).into_owned()
    }
}

static LOG_CAPTURE: TestMutex = TestMutex::new();

static GLOBAL_SINK: Once = Once::new();

// A global sink keeps callsites interesting even when another test logs before a
// thread-local capture installs its subscriber.
fn install_global_sink() {
    GLOBAL_SINK.call_once(|| {
        let sink = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(std::io::sink)
            .finish();
        let _installed = tracing::subscriber::set_global_default(sink);
    });
}

pub(crate) fn capture_logs(action: impl FnOnce()) -> String {
    install_global_sink();
    let _guard = LOG_CAPTURE.lock();
    let writer = LogWriter::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(writer.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, action);
    writer.output()
}

pub(crate) fn captured_event<'a>(output: &'a str, message: &str) -> &'a str {
    output
        .lines()
        .find(|line| line.contains(message))
        .unwrap_or_else(|| panic!("missing {message:?} event in:\n{output}"))
}

/// GTK initialization is thread-affine; each UI test gets a process and disposable preferences.
pub(crate) fn gtk_test(name: &str, run: impl FnOnce()) {
    gtk_test_with_env(name, std::iter::empty::<(&str, &std::ffi::OsStr)>(), run);
}

pub(crate) fn gtk_test_with_env(
    name: &str,
    extra_env: impl IntoIterator<Item = (impl AsRef<std::ffi::OsStr>, impl AsRef<std::ffi::OsStr>)>,
    run: impl FnOnce(),
) {
    const CHILD: &str = "STRATA_ISOLATED_GTK_TEST";
    if std::env::var(CHILD).as_deref() == Ok(name) {
        if let Err(error) = gtk::init() {
            assert!(
                std::env::var_os("STRATA_REQUIRE_GTK_TESTS").is_none(),
                "GTK display required: {error}"
            );
            eprintln!("Skipping {name}: {error}");
            return;
        }
        crate::assets::prepare().expect("bundled assets");
        crate::assets::register_icon_theme();
        run();
        return;
    }
    let extra_env: Vec<(std::ffi::OsString, std::ffi::OsString)> = extra_env
        .into_iter()
        .map(|(key, value)| (key.as_ref().to_owned(), value.as_ref().to_owned()))
        .collect();
    // Child processes still share the display's clipboard and pointer grabs.
    static DISPLAY: TestMutex = TestMutex::new();
    let _display = DISPLAY.lock().expect("GTK display lock");
    let sandbox = tempfile::tempdir().expect("isolated preferences");
    let home = sandbox.path().join("home");
    std::fs::create_dir_all(&home).expect("isolated home");
    let mut command = std::process::Command::new(std::env::current_exe().expect("test executable"));
    command
        // The parent already selected this exact case, including explicit --ignored runs.
        .args(["--exact", name, "--nocapture", "--include-ignored"])
        .env(CHILD, name)
        .env("HOME", home)
        .env("XDG_STATE_HOME", sandbox.path().join("state"))
        .env("XDG_CONFIG_HOME", sandbox.path().join("config"))
        .env("XDG_CACHE_HOME", sandbox.path().join("cache"))
        .env("XDG_DATA_HOME", sandbox.path().join("data"));
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let status = command.status().expect("isolated GTK test starts");
    assert!(status.success(), "{name} failed");
}

pub(crate) fn serve_http_once(response: Vec<u8>) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("loopback listener");
    let port = listener.local_addr().expect("listener address").port();
    std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
        // Read the request head first so the response isn't lost to a reset.
        let mut buffer = [0_u8; 8192];
        let mut used = 0;
        while used < buffer.len() {
            match stream.read(&mut buffer[used..]) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    used += count;
                    if buffer[..used].windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
            }
        }
        let _ = stream.write_all(&response);
        let _ = stream.flush();
    });
    format!("http://127.0.0.1:{port}")
}

pub(crate) fn distinct_device_dirs(name: &str) -> Option<(tempfile::TempDir, tempfile::TempDir)> {
    use std::os::unix::fs::MetadataExt;
    let dirs = (|| {
        let first = tempfile::tempdir().ok()?;
        let shm = std::path::Path::new("/dev/shm");
        if !shm.is_dir() {
            return None;
        }
        let second = tempfile::TempDir::new_in(shm).ok()?;
        let first_dev = std::fs::metadata(first.path()).ok()?.dev();
        let second_dev = std::fs::metadata(second.path()).ok()?.dev();
        (first_dev != second_dev).then_some((first, second))
    })();
    if dirs.is_none() {
        assert!(
            std::env::var_os("STRATA_REQUIRE_DEVICE_TESTS").is_none(),
            "{name} requires two filesystems: /dev/shm must be a distinct device from the temp dir"
        );
        eprintln!("Skipping {name}: /dev/shm is not a distinct device from the temp dir");
    }
    dirs
}
