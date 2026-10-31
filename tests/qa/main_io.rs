// SPDX-License-Identifier: MIT

//! The capture side of `scripts/trace-main-io.py`: with `STRATA_QA_MAIN_IO` set,
//! every Strata launch of a run starts under `strace`, tracing only the initial
//! thread, and leaves `<run>/<scenario>__<label>/launch-<n>/` with the trace,
//! the launch's log and the `meta.json` the script classifies paths by.

use std::{
    cell::OnceCell,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::{Map, Value, json};

use crate::{BINARY, Launch, POLL_INTERVAL, REPOSITORY, XDG_DIRECTORIES, shader_cache};

const RUN_VARIABLE: &str = "STRATA_QA_MAIN_IO";
pub(crate) const STALL_VARIABLE: &str = "STRATA_UI_STALL_MS";
const TRACED_CALLS: [&str; 26] = [
    "%file",
    "getdents64",
    "close",
    "fsync",
    "fdatasync",
    "syncfs",
    "flock",
    "fstatfs",
    "fstat",
    "lseek",
    "read",
    "pread64",
    "preadv",
    "write",
    "pwrite64",
    "pwritev",
    "ftruncate",
    "fallocate",
    "fchmod",
    "fchown",
    "fgetxattr",
    "flistxattr",
    "fsetxattr",
    "fremovexattr",
    "copy_file_range",
    "sendfile",
];
/// Left out with `--stacks`, which would otherwise unwind every pipe and socket transfer.
const UNSTACKED_CALLS: [&str; 2] = ["read", "write"];
const TRACED_CHILD_TIMEOUT: Duration = Duration::from_secs(5);

/// One run's share of an audit.
pub(crate) struct Audit {
    directory: PathBuf,
    scenario: String,
    stacks: bool,
}

impl Audit {
    pub(crate) fn for_run(scenario: &str, label: &str) -> Result<Option<Self>, String> {
        let Some(configured) = std::env::var_os(RUN_VARIABLE).filter(|value| !value.is_empty())
        else {
            return Ok(None);
        };
        let root = Path::new(REPOSITORY).join(configured);
        let options = root.join("options.json");
        let stacks = match fs::read_to_string(&options) {
            Ok(text) => serde_json::from_str::<Value>(&text)
                .map_err(|error| format!("{}: {error}", options.display()))?["stacks"]
                .as_bool()
                .unwrap_or(false),
            Err(_) => false,
        };
        Ok(Some(Self {
            directory: root.join(format!("{scenario}__{label}")),
            scenario: format!("{scenario}/{label}"),
            stacks,
        }))
    }

    pub(crate) fn capture(&self, launch: &Launch) -> Result<Capture, String> {
        fs::create_dir_all(&self.directory)
            .map_err(|error| format!("{}: {error}", self.directory.display()))?;
        let directory = (1..)
            .map(|launch| self.directory.join(format!("launch-{launch}")))
            .find(|directory| fs::create_dir(directory).is_ok())
            .ok_or("no free launch directory")?;
        let variable = |name: &str| {
            launch
                .extra
                .iter()
                .rev()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
                .or_else(|| launch.manifest.environment.get(name).map(String::as_str))
        };
        let folders = |list: Option<&str>| -> Vec<String> {
            list.unwrap_or_default()
                .split(':')
                .filter(|folder| !folder.is_empty())
                .map(str::to_owned)
                .collect()
        };
        let data_dirs = folders(variable("XDG_DATA_DIRS"));
        let mut harness = vec![
            shader_cache()?.display().to_string(),
            launch.log.display().to_string(),
            launch.input_log.display().to_string(),
        ];
        harness.extend(folders(std::env::var("PATH").ok().as_deref()));
        let mut meta = Map::new();
        meta.insert("scenario".into(), json!(self.scenario));
        meta.insert("fixture".into(), json!(launch.fixture));
        meta.insert("environment".into(), json!(launch.home));
        for (variable, folder) in XDG_DIRECTORIES {
            let key = variable
                .trim_start_matches("XDG_")
                .trim_end_matches("_HOME");
            meta.insert(key.to_lowercase(), json!(launch.home.join(folder)));
        }
        meta.insert("runtime".into(), json!(launch.runtime));
        meta.insert("data_dirs".into(), json!(data_dirs));
        meta.insert("binary".into(), json!(BINARY));
        // The runner's own places: the shared shader cache, the logs it reads,
        // and the PATH folders GIO looks programs up in.
        meta.insert("harness".into(), json!(harness));
        meta.insert("started".into(), json!(now()));
        meta.insert("ready".into(), Value::Null);
        let capture = Capture {
            directory,
            meta,
            stacks: self.stacks,
            log_kept: false,
        };
        capture.write()?;
        Ok(capture)
    }
}

/// One traced launch.
pub(crate) struct Capture {
    directory: PathBuf,
    meta: Map<String, Value>,
    stacks: bool,
    log_kept: bool,
}

impl Capture {
    /// `strace` running `program`, in place of `Command::new(program)`.
    pub(crate) fn command(&self, program: &str) -> Command {
        let calls: Vec<&str> = TRACED_CALLS
            .into_iter()
            .filter(|call| !(self.stacks && UNSTACKED_CALLS.contains(call)))
            .collect();
        let mut command = Command::new(strace());
        command
            .arg("-o")
            .arg(self.directory.join("trace"))
            .args(["-qq", "-y", "-ttt", "-s", "0"]);
        if self.stacks {
            command.args(["-k", "--stack-trace-frame-limit=64"]);
        }
        command
            .arg("-e")
            .arg(format!("trace={}", calls.join(",")))
            .args(["-e", "signal=none", program]);
        command
    }

    /// The launch's first window is up; later calls are the running phase.
    pub(crate) fn ready(&mut self) {
        if self.meta["ready"].is_null() {
            self.meta.insert("ready".into(), json!(now()));
            let _ = self.write();
        }
    }

    /// Copies the launch's log once, before a restart starts a new one in its place.
    pub(crate) fn keep_log(&mut self, log: &Path) {
        if !self.log_kept {
            self.log_kept = fs::copy(log, self.directory.join("strata.log")).is_ok();
        }
    }

    fn write(&self) -> Result<(), String> {
        let path = self.directory.join("meta.json");
        let text = serde_json::to_string_pretty(&self.meta).map_err(|error| error.to_string())?;
        fs::write(&path, text + "\n").map_err(|error| format!("{}: {error}", path.display()))
    }
}

/// The traced Strata under an `strace` the runner started, found once.
pub(crate) fn traced_child(tracer: u32, found: &OnceCell<u32>) -> Result<u32, String> {
    if let Some(pid) = found.get() {
        return Ok(*pid);
    }
    let parent = format!("\nPPid:\t{tracer}\n");
    let deadline = Instant::now() + TRACED_CHILD_TIMEOUT;
    loop {
        let child = fs::read_dir("/proc")
            .map_err(|error| format!("/proc: {error}"))?
            .flatten()
            .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
            .find(|pid| {
                fs::read_to_string(format!("/proc/{pid}/status"))
                    .is_ok_and(|status| status.contains(&parent))
            });
        if let Some(pid) = child {
            return Ok(*found.get_or_init(|| pid));
        }
        if Instant::now() >= deadline {
            return Err(format!("strace {tracer} has no traced child"));
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Found on the runner's PATH, as a scenario's own PATH may leave it out.
fn strace() -> PathBuf {
    std::env::var_os("PATH")
        .iter()
        .flat_map(std::env::split_paths)
        .map(|folder| folder.join("strace"))
        .find(|program| program.is_file())
        .unwrap_or_else(|| PathBuf::from("strace"))
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64())
}
