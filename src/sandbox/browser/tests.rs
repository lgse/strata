// SPDX-License-Identifier: MIT

use std::{
    io::{Read, Write},
    os::fd::AsFd,
};

use super::*;

#[test]
fn descriptor_transport_preserves_the_open_file_not_its_replacement() {
    let directory = tempfile::tempdir().expect("fixture");
    let path = directory.path().join("input");
    std::fs::write(&path, b"original").expect("input");
    let input = File::open(&path).expect("open");
    let (parent, child) = UnixStream::pair().expect("socket");
    let (read, write) =
        rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).expect("reply pipe");
    wire::send(&parent, &input, &write, Operation::Image).expect("send");
    drop(write);
    std::fs::remove_file(&path).expect("unlink");
    std::fs::write(&path, b"replacement").expect("replacement");
    let (operation, fd, output) = wire::receive(&child).expect("receive").expect("request");
    assert_eq!(operation, Operation::Image);
    assert!(
        rustix::io::fcntl_getfd(fd.as_fd())
            .expect("flags")
            .contains(rustix::io::FdFlags::CLOEXEC)
    );
    let mut value = String::new();
    File::from(fd).read_to_string(&mut value).expect("read");
    assert_eq!(value, "original");
    assert_eq!(
        rustix::io::read(&output, &mut [0]),
        Err(rustix::io::Errno::BADF)
    );
    File::from(output).write_all(b"reply").expect("reply");
    let mut reply = String::new();
    File::from(read)
        .read_to_string(&mut reply)
        .expect("one-way reply");
    assert_eq!(reply, "reply");
    drop(parent);
    assert!(wire::receive(&child).expect("EOF").is_none());
}

#[test]
fn descriptor_transport_rejects_missing_descriptors_and_unknown_operations() {
    for operation in [1, 255] {
        let (mut parent, child) = UnixStream::pair().expect("socket");
        parent.write_all(&[operation]).expect("send");
        assert!(wire::receive(&child).is_err());
    }
    assert!(Operation::parse(0).is_err());
}

#[test]
fn response_transport_bounds_allocations_and_rejects_truncation() {
    let response = Response {
        png: b"thumbnail".to_vec(),
        metadata: b"details".to_vec(),
    };
    let mut bytes = Vec::new();
    response.write(&mut bytes).expect("encode");
    for end in 0..bytes.len() {
        assert!(Response::read(&mut &bytes[..end]).is_err());
    }
    let decoded = Response::read(&mut bytes.as_slice()).expect("decode");
    assert_eq!(decoded.png, response.png);
    assert_eq!(decoded.metadata, response.metadata);
    assert!(Response::read(&mut [255; 8].as_slice()).is_err());
}

#[test]
fn reply_reads_obey_the_absolute_deadline_even_with_a_live_writer() {
    let (read, write) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).expect("pipe");
    let mut read = File::from(read);
    let mut write = File::from(write);
    let mut reader = DeadlineReader {
        reader: &mut read,
        deadline: Instant::now() + Duration::from_millis(5),
    };
    assert_eq!(
        reader
            .read(&mut [0])
            .expect_err("idle writer times out")
            .kind(),
        io::ErrorKind::TimedOut
    );
    write.write_all(b"late").expect("late bytes");
    assert_eq!(
        reader
            .read(&mut [0])
            .expect_err("late data cannot renew the deadline")
            .kind(),
        io::ErrorKind::TimedOut
    );
}

#[test]
fn source_admission_rejects_special_files_and_follows_regular_symlinks() {
    let directory = tempfile::tempdir().expect("fixture");
    let fifo = directory.path().join("fifo");
    rustix::fs::mknodat(
        rustix::fs::CWD,
        &fifo,
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::RUSR,
        0,
    )
    .expect("FIFO");
    assert!(open_source(&fifo).is_err());
    assert!(open_source(directory.path()).is_err());
    assert!(open_source(Path::new("/dev/null")).is_err());
    let path = directory.path().join("regular");
    std::fs::write(&path, b"source").expect("source");
    let link = directory.path().join("link");
    std::os::unix::fs::symlink(&path, &link).expect("link");
    let mut value = String::new();
    open_source(&link)
        .expect("regular symlink")
        .read_to_string(&mut value)
        .expect("read");
    assert_eq!(value, "source");
}

#[test]
fn file_versions_invalidate_cached_work_after_replacement() {
    let directory = tempfile::tempdir().expect("fixture");
    let path = directory.path().join("file");
    std::fs::write(&path, b"one").expect("source");
    let first = FileKey::read(&path, &File::open(&path).expect("open")).expect("version");
    let gate = cache_entry(first.clone());
    assert!(Arc::ptr_eq(&gate, &cache_entry(first)));
    std::fs::rename(&path, directory.path().join("old")).expect("move");
    std::fs::write(&path, b"two").expect("replacement");
    let second = FileKey::read(&path, &File::open(&path).expect("open")).expect("version");
    assert!(!Arc::ptr_eq(&gate, &cache_entry(second)));
}

