// SPDX-License-Identifier: MIT

//! Only the single-threaded, codec-free supervisor and its setup child use this
//! module. The GTK application and decoder runtimes must never fork through it.

use std::io;

use rustix::process::{Pid, WaitOptions, waitpid};

fn require_single_thread() -> io::Result<()> {
    let tasks = std::fs::read_dir("/proc/self/task")?
        .take(2)
        .collect::<io::Result<Vec<_>>>()?;
    if tasks.len() != 1 {
        return Err(io::Error::other(
            "Browser supervisor must be single-threaded",
        ));
    }
    Ok(())
}

#[expect(
    unsafe_code,
    reason = "libc fork is required to reuse the codec-free supervisor image without an exec per file"
)]
pub(super) fn fork() -> io::Result<Option<Pid>> {
    require_single_thread()?;
    // SAFETY: Only the codec-free supervisor/setup process calls this, with one
    // verified task, no shared mappings or external locks, and no decoder/RNG
    // state. libc runs its atfork handlers and resets threading-runtime state.
    // Children close inherited control descriptors and exit without reentering
    // the supervisor. No GTK/GIO application process uses this boundary.
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Pid::from_raw(pid))
}

#[expect(
    unsafe_code,
    reason = "namespace unshare is unsafe for shared descriptor tables; this setup process is single-threaded and does not use CLONE_FILES"
)]
pub(super) fn namespaces() -> io::Result<()> {
    use rustix::thread::{UnshareFlags, unshare_unsafe};
    require_single_thread()?;
    let uid = rustix::process::getuid().as_raw();
    let gid = rustix::process::getgid().as_raw();
    let flags =
        UnshareFlags::NEWUSER | UnshareFlags::NEWNS | UnshareFlags::NEWPID | UnshareFlags::NEWIPC;
    // SAFETY: This is a single-threaded disposable setup process. These flags
    // only replace namespaces, never the descriptor table or shared Rust state.
    unsafe { unshare_unsafe(flags) }?;
    std::fs::write("/proc/self/uid_map", format!("0 {uid} 1\n"))?;
    std::fs::write("/proc/self/setgroups", "deny\n")?;
    std::fs::write("/proc/self/gid_map", format!("0 {gid} 1\n"))?;
    rustix::mount::mount_change(
        "/",
        rustix::mount::MountPropagationFlags::PRIVATE | rustix::mount::MountPropagationFlags::REC,
    )?;
    Ok(())
}

pub(super) fn wait(pid: Pid) -> io::Result<bool> {
    loop {
        match waitpid(Some(pid), WaitOptions::empty()) {
            Ok(Some((_, status))) => return Ok(status.exit_status() == Some(0)),
            Ok(None) => continue,
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => return Err(error.into()),
        }
    }
}

pub(super) fn exit(result: Result<(), String>) -> ! {
    let status = match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("Browser decoder failed: {error}");
            1
        }
    };
    // Do not run inherited atexit handlers or flush the supervisor's buffers.
    rustix::runtime::exit_group(status)
}
