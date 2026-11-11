// SPDX-License-Identifier: MIT

//! One window and what earlier steps kept: where a step looks things up.

use std::{cell::RefCell, collections::HashMap, time::Instant};

use super::{
    LOCATE_TIMEOUT, POLL_INTERVAL,
    fields::field,
    locate::{Kept, Located, focused, locate, presented_target},
    scan::{Scene, scan},
};
use crate::{Aim, At, Backend, Boxes, FieldRef, LookupError, Rect, Target};

/// The nodes `remember` keeps under a name.
pub type Memory<B> = HashMap<String, <B as Backend>::Kept>;

/// Everything steps keep for later ones.
pub struct Remembered<B: Backend> {
    /// Text fields, which `field` targets and checks name.
    pub fields: RefCell<Memory<B>>,
    pub nodes: RefCell<Memory<B>>,
    pub boxes: RefCell<Boxes>,
}

impl<B: Backend> Default for Remembered<B> {
    fn default() -> Self {
        Self {
            fields: RefCell::default(),
            nodes: RefCell::default(),
            boxes: RefCell::default(),
        }
    }
}

pub struct Scope<'a, B: Backend> {
    pub backend: &'a B,
    pub window: B::Node,
    pub kept: &'a Remembered<B>,
}

impl<B: Backend> Scope<'_, B> {
    pub fn scene(&self) -> Scene<B::Node> {
        scan(self.backend, &self.window)
    }

    /// Where the target is right now, without waiting for it.
    pub fn locate_now(
        &self,
        target: &Target,
        at: Option<&At>,
        aim: Aim,
    ) -> Result<Located<B::Node>, LookupError> {
        locate(
            self.backend,
            &self.window,
            Kept {
                fields: &self.kept.fields.borrow(),
                nodes: &self.kept.nodes.borrow(),
                boxes: &self.kept.boxes.borrow(),
            },
            target,
            at,
            aim,
        )
    }

    pub fn presented(&self, target: &Target) -> Result<Option<B::Node>, String> {
        presented_target(
            self.backend,
            &self.window,
            Kept {
                fields: &self.kept.fields.borrow(),
                nodes: &self.kept.nodes.borrow(),
                boxes: &self.kept.boxes.borrow(),
            },
            target,
        )
        .map_err(|error| error.message().to_owned())
    }

    /// The target's node, or `None` when it is not there; a role target that need not be
    /// drawn is looked up among the presented nodes.
    pub fn resolve(&self, target: &Target) -> Result<Option<B::Node>, String> {
        if matches!(
            target,
            Target::Role {
                rendered: false,
                ..
            }
        ) {
            return self.presented(target);
        }
        match self.locate_now(target, None, Aim::Point) {
            Ok(located) => Ok(Some(located.node)),
            Err(LookupError::Missing(_)) => Ok(None),
            Err(LookupError::Broken(error)) => Err(error),
        }
    }

    pub fn field(&self, field_ref: &FieldRef) -> Result<Option<B::Node>, String> {
        field(
            self.backend,
            &self.window,
            &self.kept.fields.borrow(),
            field_ref,
        )
        .map_err(|error| match error {
            LookupError::Missing(message) | LookupError::Broken(message) => message,
        })
    }

    /// A row a listing just added can report a box before its first allocation lands, so the
    /// box must come through a painted frame unchanged.
    pub async fn find(
        &self,
        target: &Target,
        at: Option<&At>,
        aim: Aim,
    ) -> Result<Located<B::Node>, String> {
        let deadline = Instant::now() + LOCATE_TIMEOUT;
        loop {
            match self.locate_now(target, at, aim) {
                Ok(located) => {
                    self.backend.settle(&self.window).await;
                    match self.locate_now(target, at, aim) {
                        Ok(painted) if painted.rect.pixels() == located.rect.pixels() => {
                            return Ok(painted);
                        }
                        Err(LookupError::Broken(error)) => return Err(error),
                        _ if Instant::now() >= deadline => {
                            return Err(format!(
                                "the target kept moving for {} s",
                                LOCATE_TIMEOUT.as_secs()
                            ));
                        }
                        _ => {}
                    }
                }
                Err(LookupError::Broken(error)) => return Err(error),
                Err(LookupError::Missing(error)) if Instant::now() >= deadline => {
                    return Err(format!("{error} after {} s", LOCATE_TIMEOUT.as_secs()));
                }
                Err(LookupError::Missing(_)) => self.backend.sleep(POLL_INTERVAL).await,
            }
        }
    }

    /// The first drawn node, in tree order, that reports itself focused.
    pub fn focused(&self) -> Option<B::Node> {
        focused(self.backend, &self.window)
    }

    /// A box `settle`, `remember` or `keep` kept under the name.
    pub fn kept_box(&self, name: &str) -> Result<Rect, String> {
        self.kept
            .boxes
            .borrow()
            .get(name)
            .copied()
            .ok_or_else(|| format!("no box was kept as {name:?}"))
    }
}
