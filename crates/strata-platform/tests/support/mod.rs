//! The compositor and D-Bus harness (W3): each test gets a private session of its own.
//!
//! [`Session::start`] brings up, under a private `XDG_RUNTIME_DIR`:
//!
//! 1. a watchdog, first, in a process group of its own that ignores Ctrl-C: if the test process
//!    dies without tearing down (SIGKILL, SIGINT, a timeout), it kills the session's processes
//!    (those started with `XDG_RUNTIME_DIR` set to the session's directory, and no other),
//!    removes the Xwayland lock and socket a killed sway leaves, unmounts what is mounted in the
//!    directory and removes it;
//! 2. a session bus ([`Bus`]): a `dbus-daemon` with only the session's service directory;
//! 3. a nested headless sway, with Xwayland (`sway.rs` lists the traps it works around);
//! 4. an input peer ([`Session::input`]): a virtual pointer and keyboard, without which a
//!    headless seat gives no client pointer events or keyboard focus.
//!
//! Processes start through [`Session::command`], with an environment built from nothing
//! ([`Session::env`]), so none can reach the desktop's display, bus or runtime directory.
//! Dropping the session stops everything and removes the runtime directory. Logs stay in
//! `target/tmp/session/<test>/`.
//!
//! Every test that starts a session is `#[ignore]`d: `mise run test-platform` runs them, and
//! `mise run test` never does. See docs/testing.md, "Compositor and D-Bus tests".

mod bus;
mod gpu;
mod peer;
mod process;
mod sway;

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

pub use bus::Bus;
pub use peer::Peer;
pub use process::Proc;
pub use sway::{Shell, Window};

/// The display a process connects to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Display {
    /// The session's sway, over Wayland (`WAYLAND_DISPLAY` only).
    Wayland,
    /// The session's Xwayland (`DISPLAY` only), so winit takes its X11 backend.
    X11,
    /// Neither: a process that needs only the bus.
    None,
}

pub struct Options {
    /// Start sway. Off for tests that need only the bus (P5, P8).
    pub compositor: bool,
    /// The headless output's size in pixels.
    pub output: (u32, u32),
    pub scale: f64,
    /// Xwayland, started by sway on first use.
    pub xwayland: bool,
    /// The input peer's virtual pointer and keyboard.
    pub input: bool,
    /// Lines appended to the sway config.
    pub sway_config: String,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            compositor: true,
            output: (1280, 800),
            scale: 1.0,
            xwayland: true,
            input: true,
            sway_config: String::new(),
        }
    }
}

/// How long any wait lasts: 10 s times `STRATA_TEST_TIMEOUT_SCALE` (CI sets 3).
pub fn timeout() -> Duration {
    let scale = std::env::var("STRATA_TEST_TIMEOUT_SCALE")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|s| *s > 0.0)
        .unwrap_or(1.0);
    Duration::from_secs_f64(10.0 * scale)
}

/// An example of strata-platform, built beside the test binary
/// (`target/<profile>/examples/<name>`). `mise run test-platform` builds them first:
/// `cargo test` leaves examples under hashed names only.
pub fn example(name: &str) -> PathBuf {
    let exe = std::env::current_exe().expect("test binary path");
    let path = exe
        .parent()
        .and_then(|deps| deps.parent())
        .map(|profile| profile.join("examples").join(name))
        .expect("target directory");
    assert!(
        path.is_file(),
        "no example at {}: build it with `cargo build -p strata-platform --examples \
         --all-features`, or run `mise run test-platform`, which does",
        path.display()
    );
    path
}

/// The last `n` lines of a log, headed by its path.
fn tail(path: &Path, n: usize) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(n);
    format!("--- {} ---\n{}", path.display(), lines[start..].join("\n"))
}

/// Appends a line to the file `STRATA_SESSION_REPORT` names, if any: how a test that runs
/// another test binary (W3's SIGKILL checks) learns the session's directory and processes.
pub fn report(line: &str) {
    let Some(path) = std::env::var_os("STRATA_SESSION_REPORT") else {
        return;
    };
    if let Ok(mut f) = std::fs::File::options()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{line}");
    }
}

/// The desktop's display, as the shell had it: `mise run test-platform` saves it in
/// `STRATA_OUTER_*` before clearing it.
struct Outer {
    display: Option<String>,
    /// The desktop's Wayland socket.
    wayland: Option<PathBuf>,
}

