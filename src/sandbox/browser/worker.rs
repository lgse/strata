// SPDX-License-Identifier: MIT

use std::{
    io::{self, Write},
    os::unix::net::UnixStream,
};

use super::{
    process,
    wire::{self, Operation},
};

/// This process parses only the private control protocol, never media. Decoders
/// receive a source on stdin and a one-way output pipe, not the control socket.
pub(crate) fn run() -> Result<(), String> {
    restrict_mutations()?;
    let socket = UnixStream::from(rustix::io::dup(std::io::stdin()).map_err(|e| e.to_string())?);
    rustix::io::fcntl_setfd(&socket, rustix::io::FdFlags::CLOEXEC).map_err(|e| e.to_string())?;
    while let Some((operation, input, write)) = wire::receive(&socket).map_err(|e| e.to_string())? {
        let child = match process::fork().map_err(|e| e.to_string())? {
            Some(child) => child,
            None => {
                drop(socket);
                let redirected = rustix::stdio::dup2_stdin(&input)
                    .and_then(|()| rustix::stdio::dup2_stdout(&write));
                drop(input);
                drop(write);
                process::exit(
                    redirected
                        .map_err(|e| e.to_string())
                        .and_then(|()| isolated_job(operation)),
                );
            }
        };
        drop(input);
        drop(write);
        // Bytes go directly to the app. Buffering previous thumbnails here
        // would expose their contents in the next decoder's fork snapshot.
        let success = process::wait(child).map_err(|e| e.to_string())?;
        (&socket)
            .write_all(&[u8::from(success)])
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn isolated_job(operation: Operation) -> Result<(), String> {
    process::namespaces().map_err(|e| e.to_string())?;
    match process::fork().map_err(|e| e.to_string())? {
        Some(child) => {
            if process::wait(child).map_err(|e| e.to_string())? {
                Ok(())
            } else {
                Err("Decoder exited unsuccessfully".into())
            }
        }
        None => process::exit(decode(operation)),
    }
}

fn decode(operation: Operation) -> Result<(), String> {
    rustix::process::setsid().map_err(|e| e.to_string())?;
    rustix::mount::mount(
        "proc",
        "/proc",
        "proc",
        rustix::mount::MountFlags::NOSUID
            | rustix::mount::MountFlags::NODEV
            | rustix::mount::MountFlags::NOEXEC,
        None,
    )
    .map_err(|e| e.to_string())?;
    isolate_job().map_err(|e| e.to_string())?;
    protect_input(&["/tmp", "/dev/shm", "/dev/null"])?;
    crate::sandbox_helper::browser_render(std::path::Path::new("/proc/1/fd/0"), operation)
        .write(&mut std::io::stdout())
        .map_err(|e| e.to_string())
}

fn write_ruleset() -> Result<landlock::RulesetCreated, landlock::RulesetError> {
    use landlock::{ABI, AccessFs, CompatLevel, Compatible, Ruleset, RulesetAttr};
    Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(AccessFs::from_write(ABI::V3))?
        .create()
}

pub(super) fn supported() -> bool {
    write_ruleset().is_ok()
}

pub(super) fn protect_input(writable: &[&str]) -> Result<(), String> {
    use landlock::{ABI, AccessFs, PathBeneath, PathFd, RulesetCreatedAttr, RulesetStatus};
    // O_RDONLY alone does not stop /proc/self/fd from reopening an input for
    // writing. Require Landlock's truncate protection too (ABI 3), never best effort.
    let mut ruleset = write_ruleset().map_err(|e| e.to_string())?;
    for path in writable {
        let access = if *path == "/dev/null" {
            AccessFs::WriteFile | AccessFs::Truncate
        } else {
            AccessFs::from_write(ABI::V3)
        };
        ruleset = ruleset
            .add_rule(PathBeneath::new(
                PathFd::new(path).map_err(|e| e.to_string())?,
                access,
            ))
            .map_err(|e| e.to_string())?;
    }
    let status = ruleset.restrict_self().map_err(|e| e.to_string())?;
    if status.ruleset != RulesetStatus::FullyEnforced || !status.no_new_privs {
        return Err("Browser input protection is unavailable".into());
    }
    Ok(())
}

pub(super) fn restrict_mutations() -> Result<(), String> {
    use seccompiler::{
        BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter,
        SeccompRule, sock_filter,
    };
    // Landlock does not mediate chmod/chown/timestamps/xattrs on an inherited
    // descriptor. Deny those routes, filesystem ioctls, and io_uring bypasses.
    let calls = [
        libc::SYS_fchmod,
        libc::SYS_fchmodat,
        libc::SYS_fchmodat2,
        libc::SYS_fchown,
        libc::SYS_fchownat,
        libc::SYS_utimensat,
        libc::SYS_setxattr,
        libc::SYS_lsetxattr,
        libc::SYS_fsetxattr,
        libc::SYS_removexattr,
        libc::SYS_lremovexattr,
        libc::SYS_fremovexattr,
        libc::SYS_ioctl,
        libc::SYS_io_uring_setup,
        libc::SYS_io_uring_enter,
        libc::SYS_io_uring_register,
    ];
    let mut rules: std::collections::BTreeMap<_, _> =
        calls.into_iter().map(|call| (call, Vec::new())).collect();
    #[cfg(target_arch = "x86_64")]
    for call in [
        libc::SYS_chmod,
        libc::SYS_chown,
        libc::SYS_lchown,
        libc::SYS_utime,
        libc::SYS_utimes,
        libc::SYS_futimesat,
    ] {
        rules.insert(call, Vec::new());
    }
    // Rust/GIO use these descriptor controls when spawning loader processes.
    // They cannot change source contents or inode attributes.
    let descriptor_controls = [libc::FIOCLEX, libc::FIONCLEX, libc::FIONBIO, libc::FIONREAD]
        .into_iter()
        .map(|request| SeccompCondition::new(1, SeccompCmpArgLen::Dword, SeccompCmpOp::Ne, request))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    rules.insert(
        libc::SYS_ioctl,
        vec![SeccompRule::new(descriptor_controls).map_err(|error| error.to_string())?],
    );
    let filter: BpfProgram = SeccompFilter::new(
        rules,
        SeccompAction::Allow,
        SeccompAction::Errno(libc::ENOSYS as u32),
        std::env::consts::ARCH
            .try_into()
            .map_err(|e| format!("{e}"))?,
    )
    .map_err(|e| e.to_string())?
    .try_into()
    .map_err(|e: seccompiler::BackendError| e.to_string())?;
    // seccomp_data.nr is at byte zero. Reject numbers newer than Linux 6.6
    // (including x32) before the compiler's architecture/argument checks. This
    // avoids hundreds of redundant allow rules on every decoder syscall.
    let mut guarded = vec![
        sock_filter {
            code: (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16,
            jt: 0,
            jf: 0,
            k: 0,
        },
        sock_filter {
            code: (libc::BPF_JMP | libc::BPF_JGE | libc::BPF_K) as u16,
            jt: 0,
            jf: 1,
            k: 453,
        },
        sock_filter {
            code: (libc::BPF_RET | libc::BPF_K) as u16,
            jt: 0,
            jf: 0,
            k: libc::SECCOMP_RET_ERRNO | libc::ENOSYS as u32,
        },
    ];
    guarded.extend(filter);
    seccompiler::apply_filter(&guarded).map_err(|e| e.to_string())
}

fn isolate_job() -> io::Result<()> {
    use rustix::{
        mount::{MountFlags, mount},
        process::{Resource, Rlimit, setrlimit},
    };
    // A fresh PID namespace hides the supervisor and kills *all* decoder
    // descendants at job exit. Fresh writable mounts prevent cross-job state.
    let options = std::ffi::CString::new(format!(
        "size={},mode=1777",
        super::super::TEMPORARY_STORAGE_LIMIT_BYTES
    ))?;
    for path in ["/tmp", "/dev/shm"] {
        mount(
            "tmpfs",
            path,
            "tmpfs",
            MountFlags::NOSUID | MountFlags::NODEV,
            options.as_c_str(),
        )
        .map_err(|e| io::Error::other(format!("Private {path}: {e}")))?;
    }
    for (resource, value) in [
        (Resource::As, super::super::ADDRESS_SPACE_LIMIT_BYTES),
        (Resource::Cpu, 10),
        (Resource::Fsize, super::super::FILE_SIZE_LIMIT_BYTES),
        (Resource::Core, 0),
    ] {
        setrlimit(
            resource,
            Rlimit {
                current: Some(value),
                maximum: Some(value),
            },
        )
        .map_err(|e| io::Error::other(format!("Resource {resource:?}: {e}")))?;
    }
    Ok(())
}
