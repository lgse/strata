// SPDX-License-Identifier: MIT

use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const TEST_NAME: &str = "sandbox_helper::media::tests::runtime::media_runtime_overrides_decode_without_prlimit_or_nice_on_path";
const HELPER_REQUEST: &str = "STRATA_TEST_MEDIA_RUNTIME_REQUEST";

struct Runtime {
    _directory: tempfile::TempDir,
    input: PathBuf,
    covered: PathBuf,
    bin: PathBuf,
    limiter: PathBuf,
    limits_log: PathBuf,
    priority_log: PathBuf,
    output: PathBuf,
}

fn program(name: &str) -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").expect("test tool path"))
        .map(|directory| directory.join(name))
        .find(|path| path.is_file())
        .unwrap_or_else(|| panic!("{name} is required for media runtime tests"))
        .canonicalize()
        .expect("absolute test tool")
}

fn quoted(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn script(path: &Path, body: &str) {
    fs::write(path, format!("#!{}\n{body}\n", program("sh").display()))
        .expect("runtime tool script");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("executable tool");
}

fn wait(command: &mut Command) {
    let mut child = command.spawn().expect("launch media tool");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().expect("media status") {
            assert!(status.success(), "{command:?}: {status}");
            return;
        }
        if Instant::now() >= deadline {
            child.kill().expect("stop timed-out media tool");
            child.wait().expect("reap media tool");
            panic!("media runtime deadline: {command:?}");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

impl Runtime {
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("media runtime fixture");
        let root = directory.path();
        let input = root.join("clip.mp4");
        wait(
            Command::new(program("ffmpeg"))
                .args(["-nostdin", "-v", "error", "-f", "lavfi", "-i"])
                .arg("testsrc2=size=64x48:rate=30:duration=6")
                .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=6"])
                .args(["-c:v", "libx264", "-threads", "1", "-c:a", "aac"])
                .arg(&input),
        );
        let covered = root.join("covered.mp4");
        wait(
            Command::new(program("ffmpeg"))
                .args(["-nostdin", "-v", "error", "-i"])
                .arg(&input)
                .args(["-f", "lavfi", "-i", "color=c=blue:size=64x48"])
                .args(["-map", "0", "-map", "1:v", "-c", "copy", "-c:v:1", "png"])
                .args([
                    "-frames:v:1",
                    "1",
                    "-threads:v:1",
                    "1",
                    "-disposition:v:1",
                    "attached_pic",
                ])
                .arg(&covered),
        );
        let bin = root.join("bin");
        fs::create_dir(&bin).expect("restricted runtime path");
        let limits_log = root.join("limits.log");
        let priority_log = root.join("priority.log");
        let limiter = root.join("decoder-limits");
        script(
            &limiter,
            &format!(
                "printf '%s\\n' \"$*\" >> {}\nexec {} \"$@\"",
                quoted(&limits_log),
                quoted(&program("prlimit")),
            ),
        );
        script(
            &bin.join("ffmpeg"),
            &format!(
                "IFS= read -r process < /proc/self/stat\nprintf '%s\\n' \"$process\" >> {}\nexec {} \"$@\"",
                quoted(&priority_log),
                quoted(&program("ffmpeg")),
            ),
        );
        symlink(program("ffprobe"), bin.join("ffprobe")).expect("runtime probe");
        Self {
            input,
            covered,
            bin,
            limiter,
            limits_log,
            priority_log,
            output: root.join("decoded.bin"),
            _directory: directory,
        }
    }

    fn decode(&self, operation: &str, value: &str) {
        fs::write(&self.limits_log, []).expect("clear limiter log");
        fs::write(&self.priority_log, []).expect("clear priority log");
        let input = if operation == "audio-cover" {
            &self.covered
        } else {
            &self.input
        };
        let arguments = [
            operation,
            input.to_str().expect("fixture input path"),
            self.output.to_str().expect("fixture output path"),
            value,
            "software",
        ];
        // CI relocates test executables without the separately built application.
        wait(
            Command::new(std::env::current_exe().expect("test executable"))
                .args(["--exact", TEST_NAME, "--nocapture"])
                .env(
                    HELPER_REQUEST,
                    serde_json::to_string(&arguments).expect("helper request"),
                )
                .env("PATH", &self.bin)
                .env("STRATA_SANDBOX_PRLIMIT", &self.limiter)
                .stdout(Stdio::null()),
        );
    }

    fn assert_limits_and_priority(&self, expected_priority: i32) {
        let launches = fs::read_to_string(&self.limits_log).expect("configured limiter ran");
        assert!(!launches.is_empty(), "decoding must not bypass the limiter");
        for launch in launches.lines() {
            for limit in ["--core=0", "--fsize=536870912", "--as=2147483648"] {
                assert!(launch.contains(limit), "{limit}: {launch}");
            }
        }
        let priorities = fs::read_to_string(&self.priority_log).expect("decoder priorities");
        assert!(!priorities.is_empty(), "FFmpeg must actually decode");
        for process in priorities.lines() {
            let fields = process.rsplit_once(") ").expect("process stat").1;
            let nice: i32 = fields
                .split_whitespace()
                .nth(16)
                .expect("nice field")
                .parse()
                .expect("numeric priority");
            assert_eq!(nice, expected_priority, "{process}");
        }
    }
}

#[test]
fn media_runtime_overrides_decode_without_prlimit_or_nice_on_path() {
    if let Ok(request) = std::env::var(HELPER_REQUEST) {
        let arguments: Vec<String> = serde_json::from_str(&request).expect("helper request");
        crate::sandbox_helper::run(&arguments).expect("media helper with configured runtime");
        return;
    }
    let runtime = Runtime::new();
    let priority = rustix::process::getpriority_process(None).expect("test priority");
    for (operation, value, magic, background) in [
        ("preview-media", "64x48", b"STRRAW01", false),
        ("preview-audio", "64x48", b"STRRAW01", false),
        ("audio-peaks", "0", b"STRPEAK1", true),
        ("video-storyboard", "64", b"STRSTB01", true),
        ("audio-cover", "64", b"\x89PNG\r\n\x1a\n", false),
    ] {
        runtime.decode(operation, value);
        let output = fs::read(&runtime.output).expect("decoded media");
        assert!(output.starts_with(magic), "{operation}");
        assert!(
            output.len() > 40,
            "{operation} must produce decoded content"
        );
        runtime.assert_limits_and_priority(if background {
            (priority + 10).min(19)
        } else {
            priority
        });
    }
}
