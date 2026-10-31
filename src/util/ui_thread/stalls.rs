// SPDX-License-Identifier: MIT

use std::{
    fs::File,
    os::unix::fs::FileExt,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError},
    },
    time::{Duration, Instant},
};

use gtk::glib;

const VARIABLE: &str = "STRATA_UI_STALL_MS";
const PING_INTERVAL: Duration = Duration::from_millis(100);
const AT_FDCWD: i32 = -100;
const POLLED_SHOWN: u64 = 4;

pub(crate) fn start() {
    let Some(limit) = limit(std::env::var(VARIABLE).ok().as_deref()) else {
        return;
    };
    let watched = Watched {
        context: glib::MainContext::default(),
        thread: rustix::thread::gettid().as_raw_nonzero().get(),
        limit,
    };
    if let Err(error) = watch(watched, Arc::default(), |line| tracing::warn!("{line}")) {
        tracing::warn!(%error, "could not start the UI stall watchdog");
    }
}

fn limit(value: Option<&str>) -> Option<Duration> {
    let milliseconds = value?.trim().parse::<u64>().ok()?;
    (milliseconds > 0).then(|| Duration::from_millis(milliseconds))
}

pub(super) struct Watched {
    pub(super) context: glib::MainContext,
    pub(super) thread: i32,
    pub(super) limit: Duration,
}

pub(super) fn watch(
    watched: Watched,
    stopped: Arc<AtomicBool>,
    mut report: impl FnMut(String) + Send + 'static,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("strata-ui-stalls".to_owned())
        .spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                let posted = Instant::now();
                let (answer, answered) = mpsc::sync_channel(1);
                glib::idle_source_new(None, glib::Priority::DEFAULT, move || {
                    let _ = answer.try_send(());
                    glib::ControlFlow::Break
                })
                .attach(Some(&watched.context));
                let mut next_report = watched.limit;
                let mut stalled = false;
                loop {
                    match answered.recv_timeout(next_report.saturating_sub(posted.elapsed())) {
                        Ok(()) => break,
                        Err(RecvTimeoutError::Disconnected) => return,
                        Err(RecvTimeoutError::Timeout) => {
                            if stopped.load(Ordering::Relaxed) {
                                return;
                            }
                            stalled = true;
                            report(format!(
                                "UI thread stalled {} ms {}",
                                posted.elapsed().as_millis(),
                                sample(watched.thread)
                            ));
                            next_report *= 2;
                        }
                    }
                }
                if stalled {
                    report(format!(
                        "UI thread ran again after {} ms",
                        posted.elapsed().as_millis()
                    ));
                }
                std::thread::sleep(PING_INTERVAL.saturating_sub(posted.elapsed()));
            }
        })
        .map(drop)
}

fn sample(thread: i32) -> String {
    let task = PathBuf::from(format!("/proc/self/task/{thread}"));
    let call = std::fs::read_to_string(task.join("syscall")).unwrap_or_default();
    let wchan = std::fs::read_to_string(task.join("wchan")).unwrap_or_default();
    let wchan = match wchan.trim() {
        "" | "0" => "-",
        name => name,
    };
    format!("{} wchan={wchan}", describe(call.trim()))
}

pub(super) fn describe(call: &str) -> String {
    let mut fields = call.split_whitespace();
    let number = match fields.next() {
        Some("running") => return "while running code".to_owned(),
        Some("-1") => return "outside a system call".to_owned(),
        Some(number) => number,
        None => return "in an unknown state".to_owned(),
    };
    let Ok(number) = number.parse::<u64>() else {
        return format!("in an unknown state ({call})");
    };
    let arguments: Vec<u64> = fields.take(6).filter_map(hex).collect();
    let Some(name) = name_of(number) else {
        return format!("in syscall {number}");
    };
    let argument = |index: usize| arguments.get(index).copied();
    let shown = match arguments_of(name) {
        Arguments::Path(path) => argument(path).and_then(read_path).map(quoted),
        Arguments::At(folder, path) => argument(folder)
            .zip(argument(path))
            .and_then(|(folder, path)| at(folder as i32, path))
            .map(quoted),
        Arguments::Descriptor(descriptor) => argument(descriptor)
            .and_then(|descriptor| descriptor_path(descriptor as i32))
            .map(quoted),
        Arguments::Polled => argument(0)
            .zip(argument(1))
            .map(|(at, count)| polled(at, count)),
        Arguments::Other => None,
    };
    format!("in {name}({})", shown.unwrap_or_default())
}

fn hex(field: &str) -> Option<u64> {
    u64::from_str_radix(field.strip_prefix("0x").unwrap_or(field), 16).ok()
}

