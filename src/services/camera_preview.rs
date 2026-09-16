// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    collections::HashMap,
    rc::{Rc, Weak},
    time::Duration,
};

use futures_channel::oneshot;
use gtk::{gio, glib, prelude::*};

use crate::model::Location;

// Allow GTK to bind the published rows and admit a visible preview wave first.
const BIND_GRACE: Duration = Duration::from_millis(200);
const PREVIEW_BUDGET: Duration = Duration::from_secs(2);

type Completion = RefCell<Vec<oneshot::Sender<()>>>;

thread_local! {
    static ACTIVE: RefCell<HashMap<String, Vec<Weak<Completion>>>> = RefCell::new(HashMap::new());
}

pub(crate) struct PreviewTurn {
    root: String,
    completion: Rc<Completion>,
}

impl Drop for PreviewTurn {
    fn drop(&mut self) {
        for waiter in self.completion.take() {
            let _ = waiter.send(());
        }
        ACTIVE.with(|active| {
            let mut active = active.borrow_mut();
            if let Some(previews) = active.get_mut(&self.root) {
                previews.retain(|preview| {
                    preview
                        .upgrade()
                        .is_some_and(|preview| !Rc::ptr_eq(&preview, &self.completion))
                });
                if previews.is_empty() {
                    active.remove(&self.root);
                }
            }
        });
    }
}

fn root_uri(uri: &str) -> String {
    let mut file = gio::File::for_uri(uri);
    while let Some(parent) = file.parent() {
        file = parent;
    }
    file.uri().to_string()
}

pub(crate) fn begin(uri: &str) -> PreviewTurn {
    let root = root_uri(uri);
    let completion = Rc::new(RefCell::new(Vec::new()));
    ACTIVE.with(|active| {
        active
            .borrow_mut()
            .entry(root.clone())
            .or_default()
            .push(Rc::downgrade(&completion));
    });
    PreviewTurn { root, completion }
}

pub(crate) async fn yield_after_batch(location: &Location) {
    let Some(uri) = location
        .uri_value()
        .filter(|_| location.backend_name() == "gphoto2")
    else {
        return;
    };
    glib::timeout_future(BIND_GRACE).await;
    wait_for_wave(&root_uri(uri), PREVIEW_BUDGET).await;
}

async fn wait_for_wave(root: &str, budget: Duration) {
    // Snapshot only this wave: scrolling or another window must not extend the
    // pause indefinitely. GVfs gphoto2 serializes enumeration and preview I/O.
    let waiters = ACTIVE.with(|active| {
        active
            .borrow()
            .get(root)
            .into_iter()
            .flatten()
            .filter_map(Weak::upgrade)
            .map(|preview| {
                let (sender, receiver) = oneshot::channel();
                preview.borrow_mut().push(sender);
                receiver
            })
            .collect::<Vec<_>>()
    });
    let _ = glib::future_with_timeout(budget, async move {
        for waiter in waiters {
            let _ = waiter.await;
        }
    })
    .await;
}

#[cfg(test)]
mod tests;
