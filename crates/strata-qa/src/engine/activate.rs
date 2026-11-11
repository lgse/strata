// SPDX-License-Identifier: MIT

use std::time::Instant;

use super::{LOCATE_TIMEOUT, POLL_INTERVAL, lookup::find_presented, scope::Scope};
use crate::{Aim, Backend, Target};

impl<B: Backend> Scope<'_, B> {
    /// Runs a node's default action. A target that need not be drawn, and a plain role and
    /// name, are looked up among the presented nodes; any other target has to be on screen.
    pub(super) async fn activate(
        &self,
        target: &Target,
        message: Option<&str>,
    ) -> Result<(), String> {
        let backend = self.backend;
        let waiting = || {
            message.map_or_else(String::new, |message| {
                format!("timed out waiting for {message}: ")
            })
        };
        let node = if let Target::Role {
            rendered: false, ..
        } = target
        {
            let deadline = Instant::now() + LOCATE_TIMEOUT;
            loop {
                if let Some(node) = self.presented(target)? {
                    break node;
                }
                if Instant::now() >= deadline {
                    return Err(format!(
                        "{}no {} presented after {} s",
                        waiting(),
                        target.to_json(),
                        LOCATE_TIMEOUT.as_secs()
                    ));
                }
                backend.sleep(POLL_INTERVAL).await;
            }
        } else if let Target::Role {
            role,
            other_roles,
            name: Some(name),
            description: None,
            within: None,
            name_ends_with,
            nth: None,
            states,
            name_matches: None,
            has: None,
            rendered: true,
        } = target
            && name_ends_with.is_empty()
            && states.is_empty()
            && other_roles.is_empty()
        {
            let deadline = Instant::now() + LOCATE_TIMEOUT;
            loop {
                if let Some(node) = find_presented(backend, &self.window, role, name) {
                    break node;
                }
                if Instant::now() >= deadline {
                    return Err(format!(
                        "{}no {role} named {name:?} after {} s",
                        waiting(),
                        LOCATE_TIMEOUT.as_secs()
                    ));
                }
                backend.sleep(POLL_INTERVAL).await;
            }
        } else {
            self.find(target, None, Aim::Point)
                .await
                .map_err(|error| match message {
                    Some(message) => format!("timed out waiting for {message}: {error}"),
                    None => error,
                })?
                .node
        };
        if !backend.activate(&node) {
            return Err(format!("{} has no default action", target.to_json()));
        }
        backend.settle(&self.window).await;
        Ok(())
    }
}
