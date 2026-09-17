// SPDX-License-Identifier: MIT

use std::time::Duration;

use super::*;

#[test]
fn cache_work_progresses_while_every_render_thread_is_busy() {
    let count = crate::sandbox::browser::worker_limit();
    let (release, wait) = mpsc::channel();
    let wait = Arc::new(Mutex::new(wait));
    let (started, ready) = mpsc::channel();
    let jobs = (0..count)
        .map(|_| {
            let wait = wait.clone();
            let started = started.clone();
            render(move || {
                started.send(()).expect("admitted");
                wait.lock()
                    .expect("receiver")
                    .recv_timeout(Duration::from_secs(10))
                    .expect("release");
            })
        })
        .collect::<Vec<_>>();
    for _ in 0..count {
        ready
            .recv_timeout(Duration::from_secs(5))
            .expect("render started");
    }
    let result = futures_lite::future::block_on(futures_lite::future::race(cache(|| 7), async {
        async_io::Timer::after(Duration::from_secs(5)).await;
        Err("cache work waited for a renderer".to_owned())
    }));
    for _ in 0..count {
        release.send(()).expect("release renderer");
    }
    for job in jobs {
        futures_lite::future::block_on(job).expect("renderer finished");
    }
    assert_eq!(result.expect("independent cache work"), 7);
}

#[test]
fn a_panicking_task_does_not_retire_executor_threads() {
    for _ in 0..super::super::MAX_CACHE_READERS {
        assert!(
            futures_lite::future::block_on(cache(|| -> usize { panic!("failed cache task") }))
                .is_err()
        );
    }
    assert_eq!(
        futures_lite::future::block_on(cache(|| 9)).expect("executor survived"),
        9
    );
}