fn quoted(path: PathBuf) -> String {
    format!("{path:?}")
}

fn at(folder: i32, address: u64) -> Option<PathBuf> {
    let name = if address == 0 {
        PathBuf::new()
    } else {
        read_path(address)?
    };
    if name.is_absolute() {
        return Some(name);
    }
    let base = if folder == AT_FDCWD {
        std::fs::read_link("/proc/self/cwd").ok()?
    } else {
        descriptor_path(folder)?
    };
    Some(if name.as_os_str().is_empty() {
        base
    } else {
        base.join(name)
    })
}

fn descriptor_path(descriptor: i32) -> Option<PathBuf> {
    (descriptor >= 0)
        .then(|| std::fs::read_link(format!("/proc/self/fd/{descriptor}")).ok())
        .flatten()
}

fn polled(address: u64, count: u64) -> String {
    let Ok(memory) = File::open("/proc/self/mem") else {
        return String::new();
    };
    (0..count.min(POLLED_SHOWN))
        .filter_map(|index| {
            let mut entry = [0_u8; 8];
            memory.read_exact_at(&mut entry, address + index * 8).ok()?;
            let descriptor = i32::from_ne_bytes(entry[..4].try_into().ok()?);
            descriptor_path(descriptor).map(quoted)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn read_path(address: u64) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;

    const PAGE: u64 = 4096;
    let memory = File::open("/proc/self/mem").ok()?;
    let mut bytes = Vec::new();
    let mut at = address;
    while bytes.len() < libc::PATH_MAX as usize {
        let mut chunk = vec![0_u8; (PAGE - at % PAGE) as usize];
        let read = memory.read_at(&mut chunk, at).ok()?;
        if read == 0 {
            return None;
        }
        if let Some(end) = chunk[..read].iter().position(|byte| *byte == 0) {
            bytes.extend_from_slice(&chunk[..end]);
            return Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)));
        }
        bytes.extend_from_slice(&chunk[..read]);
        at += read as u64;
    }
    None
}

#[derive(Clone, Copy)]
enum Arguments {
    Path(usize),
    At(usize, usize),
    Descriptor(usize),
    Polled,
    Other,
}

fn arguments_of(name: &str) -> Arguments {
    match name {
        "open" | "stat" | "lstat" | "access" | "truncate" | "chdir" | "rename" | "mkdir"
        | "rmdir" | "creat" | "link" | "unlink" | "readlink" | "chmod" | "chown" | "lchown"
        | "statfs" | "setxattr" | "lsetxattr" | "getxattr" | "lgetxattr" | "listxattr"
        | "llistxattr" | "removexattr" | "lremovexattr" | "execve" => Arguments::Path(0),
        "symlink" | "inotify_add_watch" => Arguments::Path(1),
        "openat" | "openat2" | "mkdirat" | "mknodat" | "fchownat" | "newfstatat" | "unlinkat"
        | "renameat" | "renameat2" | "linkat" | "readlinkat" | "fchmodat" | "fchmodat2"
        | "faccessat" | "faccessat2" | "utimensat" | "name_to_handle_at" | "execveat" | "statx"
        | "open_tree" => Arguments::At(0, 1),
        "symlinkat" => Arguments::At(1, 2),
        "read" | "write" | "close" | "fstat" | "lseek" | "ioctl" | "pread64" | "pwrite64"
        | "readv" | "writev" | "fcntl" | "flock" | "fsync" | "fdatasync" | "ftruncate"
        | "getdents64" | "fchdir" | "fchmod" | "fchown" | "fstatfs" | "fsetxattr" | "fgetxattr"
        | "flistxattr" | "fremovexattr" | "fadvise64" | "sync_file_range" | "fallocate"
        | "syncfs" | "copy_file_range" | "recvmsg" | "sendmsg" => Arguments::Descriptor(0),
        "sendfile" => Arguments::Descriptor(1),
        "poll" | "ppoll" => Arguments::Polled,
        _ => Arguments::Other,
    }
}

fn name_of(number: u64) -> Option<&'static str> {
    NAMES
        .iter()
        .find_map(|(known, name)| (*known == number).then_some(*name))
}