impl Outer {
    fn get() -> Outer {
        let var = |saved: &str, live: &str| {
            std::env::var(saved)
                .ok()
                .or_else(|| std::env::var(live).ok())
                .filter(|v| !v.is_empty())
        };
        let runtime = var("STRATA_OUTER_XDG_RUNTIME_DIR", "XDG_RUNTIME_DIR");
        let wayland = var("STRATA_OUTER_WAYLAND_DISPLAY", "WAYLAND_DISPLAY").map(|w| {
            let w = PathBuf::from(w);
            match (&runtime, w.is_absolute()) {
                (Some(dir), false) => Path::new(dir).join(w),
                _ => w,
            }
        });
        Outer {
            display: var("STRATA_OUTER_DISPLAY", "DISPLAY"),
            wayland,
        }
    }
}

pub struct Session {
    name: String,
    dir: PathBuf,
    logs: PathBuf,
    icd: Option<OsString>,
    outer: Outer,
    input: Option<Peer>,
    sway: Option<sway::Sway>,
    bus: Option<Bus>,
    watchdog: Child,
}

impl Session {
    /// A session with the defaults: sway, Xwayland, the bus and the input peer.
    pub fn start() -> Session {
        Session::with(Options::default())
    }

    pub fn with(opts: Options) -> Session {
        static N: AtomicU32 = AtomicU32::new(0);
        static NAMES: Mutex<Option<HashMap<String, u32>>> = Mutex::new(None);
        let n = N.fetch_add(1, Ordering::Relaxed);
        // `STRATA_SESSION_NAME` names the logs of a test run in a binary of its own (W3's kill
        // tests run several at once); otherwise the test's name does.
        let base = match (
            std::env::var("STRATA_SESSION_NAME")
                .ok()
                .filter(|v| !v.is_empty()),
            std::thread::current().name(),
        ) {
            (Some(name), _) => name,
            (None, Some(t)) if t != "main" => t.replace("::", "-"),
            _ => format!("session-{n}"),
        };
        // A second session in the same test gets a log directory of its own.
        let name = {
            let mut names = NAMES.lock().expect("session names");
            let count = names
                .get_or_insert_with(HashMap::new)
                .entry(base.clone())
                .or_default();
            *count += 1;
            match *count {
                1 => base,
                k => format!("{base}-{k}"),
            }
        };
        let logs = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join("session")
            .join(&name);
        let _ = std::fs::remove_dir_all(&logs);
        std::fs::create_dir_all(&logs).expect("session log directory");
        // Unique even when this test process's pid is reused (its start time tells them apart),
        // and short: sockets in it (`sway-ipc.<uid>.<pid>.sock`, about 30 bytes) must fit in a
        // Unix socket path's 108 bytes.
        let dir = PathBuf::from(format!(
            "/tmp/strata-session-{}-{}-{n}",
            std::process::id(),
            start_time("self").expect("this process's start time")
        ));
        assert!(
            dir.as_os_str().len() + 40 < 108,
            "{} is too long for socket paths in it",
            dir.display()
        );
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&dir)
            .unwrap_or_else(|e| panic!("creating {}: {e}", dir.display()));
        report(&format!("dir {}", dir.display()));
        let watchdog = watchdog(&dir);
        report(&format!("watchdog {}", watchdog.id()));
        for sub in [
            "home",
            "home/.config",
            "home/.local/share",
            "home/.cache",
            "home/.local/state",
        ] {
            std::fs::create_dir_all(dir.join(sub)).expect("session home");
        }
        let bus = Bus::start(&dir, &logs, &base_env(&dir, None), |pid| record(&dir, pid));
        let mut session = Session {
            name,
            outer: Outer::get(),
            icd: gpu::icd(),
            bus: Some(bus),
            dir,
            logs,
            input: None,
            sway: None,
            watchdog,
        };
        if opts.compositor {
            let renderer = gpu::renderer(session.icd.as_deref());
            let env = base_env(&session.dir, Some(session.bus().address()));
            let dir = session.dir.clone();
            session.sway = Some(sway::Sway::start(
                &session.dir,
                &session.logs,
                &opts,
                &renderer,
                &env,
                |pid| {
                    record(&dir, pid);
                    report("sway-starting");
                },
            ));
            std::fs::write(
                session.logs.join("session.txt"),
                format!(
                    "renderer {renderer}\nvulkan icd {:?}\nruntime dir {}\n",
                    session.icd,
                    session.dir.display()
                ),
            )
            .expect("session notes");
        }
        if opts.compositor && opts.input {
            let input = Peer::spawn(&session, Display::Wayland, "input");
            let (w, h) = opts.output;
            let logical = |px: u32| (f64::from(px) / opts.scale).round() as u32;
            input.devices([logical(w), logical(h)]);
            session.input = Some(input);
        }
        report("ready");
        session
    }

    fn sway(&self) -> &sway::Sway {
        self.sway
            .as_ref()
            .expect("this session has no compositor (Options::compositor)")
    }

    /// The environment of a process in this session, built from nothing. It passes on
    /// `STRATA_TEST_TIMEOUT_SCALE` and `LD_LIBRARY_PATH` where set; ICD paths in
    /// `STRATA_VK_ICD` should be absolute.
    pub fn env(&self, display: Display) -> Vec<(OsString, OsString)> {
        let mut env: Vec<(OsString, OsString)> = base_env(&self.dir, Some(self.bus().address()))
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        let mut set = |k: &str, v: &OsStr| env.push((k.into(), v.to_owned()));
        match display {
            Display::Wayland => {
                let name = self.sway().wayland_display();
                assert!(
                    self.outer.wayland.as_deref() != Some(self.dir.join(name).as_path()),
                    "the session's Wayland socket {name} is the desktop's"
                );
                set("WAYLAND_DISPLAY", name.as_ref());
                set("XDG_SESSION_TYPE", "wayland".as_ref());
            }
            Display::X11 => {
                let x = self
                    .sway()
                    .x_display_name()
                    .expect("this session has no Xwayland (Options::xwayland)");
                assert!(
                    self.outer.display.as_deref() != Some(x),
                    "the session's DISPLAY {x} is the desktop's"
                );
                set("DISPLAY", x.as_ref());
                set("XDG_SESSION_TYPE", "x11".as_ref());
            }
            Display::None => {}
        }
        if self.sway.is_some() {
            set("XDG_CURRENT_DESKTOP", "sway".as_ref());
        }
        if let Some(icd) = &self.icd {
            set("VK_ICD_FILENAMES", icd);
            set("VK_DRIVER_FILES", icd);
        }
        for pass in ["STRATA_TEST_TIMEOUT_SCALE", "LD_LIBRARY_PATH"] {
            if let Some(v) = std::env::var_os(pass) {
                set(pass, &v);
            }
        }
        set("RUNE_GPU_LOG", "1".as_ref());
        set("RUST_BACKTRACE", "1".as_ref());
        set("STRATA_PEER_DIR", self.logs.as_os_str());
        env
    }

    /// `program` with this session's environment ([`Session::env`]), run from its home.
    pub fn command(&self, program: impl AsRef<OsStr>, display: Display) -> Command {
        let mut cmd = Command::new(program);
        cmd.env_clear()
            .envs(self.env(display))
            .current_dir(self.dir.join("home"));
        cmd
    }

    /// Starts `cmd` (from [`Session::command`]), logging as `<name>.stdout.log` and
    /// `<name>.stderr.log` in the session's log directory (`<name>-2` and so on for a name
    /// already taken).
    pub fn spawn(&self, name: &str, cmd: Command) -> Proc {
        let mut unique = name.to_owned();
        let mut n = 1;
        while self.logs.join(format!("{unique}.stdout.log")).exists() {
            n += 1;
            unique = format!("{name}-{n}");
        }
        let proc = Proc::spawn(&unique, cmd, &self.logs);
        record(&self.dir, proc.pid());
        proc
    }

    /// A new peer process on `display`.
    pub fn peer(&self, display: Display) -> Peer {
        let name = match display {
            Display::Wayland => "peer-wayland",
            Display::X11 => "peer-x11",
            Display::None => panic!("a peer needs a display"),
        };
        Peer::spawn(self, display, name)
    }

    /// The session's virtual pointer and keyboard, in layout points.
    pub fn input(&self) -> &Peer {
        self.input
            .as_ref()
            .expect("this session has no input peer (Options::input)")
    }

    pub fn bus(&self) -> &Bus {
        self.bus.as_ref().expect("the session bus")
    }

    /// Runs sway commands (`[con_id=4] focus`) and panics unless all succeed.
    pub fn sway_command(&self, cmd: &str) {
        self.sway().command(cmd);
    }

    /// sway's whole tree, as `swaymsg -t get_tree` gives it.
    pub fn tree(&self) -> serde_json::Value {
        self.sway().tree()
    }

    /// Waits for a toplevel of process `pid` with a size.
    pub fn wait_window(&self, pid: u32) -> Window {
        self.wait_window_where(&format!("a window of pid {pid}"), |w| {
            w.pid == pid && w.content.width > 0.0
        })
    }

    /// Waits for a toplevel that passes `pred`; `what` names it in the failure.
    pub fn wait_window_where(&self, what: &str, pred: impl Fn(&Window) -> bool) -> Window {
        let deadline = Instant::now() + timeout();
        loop {
            let windows = sway::windows(&self.tree());
            if let Some(w) = windows.iter().find(|w| pred(w)) {
                return w.clone();
            }
            assert!(
                Instant::now() < deadline,
                "{}: no {what} in sway's tree, which has {windows:#?}",
                self.name
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Waits until each of `pids` has a toplevel and no two overlap: sway has tiled them side
    /// by side. The clients may not have drawn at their new size yet: sync each one before
    /// sending input.
    pub fn wait_windows(&self, pids: &[u32]) -> Vec<Window> {
        let deadline = Instant::now() + timeout();
        loop {
            let all = sway::windows(&self.tree());
            let found: Vec<Window> = pids
                .iter()
                .filter_map(|p| all.iter().find(|w| w.pid == *p).cloned())
                .collect();
            let apart = found.len() == pids.len()
                && found.iter().enumerate().all(|(i, a)| {
                    found[i + 1..]
                        .iter()
                        .all(|b| !a.content.overlaps(&b.content))
                });
            if apart {
                return found;
            }
            assert!(
                Instant::now() < deadline,
                "{}: no windows of pids {pids:?} side by side in sway's tree, which has {all:#?}",
                self.name
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Moves the input peer's pointer to `at` until `reached` holds, checking for 200 ms after
    /// each move, and moving again a point off if it doesn't.
    ///
    /// For a moment after sway re-tiles, a motion can reach no surface (likely sway's saved
    /// snapshots of windows it is resizing), and a press then goes nowhere. `reached` says how
    /// the test knows the pointer got there: a peer's event ([`Session::point_at`]), an app's
    /// log line.
    pub fn point_until(&self, at: (f64, f64), reached: impl Fn() -> bool) {
        let deadline = Instant::now() + timeout();
        let mut nudge = 0.0;
        loop {
            // Replies once sway has the motion.
            self.input().pointer_move((at.0 + nudge, at.1 + nudge));
            let settle = Instant::now() + Duration::from_millis(200);
            loop {
                if reached() {
                    return;
                }
                if Instant::now() >= settle {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(
                Instant::now() < deadline,
                "{}: the pointer at {at:?} never got there",
                self.name
            );
            nudge = 1.0 - nudge;
        }
    }

    /// Moves the pointer to `at` until `peer` reports an enter or a motion that came after the
    /// first move: an enter from before it doesn't count. Each check syncs with the peer first,
    /// so it has read whatever sway sent it (Xwayland passes motion on later, so X11 peers may
    /// take another check).
    pub fn point_at(&self, peer: &Peer, at: (f64, f64)) {
        let mark = peer.line_count();
        self.point_until(at, || {
            peer.sync();
            peer.pointer_since(mark, Duration::ZERO)
        });
    }

    /// Gives `window` keyboard focus and waits until sway reports it.
    pub fn focus(&self, window: &Window) -> Window {
        self.sway_command(&format!("[con_id={}] focus", window.id));
        let id = window.id;
        self.wait_window_where("the window focused", |w| w.id == id && w.focused)
    }

    /// The private runtime directory (`XDG_RUNTIME_DIR`).
    pub fn runtime_dir(&self) -> &Path {
        &self.dir
    }

    /// Xwayland's pid and display number, once an X client has started it. wlroots forks
    /// Xwayland twice, so it isn't sway's child: find it by its command line, `Xwayland :N`.
    pub fn xwayland(&self) -> Option<(u32, u32)> {
        let name = self.sway.as_ref()?.x_display_name()?;
        let display = name.trim_start_matches(':').parse().ok()?;
        let pid = std::fs::read_dir("/proc").ok()?.flatten().find_map(|e| {
            let pid: u32 = e.file_name().to_str()?.parse().ok()?;
            let cmdline = std::fs::read(e.path().join("cmdline")).ok()?;
            let mut args = cmdline.split(|b| *b == 0);
            let program = args.next()?;
            (program.ends_with(b"Xwayland") && args.next()? == name.as_bytes()).then_some(pid)
        })?;
        Some((pid, display))
    }
}

/// Adds `pid` to the session's `pids` file, for diagnostics and the SIGKILL test's report.
fn record(dir: &Path, pid: u32) {
    if let Ok(mut f) = std::fs::File::options()
        .create(true)
        .append(true)
        .open(dir.join("pids"))
    {
        let _ = writeln!(f, "{pid}");
    }
    report(&format!("pid {pid}"));
}

/// The watchdog's script: wait for the test process to go, then clean up after it. `$1` is the
/// test's pid, `$2` its start time (field 22 of `/proc/<pid>/stat`), `$3` the session
/// directory.
///
/// It acts only on the session's processes: those whose environment has `XDG_RUNTIME_DIR=$3`
/// (the desktop's is `/run/user/<uid>`), found by scanning `/proc`, never by a pid number that
/// may have been reused. D-Bus-activated services count too. A killed sway leaves Xwayland's
/// lock file and socket behind; the display comes from the sockets the session's processes
/// hold, and a lock goes only if it names one of them.
const WATCHDOG: &str = r#"
trap '' INT TERM HUP
t=$1; since=$2; d=$3
alive() {
  s=$(cat "/proc/$t/stat" 2>/dev/null) || return 1
  set -- ${s##*") "}
  [ "$1" != Z ] && [ "${20}" = "$since" ]
}
while alive; do sleep 0.2; done
[ -d "$d" ] || exit 0
# The session's processes: one grep over every environ. `mine` checks one again just before it
# is signalled, in case it exited and its pid went to someone else in between.
ours() {
  grep -lxzF "XDG_RUNTIME_DIR=$d" /proc/[0-9]*/environ 2>/dev/null |
    sed 's#^/proc/##; s#/environ$##'
}
mine() { grep -qxzF "XDG_RUNTIME_DIR=$d" "/proc/$1/environ" 2>/dev/null; }
pids=$(ours)
inodes=$(for p in $pids; do for f in /proc/$p/fd/*; do readlink "$f"; done; done 2>/dev/null |
  sed -n 's/^socket:\[\([0-9]*\)\]$/\1/p' | sort -u)
if [ -n "$inodes" ]; then
  displays=$(printf '%s\n' "$inodes" | awk 'NR == FNR { own[$1] = 1; next }
    FNR > 1 && ($7 in own) && $8 ~ /^@?\/tmp\/\.X11-unix\/X[0-9]+$/ { sub(/.*X/, "", $8); print $8 }' \
    - /proc/net/unix | sort -u)
  for n in $displays; do
    owner=$(tr -d ' \n' < "/tmp/.X$n-lock" 2>/dev/null)
    [ -n "$owner" ] || continue
    for p in $pids; do
      [ "$p" = "$owner" ] && rm -f "/tmp/.X$n-lock" "/tmp/.X11-unix/X$n"
    done
  done
fi
for round in 1 2 3 4 5; do
  [ -n "$pids" ] || break
  for p in $pids; do
    mine "$p" && kill -KILL "$p" 2>/dev/null
  done
  sleep 0.2
  pids=$(ours)
done
awk -v d="$d/" '{ m = $5; gsub(/\\040/, " ", m); gsub(/\\011/, "\t", m); gsub(/\\134/, "\\", m) }
  index(m "/", d) == 1 { print m }' /proc/self/mountinfo |
  while IFS= read -r m; do
    fusermount3 -u -z "$m" 2>/dev/null || fusermount -u -z "$m" 2>/dev/null || umount -l "$m" 2>/dev/null
  done
rm -rf --one-file-system "$d"
"#;

/// Starts the watchdog for `dir` (see [`WATCHDOG`]), in a process group of its own so a Ctrl-C
/// meant for the tests doesn't reach it. It runs until the test process is gone, or until the
/// session kills it after tearing down.
fn watchdog(dir: &Path) -> Child {
    Command::new("sh")
        .arg("-c")
        .arg(WATCHDOG)
        .arg("watchdog")
        .arg(std::process::id().to_string())
        .arg(start_time("self").expect("this process's start time"))
        .arg(dir)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .current_dir("/")
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("starting the session watchdog")
}

/// Field 22 of `/proc/<pid>/stat`: when the process started, which tells it from a later
/// process that reuses its pid.
fn start_time(pid: &str) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, rest) = stat.rsplit_once(") ")?;
    rest.split(' ').nth(19).map(str::to_owned)
}

/// Whether `pid` runs (a zombie doesn't).
pub fn alive(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| {
            stat.rsplit_once(") ")
                .map(|(_, rest)| !rest.starts_with('Z'))
        })
        .unwrap_or(false)
}

/// The live processes started with `XDG_RUNTIME_DIR` set to `dir`: a session's own, D-Bus
/// activated services and Xwayland (which wlroots forks twice) included.
pub fn processes_of(dir: &Path) -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| started_with(*pid, dir))
        .filter(|pid| alive(*pid))
        .collect()
}

/// Whether process `pid` was started with `XDG_RUNTIME_DIR` set to `dir`.
fn started_with(pid: u32, dir: &Path) -> bool {
    let want = format!("XDG_RUNTIME_DIR={}", dir.display());
    std::fs::read(format!("/proc/{pid}/environ"))
        .is_ok_and(|env| env.split(|b| *b == 0).any(|v| v == want.as_bytes()))
}

/// A mount point from `/proc/self/mountinfo`, with its octal escapes (`\040` for a space)
/// undone.
fn unescape_mount(point: &str) -> String {
    point
        .replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\")
}

/// What every process of a session gets: a private home and runtime directory, the session
/// bus, and settings that keep GLib from reaching outside (gvfs, dconf, the a11y bus).
fn base_env(dir: &Path, bus: Option<&str>) -> Vec<(String, String)> {
    let home = dir.join("home");
    let at = |sub: &str| home.join(sub).display().to_string();
    let mut env = vec![
        ("HOME".into(), home.display().to_string()),
        ("XDG_RUNTIME_DIR".into(), dir.display().to_string()),
        ("XDG_CONFIG_HOME".into(), at(".config")),
        ("XDG_DATA_HOME".into(), at(".local/share")),
        ("XDG_CACHE_HOME".into(), at(".cache")),
        ("XDG_STATE_HOME".into(), at(".local/state")),
        (
            "PATH".into(),
            std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into()),
        ),
        ("LANG".into(), "C.UTF-8".into()),
        ("GIO_USE_VFS".into(), "local".into()),
        ("GVFS_DISABLE_FUSE".into(), "1".into()),
        ("GSETTINGS_BACKEND".into(), "memory".into()),
        ("NO_AT_BRIDGE".into(), "1".into()),
    ];
    if let Some(bus) = bus {
        env.push(("DBUS_SESSION_BUS_ADDRESS".into(), bus.into()));
    }
    env
}

impl Drop for Session {
    fn drop(&mut self) {
        // The input peer, then sway (its clients and Xwayland go with it), then the bus.
        self.input.take();
        if let Some(mut sway) = self.sway.take() {
            sway.stop();
        }
        self.bus.take();
        // Nobody waits for what wlroots forked (Xwayland) or the bus activated: wait for every
        // process started with the session's directory to go, then kill what is left.
        let deadline = Instant::now() + timeout();
        let mut left = processes_of(&self.dir);
        while !left.is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
            left = processes_of(&self.dir);
        }
        if !left.is_empty() {
            eprintln!("{}: killing what outlived the session: {left:?}", self.name);
            for pid in left {
                // Again just before the signal: the pid may have gone to another process since
                // the scan. (A pidfd would close the gap, but not without unsafe code.)
                if started_with(pid, &self.dir) {
                    let _ = Command::new("kill")
                        .args(["-KILL", &pid.to_string()])
                        .status();
                }
            }
            while !processes_of(&self.dir).is_empty() && Instant::now() < deadline + timeout() {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        // A FUSE mount a GIO service left would keep the runtime directory from going.
        if let Ok(mounts) = std::fs::read_to_string("/proc/self/mountinfo") {
            for point in mounts.lines().filter_map(|l| l.split(' ').nth(4)) {
                let point = unescape_mount(point);
                if Path::new(&point).starts_with(&self.dir) {
                    let _ = Command::new("fusermount3")
                        .args(["-u", "-z", &point])
                        .status();
                }
            }
        }
        let _ = std::fs::remove_dir_all(&self.dir);
        let _ = self.watchdog.kill();
        let _ = self.watchdog.wait();
        if std::thread::panicking() {
            eprintln!("{}: session logs in {}", self.name, self.logs.display());
            eprintln!("{}", tail(&self.logs.join("sway.log"), 20));
        }
    }
}
