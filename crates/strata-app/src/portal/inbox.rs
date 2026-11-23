// SPDX-License-Identifier: MIT

use std::{
    cell::RefCell,
    collections::VecDeque,
    rc::Rc,
    sync::{Arc, atomic::AtomicBool},
};

use super::{ChooserRequest, Completion, PortalCallbacks};

/// One thing the service asks of the host.
pub(crate) enum PortalEvent {
    /// Open a chooser for the request, unless `cancelled` is set first.
    Present {
        request: Box<ChooserRequest>,
        cancelled: Arc<AtomicBool>,
        completion: Completion,
    },
    /// The caller closed the request with the token.
    Cancel(String),
    /// The service lost its bus name.
    Quit,
}

/// The queue between the service and the host, both on the interface thread.
#[derive(Clone, Default)]
pub(crate) struct PortalInbox(Rc<RefCell<VecDeque<PortalEvent>>>);

impl PortalInbox {
    /// The service's callbacks, which queue here and then call `wake` so the host looks.
    pub(crate) fn callbacks(&self, wake: Rc<dyn Fn()>) -> PortalCallbacks {
        let present = {
            let inbox = self.clone();
            let wake = wake.clone();
            Rc::new(move |request, cancelled, completion| {
                inbox.push(PortalEvent::Present {
                    request: Box::new(request),
                    cancelled,
                    completion,
                });
                wake();
            })
        };
        let cancel = {
            let inbox = self.clone();
            let wake = wake.clone();
            Rc::new(move |token: &str| {
                inbox.push(PortalEvent::Cancel(token.to_owned()));
                wake();
            })
        };
        let quit = {
            let inbox = self.clone();
            Rc::new(move || {
                inbox.push(PortalEvent::Quit);
                wake();
            })
        };
        PortalCallbacks {
            present,
            cancel,
            quit,
        }
    }

    fn push(&self, event: PortalEvent) {
        self.0.borrow_mut().push_back(event);
    }

    /// Everything queued so far, oldest first.
    pub(crate) fn take(&self) -> Vec<PortalEvent> {
        self.0.borrow_mut().drain(..).collect()
    }
}
