// SPDX-License-Identifier: MIT

use std::sync::{Arc, Mutex, OnceLock, mpsc};

use futures_channel::oneshot;

type Task = Box<dyn FnOnce() + Send>;
struct Executor {
    sender: mpsc::SyncSender<Task>,
    receiver: Arc<Mutex<mpsc::Receiver<Task>>>,
    threads: Mutex<usize>,
}

static CACHE: OnceLock<Executor> = OnceLock::new();
static RENDER: OnceLock<Executor> = OnceLock::new();

#[cfg(test)]
mod tests;

pub(super) fn cache<T: Send + 'static>(
    task: impl FnOnce() -> T + Send + 'static,
) -> impl Future<Output = Result<T, String>> {
    submit(&CACHE, super::MAX_CACHE_READERS, "thumbnail-cache", task)
}

pub(super) fn render<T: Send + 'static>(
    task: impl FnOnce() -> T + Send + 'static,
) -> impl Future<Output = Result<T, String>> {
    submit(
        &RENDER,
        crate::sandbox::browser::worker_limit(),
        "thumbnail-render",
        task,
    )
}

fn submit<T: Send + 'static>(
    executor: &'static OnceLock<Executor>,
    threads: usize,
    name: &'static str,
    task: impl FnOnce() -> T + Send + 'static,
) -> impl Future<Output = Result<T, String>> {
    let executor = executor.get_or_init(|| {
        let (sender, receiver) = mpsc::sync_channel::<Task>(super::MAX_QUEUED_THUMBNAILS);
        Executor {
            sender,
            receiver: Arc::new(Mutex::new(receiver)),
            threads: Mutex::new(0),
        }
    });
    let ready = (|| {
        let mut started = executor.threads.lock().unwrap_or_else(|p| p.into_inner());
        while *started < threads {
            let receiver = executor.receiver.clone();
            std::thread::Builder::new()
                .name(name.into())
                .spawn(move || {
                    loop {
                        let task = receiver.lock().unwrap_or_else(|p| p.into_inner()).recv();
                        let Ok(task) = task else {
                            break;
                        };
                        // A task panic must not permanently reduce executor capacity.
                        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task));
                    }
                })
                .map_err(|error| error.to_string())?;
            *started += 1;
        }
        Ok::<_, String>(())
    })();
    let (send, receive) = oneshot::channel();
    let submitted = ready.and_then(|()| {
        executor
            .sender
            .try_send(Box::new(move || {
                let _ = send.send(task());
            }))
            .map_err(|_| "Thumbnail executor is unavailable".to_owned())
    });
    async move {
        submitted?;
        receive
            .await
            .map_err(|_| "Thumbnail task failed".to_owned())
    }
}
