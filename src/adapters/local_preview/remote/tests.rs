// SPDX-License-Identifier: MIT

use super::*;

fn main_context_lock() -> std::sync::MutexGuard<'static, ()> {
    crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("main context lock")
}

#[test]
fn streamed_limit_covers_empty_exact_oversized_and_multi_chunk_inputs() {
    for (length, limit, succeeds) in [
        (0, 0, true),
        (4, 4, true),
        (5, 4, false),
        (CHUNK_BYTES + 17, CHUNK_BYTES + 17, true),
        (CHUNK_BYTES + 17, CHUNK_BYTES + 16, false),
    ] {
        let input = vec![42; length];
        let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(input.clone()));
        let mut output = Vec::new();
        let result = copy_bounded(&stream, &mut output, limit as u64, &gio::Cancellable::new());
        assert_eq!(result.is_ok(), succeeds);
        assert!(output.len() <= limit);
        if succeeds {
            assert_eq!(output, input);
        }
    }
}

#[test]
fn cancellation_and_write_failure_stop_the_transfer() {
    let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_static(b"photo"));
    let cancellation = gio::Cancellable::new();
    let mut output = Vec::new();
    {
        let _guard = CancelOnDrop(cancellation.clone());
    }
    assert!(copy_bounded(&stream, &mut output, 100, &cancellation).is_err());
    assert!(output.is_empty());

    struct FullDisk;
    impl Write for FullDisk {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("disk full"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let error = copy_bounded(&stream, &mut FullDisk, 100, &gio::Cancellable::new())
        .expect_err("write failure");
    assert!(error.contains("disk full"));
}

#[test]
fn transfer_timeout_and_abort_retain_partial_files_until_worker_exit() {
    let _lock = main_context_lock();
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("main context owner");
    for timeout in [false, true] {
        let (started, receive_started) = futures_channel::oneshot::channel();
        let (finish, receive_finish) = std::sync::mpsc::channel();
        let (cancelled, receive_cancelled) = futures_channel::oneshot::channel();
        let (done, receive_done) = futures_channel::oneshot::channel();
        let duration = if timeout {
            Duration::from_millis(100)
        } else {
            Duration::from_secs(10)
        };
        let task = context.spawn_local(async move {
            let result = transfer(".jpg".into(), duration, move |file, cancellation| {
                file.write_all(b"partial").expect("partial input");
                started.send(file.path().to_owned()).expect("notify start");
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                while !cancellation.is_cancelled() && std::time::Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(2));
                }
                assert!(cancellation.is_cancelled());
                cancelled.send(()).expect("notify cancellation");
                receive_finish
                    .recv_timeout(Duration::from_secs(5))
                    .expect("release worker");
                Ok(())
            })
            .await;
            done.send(result.err()).ok();
        });
        context.block_on(async {
            let path = receive_started.await.expect("worker started");
            assert!(path.exists());
            if timeout {
                assert!(
                    receive_done
                        .await
                        .expect("timeout response")
                        .expect("transfer failed")
                        .contains("timed out")
                );
            } else {
                task.abort();
                drop(task);
            }
            receive_cancelled.await.expect("worker cancelled");
            assert!(
                path.exists(),
                "worker must own partial input until it exits"
            );
            assert_eq!(STAGED_PREVIEWS.load(Ordering::Acquire), 1);
            finish.send(()).expect("release worker");
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while STAGED_PREVIEWS.load(Ordering::Acquire) != 0
                && std::time::Instant::now() < deadline
            {
                glib::timeout_future(Duration::from_millis(2)).await;
            }
            assert_eq!(STAGED_PREVIEWS.load(Ordering::Acquire), 0);
            assert!(!path.exists());
        });
    }
}

#[test]
fn staged_transfer_enforces_stream_limit_cleanup_and_aggregate_slots() {
    let _lock = main_context_lock();
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("main context owner");
    context.block_on(async {
        let (send, receive) = futures_channel::oneshot::channel();
        let result = transfer(
            ".png".into(),
            Duration::from_secs(5),
            move |file, cancellation| {
                send.send(file.path().to_owned())
                    .expect("report staged path");
                let input = vec![1; CHUNK_BYTES + 1];
                let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(input));
                copy_bounded(&stream, file, CHUNK_BYTES as u64, cancellation)
            },
        )
        .await;
        assert!(
            result
                .err()
                .expect("size limit failure")
                .contains("download limit")
        );
        assert!(!receive.await.expect("staged path").exists());
        assert_eq!(STAGED_PREVIEWS.load(Ordering::Acquire), 0);

        let mut staged = Vec::new();
        for _ in 0..MAX_STAGED_PREVIEWS {
            staged.push(
                transfer(".jpg".into(), Duration::from_secs(5), |file, _| {
                    file.write_all(b"image").map_err(|error| error.to_string())
                })
                .await
                .expect("staged image"),
            );
        }
        let result = transfer(String::new(), Duration::from_secs(5), |_, _| {
            panic!("fifth transfer must not start")
        })
        .await;
        assert!(
            result
                .err()
                .expect("concurrency limit failure")
                .contains("Too many")
        );
        let paths: Vec<_> = staged.iter().map(|file| file.path().to_owned()).collect();
        drop(staged);
        assert!(paths.iter().all(|path| !path.exists()));
        assert!(StagingPermit::acquire().is_ok());
    });
}
