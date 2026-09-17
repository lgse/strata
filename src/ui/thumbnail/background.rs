// SPDX-License-Identifier: MIT

use std::sync::{Arc, Mutex, OnceLock, mpsc};

use futures_channel::oneshot;

type Task = Box<dyn FnOnce() + Send>;
type Executor = OnceLock<Result<mpsc::SyncSender<Task>, String>>;

static CACHE: Executor = OnceLock::new();
static RENDER: Executor = OnceLock::new();

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
    executor: &'static Executor,
    threads: usize,
    name: &'static str,
    task: impl FnOnce() -> T + Send + 'static,
) -> impl Future<Output = Result<T, String>> {
    let sender = executor.get_or_init(|| {
        let (sender, receiver) = mpsc::sync_channel::<Task>(super::MAX_QUEUED_THUMBNAILS);
        let receiver = Arc::new(Mutex::new(receiver));
        for _ in 0..threads {
            let receiver = receiver.clone();
            std::thread::Builder::new()
                .name(name.into())
                .spawn(move || {
                    loop {
                        let task = receiver.lock().unwrap_or_else(|p| p.into_inner()).recv();
                        let Ok(task) = task else {
                            break;
                        };
                        // Like spawn_blocking, a panic cancels this completion, not the
                        // executor. Listing's GIO threads never wait on decoder jobs.
                        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task));
                    }
                })
                .map_err(|error| error.to_string())?;
        }
        Ok(sender)
    });
    let (send, receive) = oneshot::channel();
    let submitted = sender.as_ref().map_err(Clone::clone).and_then(|sender| {
        sender
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