#[test]
fn cancelled_admission_never_starts_a_worker_or_strands_a_waiter() {
    let pool = Pool {
        state: Mutex::default(),
        changed: Condvar::new(),
        limit: 2,
        idle_timeout: DEFAULT_WORKER_IDLE_TIMEOUT,
    };
    let cancellation = Cancellation::default();
    cancellation.cancel();
    assert!(
        metadata(
            Path::new("/never-open-a-cancelled-input"),
            true,
            &cancellation
        )
        .is_err()
    );
    for operation in [Operation::Image, Operation::MediaMetadata] {
        assert!(pool.acquire(operation, &cancellation).is_err());
        let state = pool.state.lock().expect("pool");
        assert_eq!(state.count, 0);
        assert_eq!(state.thumbnail_waiters, 0);
        assert_eq!(state.slow_running, 0);
    }
}

#[test]
fn slow_work_preserves_capacity_for_visible_images_and_releases_its_permit() {
    let pool = Pool {
        state: Mutex::new(PoolState {
            count: 2,
            idle: (0..2)
                .map(|_| IdleWorker {
                    worker: Worker::OneShot,
                    since: Instant::now(),
                })
                .collect(),
            ..Default::default()
        }),
        changed: Condvar::new(),
        limit: 2,
        idle_timeout: DEFAULT_WORKER_IDLE_TIMEOUT,
    };
    let cancellation = Cancellation::default();
    let slow = pool
        .acquire(Operation::MediaMetadata, &cancellation)
        .expect("slow work");
    let image = pool
        .acquire(Operation::Image, &cancellation)
        .expect("image admission");
    assert_eq!(pool.state.lock().expect("pool").slow_running, 1);
    assert!(pool.state.lock().expect("pool").idle.is_empty());
    drop(image);
    drop(slow);
    assert_eq!(pool.state.lock().expect("pool").slow_running, 0);
    assert_eq!(pool.state.lock().expect("pool").idle.len(), 2);
}

#[test]
fn idle_expiry_releases_only_expired_workers_and_can_empty_the_pool() {
    let now = Instant::now();
    let timeout = configured_idle_timeout(Some("10"));
    let pool = Pool {
        state: Mutex::new(PoolState {
            count: 2,
            idle: [now, now + Duration::from_secs(5)]
                .into_iter()
                .map(|since| IdleWorker {
                    worker: Worker::OneShot,
                    since,
                })
                .collect(),
            ..Default::default()
        }),
        changed: Condvar::new(),
        limit: 2,
        idle_timeout: timeout,
    };
    assert_eq!(pool.next_expiration(now), Some(timeout));
    assert_eq!(pool.retire_idle(now + Duration::from_secs(9)), 0);
    assert_eq!(pool.retire_idle(now + timeout), 1);
    assert_eq!(pool.state.lock().expect("pool").count, 1);
    assert_eq!(
        pool.next_expiration(now + timeout),
        Some(Duration::from_secs(5))
    );
    assert_eq!(pool.retire_idle(now + Duration::from_secs(15)), 1);
    assert_eq!(pool.state.lock().expect("pool").count, 0);
    assert!(
        pool.next_expiration(now + Duration::from_secs(15))
            .is_none()
    );
}

#[test]
fn idle_expiry_never_interrupts_a_lease_and_returning_it_resets_the_deadline() {
    let timeout = Duration::from_secs(10);
    let now = Instant::now();
    let pool = Pool {
        state: Mutex::new(PoolState {
            count: 1,
            idle: vec![IdleWorker {
                worker: Worker::OneShot,
                since: now - timeout * 2,
            }],
            ..Default::default()
        }),
        changed: Condvar::new(),
        limit: 1,
        idle_timeout: timeout,
    };
    let lease = pool
        .acquire(Operation::MediaMetadata, &Cancellation::default())
        .expect("reused lease");
    assert_eq!(pool.retire_idle(now), 0);
    assert!(pool.next_expiration(now).is_none());
    assert_eq!(pool.state.lock().expect("pool").count, 1);
    assert_eq!(pool.state.lock().expect("pool").slow_running, 1);
    drop(lease);
    assert_eq!(
        pool.retire_idle(now),
        0,
        "returning a worker renews its idle lifetime"
    );
    assert_eq!(pool.state.lock().expect("pool").slow_running, 0);
    assert_eq!(pool.retire_idle(Instant::now() + timeout), 1);
    assert_eq!(pool.state.lock().expect("pool").count, 0);
}

