// SPDX-License-Identifier: MIT
use super::{Manifest, PATH_LIMIT, slug};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, io, path::Path};

pub(super) const FRAME_LIMIT: usize = 1024 * 1024;
const NODE_LIMIT: usize = 64;
const DEPTH_LIMIT: usize = 4;

fn path_frame_size(path: &str) -> usize {
    3 + path
        .bytes()
        .map(|byte| match byte {
            b'"' | b'\\' | b'\n' | b'\r' | b'\t' | 8 | 12 => 2,
            0..=31 => 6,
            _ => 1,
        })
        .sum::<usize>()
}

pub(crate) fn selection_fits(paths: &[String]) -> bool {
    // Reserve space for the envelope and the largest escaped leaf context.
    paths
        .iter()
        .map(|path| path_frame_size(path))
        .sum::<usize>()
        < FRAME_LIMIT - 16384
}

pub(crate) fn query_batch(paths: Vec<String>) -> Vec<String> {
    let mut size = 0;
    paths
        .into_iter()
        .take(PATH_LIMIT)
        .take_while(|path| {
            size += path_frame_size(path);
            size < FRAME_LIMIT - 16384
        })
        .collect()
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Request {
    pub version: u32,
    pub id: u64,
    pub method: String,
    pub paths: Vec<String>,
    pub background: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

pub(super) fn native_path(path: &str) -> bool {
    path.len() <= 16384 && Path::new(path).is_absolute() && !path.contains('\0')
}

impl Request {
    pub(super) fn valid(&self) -> bool {
        self.version == 1
            && !self.paths.is_empty()
            && self.paths.len() <= PATH_LIMIT
            && self.paths.iter().all(|p| native_path(p))
            && (!self.background || self.paths.len() == 1)
            && match self.method.as_str() {
                "query" | "menu" => self.action.is_none() && self.context.is_none(),
                "activate" => {
                    self.action.as_ref().is_some_and(|a| slug(a))
                        && self.context.as_ref().is_none_or(|c| c.len() <= 2048)
                }
                _ => false,
            }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct Decoration {
    pub path: String,
    pub badge: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub priority: u8,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct MenuAction {
    pub id: String,
    pub label: String,
    pub icon: Option<String>,
    pub context: Option<String>,
    pub children: Option<Vec<MenuAction>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum OutcomeStatus {
    Accepted,
    Rejected,
    Partial,
    Unknown,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Outcome {
    pub status: OutcomeStatus,
    pub accepted: Option<usize>,
    pub total: Option<usize>,
    pub job: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub(crate) struct Reply {
    pub version: u32,
    pub id: Option<u64>,
    pub event: Option<String>,
    pub paths: Option<Vec<String>>,
    pub revision: Option<u64>,
    pub decorations: Option<Vec<Decoration>>,
    pub actions: Option<Vec<MenuAction>>,
    #[serde(default)]
    pub message: String,
    pub outcome: Option<Outcome>,
    pub error: Option<String>,
}

impl Reply {
    pub(super) fn matches(&self, request: &Request) -> bool {
        if self.error.is_some() {
            return self.decorations.is_none() && self.actions.is_none() && self.outcome.is_none();
        }
        match request.method.as_str() {
            "query" => {
                self.decorations.is_some()
                    && self.actions.as_ref().is_none_or(Vec::is_empty)
                    && self.outcome.is_none()
            }
            "menu" => {
                self.actions.is_some()
                    && self.decorations.as_ref().is_none_or(Vec::is_empty)
                    && self.outcome.is_none()
            }
            "activate" => {
                !self.message.is_empty()
                    && self.decorations.as_ref().is_none_or(Vec::is_empty)
                    && self.actions.as_ref().is_none_or(Vec::is_empty)
                    && self
                        .outcome
                        .as_ref()
                        .is_none_or(|o| o.total.is_none_or(|total| total == request.paths.len()))
            }
            _ => false,
        }
    }
}

fn menu_valid(actions: &[MenuAction], manifest: &Manifest) -> bool {
    fn visit(
        actions: &[MenuAction],
        depth: usize,
        count: &mut usize,
        ids: &mut HashSet<String>,
        manifest: &Manifest,
    ) -> bool {
        if depth > DEPTH_LIMIT {
            return false;
        }
        for a in actions {
            *count += 1;
            if *count > NODE_LIMIT
                || !slug(&a.id)
                || !ids.insert(a.id.clone())
                || a.label.is_empty()
                || a.label.len() > 128
                || a.label.chars().any(char::is_control)
                || a.icon
                    .as_ref()
                    .is_some_and(|i| !manifest.icons.contains_key(i))
                || a.context
                    .as_ref()
                    .is_some_and(|c| c.is_empty() || c.len() > 2048)
            {
                return false;
            }
            if let Some(children) = &a.children
                && (children.is_empty()
                    || a.context.is_some()
                    || !visit(children, depth + 1, count, ids, manifest))
            {
                return false;
            }
        }
        true
    }
    visit(actions, 1, &mut 0, &mut HashSet::new(), manifest)
}

fn plain_text(text: &str) -> bool {
    !text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
}

pub(super) fn checked_reply(bytes: &[u8], manifest: &Manifest) -> io::Result<Reply> {
    let r: Reply = serde_json::from_slice(bytes).map_err(|_| io::Error::other("invalid JSON"))?;
    let invalid_outcome = r.outcome.as_ref().is_some_and(|o| {
        o.accepted.is_some() != o.total.is_some()
            || o.total
                .is_some_and(|total| total == 0 || total > PATH_LIMIT)
            || o.accepted.zip(o.total).is_some_and(|(accepted, total)| {
                accepted > total
                    || match o.status {
                        OutcomeStatus::Accepted => accepted != total,
                        OutcomeStatus::Rejected => accepted != 0,
                        OutcomeStatus::Partial => accepted == 0 || accepted == total,
                        OutcomeStatus::Unknown => false,
                    }
            })
            || o.job
                .as_ref()
                .is_some_and(|j| j.is_empty() || j.len() > 128 || j.chars().any(char::is_control))
    });
    if r.version != 1
        || (r.id.is_some() == r.event.is_some())
        || r.message.len() > 16384
        || !plain_text(&r.message)
        || r.error.as_ref().is_some_and(|e| !slug(e))
        || invalid_outcome
        || r.decorations.as_ref().is_some_and(|ds| {
            ds.len() > PATH_LIMIT
                || ds.iter().any(|d| {
                    !native_path(&d.path)
                        || d.description.len() > 512
                        || !plain_text(&d.description)
                        || d.priority > 2
                        || d.badge
                            .as_ref()
                            .is_some_and(|i| !manifest.icons.contains_key(i))
                })
        })
        || r.actions.as_ref().is_some_and(|a| !menu_valid(a, manifest))
        || r.paths.as_ref().is_some_and(|paths| {
            paths.is_empty() || paths.len() > PATH_LIMIT || !paths.iter().all(|p| native_path(p))
        })
        || (r.event.is_some()
            && (r.event.as_deref() != Some("invalidate")
                || r.decorations.is_some()
                || r.actions.is_some()
                || r.outcome.is_some()
                || r.error.is_some()
                || !r.message.is_empty()))
        || (r.id.is_some() && r.paths.is_some())
    {
        return Err(io::Error::other("invalid response fields"));
    }
    Ok(r)
}