#[cfg(target_arch = "x86_64")]
const NAMES: &[(u64, &str)] = &[
    (0, "read"),
    (1, "write"),
    (2, "open"),
    (3, "close"),
    (4, "stat"),
    (5, "fstat"),
    (6, "lstat"),
    (7, "poll"),
    (8, "lseek"),
    (16, "ioctl"),
    (17, "pread64"),
    (18, "pwrite64"),
    (19, "readv"),
    (20, "writev"),
    (21, "access"),
    (35, "nanosleep"),
    (40, "sendfile"),
    (46, "sendmsg"),
    (47, "recvmsg"),
    (59, "execve"),
    (61, "wait4"),
    (72, "fcntl"),
    (73, "flock"),
    (74, "fsync"),
    (75, "fdatasync"),
    (76, "truncate"),
    (77, "ftruncate"),
    (80, "chdir"),
    (81, "fchdir"),
    (82, "rename"),
    (83, "mkdir"),
    (84, "rmdir"),
    (85, "creat"),
    (86, "link"),
    (87, "unlink"),
    (88, "symlink"),
    (89, "readlink"),
    (90, "chmod"),
    (91, "fchmod"),
    (92, "chown"),
    (93, "fchown"),
    (94, "lchown"),
    (137, "statfs"),
    (138, "fstatfs"),
    (162, "sync"),
    (188, "setxattr"),
    (189, "lsetxattr"),
    (190, "fsetxattr"),
    (191, "getxattr"),
    (192, "lgetxattr"),
    (193, "fgetxattr"),
    (194, "listxattr"),
    (195, "llistxattr"),
    (196, "flistxattr"),
    (197, "removexattr"),
    (198, "lremovexattr"),
    (199, "fremovexattr"),
    (202, "futex"),
    (217, "getdents64"),
    (221, "fadvise64"),
    (230, "clock_nanosleep"),
    (232, "epoll_wait"),
    (247, "waitid"),
    (254, "inotify_add_watch"),
    (257, "openat"),
    (258, "mkdirat"),
    (259, "mknodat"),
    (260, "fchownat"),
    (262, "newfstatat"),
    (263, "unlinkat"),
    (264, "renameat"),
    (265, "linkat"),
    (266, "symlinkat"),
    (267, "readlinkat"),
    (268, "fchmodat"),
    (269, "faccessat"),
    (271, "ppoll"),
    (277, "sync_file_range"),
    (280, "utimensat"),
    (281, "epoll_pwait"),
    (285, "fallocate"),
    (303, "name_to_handle_at"),
    (306, "syncfs"),
    (316, "renameat2"),
    (322, "execveat"),
    (326, "copy_file_range"),
    (332, "statx"),
    (428, "open_tree"),
    (437, "openat2"),
    (439, "faccessat2"),
    (452, "fchmodat2"),
];

#[cfg(target_arch = "aarch64")]
const NAMES: &[(u64, &str)] = &[
    (5, "setxattr"),
    (6, "lsetxattr"),
    (7, "fsetxattr"),
    (8, "getxattr"),
    (9, "lgetxattr"),
    (10, "fgetxattr"),
    (11, "listxattr"),
    (12, "llistxattr"),
    (13, "flistxattr"),
    (14, "removexattr"),
    (15, "lremovexattr"),
    (16, "fremovexattr"),
    (22, "epoll_pwait"),
    (25, "fcntl"),
    (27, "inotify_add_watch"),
    (29, "ioctl"),
    (32, "flock"),
    (33, "mknodat"),
    (34, "mkdirat"),
    (35, "unlinkat"),
    (36, "symlinkat"),
    (37, "linkat"),
    (38, "renameat"),
    (43, "statfs"),
    (44, "fstatfs"),
    (45, "truncate"),
    (46, "ftruncate"),
    (47, "fallocate"),
    (48, "faccessat"),
    (49, "chdir"),
    (50, "fchdir"),
    (52, "fchmod"),
    (53, "fchmodat"),
    (54, "fchownat"),
    (55, "fchown"),
    (56, "openat"),
    (57, "close"),
    (61, "getdents64"),
    (62, "lseek"),
    (63, "read"),
    (64, "write"),
    (65, "readv"),
    (66, "writev"),
    (67, "pread64"),
    (68, "pwrite64"),
    (71, "sendfile"),
    (73, "ppoll"),
    (78, "readlinkat"),
    (79, "newfstatat"),
    (80, "fstat"),
    (81, "sync"),
    (82, "fsync"),
    (83, "fdatasync"),
    (84, "sync_file_range"),
    (88, "utimensat"),
    (95, "waitid"),
    (98, "futex"),
    (101, "nanosleep"),
    (115, "clock_nanosleep"),
    (211, "sendmsg"),
    (212, "recvmsg"),
    (221, "execve"),
    (223, "fadvise64"),
    (260, "wait4"),
    (264, "name_to_handle_at"),
    (267, "syncfs"),
    (276, "renameat2"),
    (281, "execveat"),
    (285, "copy_file_range"),
    (291, "statx"),
    (428, "open_tree"),
    (437, "openat2"),
    (439, "faccessat2"),
    (452, "fchmodat2"),
];

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
const NAMES: &[(u64, &str)] = &[];

#[cfg(test)]
mod tests;