#[test]
fn saturated_pool_wait_is_cancellable() {
    let pool = Pool {
        state: Mutex::new(PoolState {
            count: 2,
            ..Default::default()
        }),
        changed: Condvar::new(),
        limit: 2,
        idle_timeout: DEFAULT_WORKER_IDLE_TIMEOUT,
    };
    let cancellation = Cancellation::default();
    std::thread::scope(|scope| {
        let handle = scope.spawn(|| pool.acquire(Operation::Image, &cancellation).is_err());
        loop {
            if pool.state.lock().expect("pool").thumbnail_waiters == 1 {
                break;
            }
            std::thread::yield_now();
        }
        cancellation.cancel();
        pool.changed.notify_all();
        assert!(handle.join().expect("admission"));
    });
    assert_eq!(pool.state.lock().expect("pool").thumbnail_waiters, 0);
}

#[test]
fn decoder_cannot_reopen_or_mutate_a_read_only_source_descriptor() {
    const CHILD: &str = "STRATA_BROWSER_PROTECTION_TEST";
    const SOURCE: &str = "P3\n2 2\n255\n0 255 0 0 255 0 0 255 0 0 255 0\n";
    if std::env::var_os(CHILD).is_some() {
        use std::os::unix::fs::PermissionsExt;
        let writable = std::env::temp_dir();
        worker::restrict_mutations().expect("inherited mutation filter");
        worker::protect_input(&[writable.to_str().expect("scratch path"), "/dev/null"])
            .expect("input protections");
        let file = File::from(rustix::io::dup(std::io::stdin()).expect("source fd"));
        assert_eq!(
            rustix::fs::ioctl_getflags(&file),
            Err(rustix::io::Errno::NOSYS)
        );
        assert!(
            file.set_permissions(std::fs::Permissions::from_mode(0o777))
                .is_err()
        );
        assert!(
            file.set_modified(std::time::SystemTime::UNIX_EPOCH)
                .is_err()
        );
        for truncate in [false, true] {
            assert!(
                std::fs::OpenOptions::new()
                    .write(true)
                    .truncate(truncate)
                    .open("/proc/self/fd/0")
                    .is_err()
            );
        }
        let source = std::fs::read_link("/proc/self/fd/0").expect("source location");
        assert!(std::fs::remove_file(&source).is_err());
        assert!(std::os::unix::fs::symlink(&source, source.with_extension("escape")).is_err());
        let mut contents = String::new();
        (&file)
            .read_to_string(&mut contents)
            .expect("source remains readable");
        assert_eq!(contents, SOURCE);
        std::fs::write(writable.join("output"), b"decoder output")
            .expect("private writes permitted");
        // libtest applies the policy on its test thread, not the process leader.
        let input = PathBuf::from("/proc")
            .join(std::fs::read_link("/proc/thread-self").expect("decoder thread"))
            .join("fd/0");
        let mut response = crate::sandbox_helper::browser_render(&input, Operation::Image);
        assert!(super::super::valid_output(
            ParseOperation::ThumbnailImage,
            &response.png
        ));
        if response.metadata.is_empty() {
            response = crate::sandbox_helper::browser_render(&input, Operation::ImageMetadata);
        }
        assert_eq!(
            MediaMetadata::from_json(&response.metadata, true)
                .expect("source details")
                .dimensions,
            Some((2, 2))
        );
        return;
    }
    if !worker::supported() {
        assert!(matches!(
            Worker::spawn().expect("legacy worker"),
            Worker::OneShot
        ));
        return;
    }
    let source = tempfile::NamedTempFile::new().expect("source");
    let scratch = tempfile::tempdir().expect("decoder scratch");
    std::fs::write(source.path(), SOURCE).expect("fixture");
    let before = source.as_file().metadata().expect("metadata");
    let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .args(["--exact", "sandbox::browser::tests::decoder_cannot_reopen_or_mutate_a_read_only_source_descriptor", "--nocapture"])
        .env(CHILD, "1").env("TMPDIR", scratch.path()).env("PATH", "/usr/bin")
        .env("HOME", "/nonexistent").env("XDG_CACHE_HOME", scratch.path().join("cache"))
        .env_remove("DISPLAY").env_remove("WAYLAND_DISPLAY").env_remove("DBUS_SESSION_BUS_ADDRESS")
        .env_remove("XDG_RUNTIME_DIR")
        .stdin(Stdio::from(File::open(source.path()).expect("read-only input")))
        .output().expect("isolated protection test");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let after = source.as_file().metadata().expect("metadata");
    assert_eq!(before.mode(), after.mode());
    assert_eq!(before.mtime(), after.mtime());
    assert_eq!(
        std::fs::read(source.path()).expect("source"),
        SOURCE.as_bytes()
    );
}

#[test]
fn supervisor_fork_refuses_a_multithreaded_caller() {
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            barrier.wait();
            barrier.wait();
        });
        barrier.wait();
        let result = process::fork();
        barrier.wait();
        assert!(result.is_err());
    });
}

#[test]
fn worker_configuration_is_bounded_and_invalid_values_use_the_default() {
    for (input, expected) in [
        (None, 3),
        (Some("bad"), 3),
        (Some("0"), 1),
        (Some("2"), 2),
        (Some("1000"), 16),
    ] {
        assert_eq!(configured_limit(input, 3), expected);
    }
}
