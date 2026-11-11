//! ICCCM 2.6.2 `MULTIPLE`: one request for several targets at once, each into its own
//! property. The requestor names the pairs (target, property) in an `ATOM_PAIR` property;
//! the owner writes each target into its property, sets a pair's property to `None` where it
//! refuses that target, and answers with one `SelectionNotify`. Clipboard managers take a
//! selection this way when its owner hands it over (`SAVE_TARGETS`, see `handoff`).
//!
//! A pair's data larger than one chunk goes by INCR, each on its own property, as a single
//! request's does.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use x11rb::NONE;
use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt as _, PropMode};
use x11rb::wrapper::ConnectionExt as _;

use super::super::data::{SourceSpec, WRITE_IDLE};
use super::super::event::{Event, SourceId};
use super::selection::Request;
use super::{Actor, Cmd};

/// What one pair gets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Answer {
    /// Refused: the pair's property becomes `None` in the list.
    Refused,
    /// 32-bit values of this type (`TARGETS`, `TIMESTAMP`).
    Words(Atom, Vec<u32>),
    /// The bytes of a type, as its writer made them.
    Bytes(Arc<[u8]>),
}

struct Pair {
    target: Atom,
    property: Atom,
    answer: Option<Answer>,
}

/// A `MULTIPLE` request waiting for its lazy writers, until its deadline.
pub(super) struct Multi {
    key: u64,
    req: Request,
    pairs: Vec<Pair>,
    deadline: Instant,
}

impl Multi {
    pub(super) fn deadline(&self) -> Instant {
        self.deadline
    }
}

/// The most pairs one `MULTIPLE` request may name. A request can carry up to 32k, and each
/// distinct target costs a round trip for its name; more is refused whole.
pub(super) const MAX_PAIRS: usize = 1024;

/// How long a `MULTIPLE` request waits for its lazy writers: then the pairs still waiting are
/// refused and the rest answered, as long as a source write may wait for its reader.
const WAIT: Duration = WRITE_IDLE;

/// The targets the pairs ask for, each once, in the order first asked.
pub(super) fn distinct(pairs: &[(Atom, Atom)]) -> Vec<Atom> {
    let mut seen = Vec::new();
    for &(t, _) in pairs {
        if !seen.contains(&t) {
            seen.push(t);
        }
    }
    seen
}

/// The pairs of an `ATOM_PAIR` list, or None if it isn't one (an odd count).
pub(super) fn parse_pairs(words: &[u32]) -> Option<Vec<(Atom, Atom)>> {
    let (pairs, rest) = words.as_chunks::<2>();
    rest.is_empty()
        .then(|| pairs.iter().map(|[t, p]| (*t, *p)).collect())
}

/// The list written back: each pair as asked, with the property of a refused one `None`.
pub(super) fn answered_pairs(pairs: &[(Atom, Atom)], refused: &[bool]) -> Vec<u32> {
    pairs
        .iter()
        .zip(refused)
        .flat_map(|(&(t, p), &no)| [t, if no { NONE } else { p }])
        .collect()
}

impl Actor {
    /// A `MULTIPLE` request for a selection we hold as `id` with `spec`.
    pub(super) fn multiple(&mut self, req: Request, id: SourceId, spec: SourceSpec, time: u32) {
        // An obsolete requestor names no property, and MULTIPLE can't work without one. A
        // property named MULTIPLE is fine: gsd-clipboard and xfsettingsd ask into it.
        if req.obsolete {
            return self.notify(&req, NONE);
        }
        let words = self
            .conn
            .get_property(
                false,
                req.requestor,
                req.property,
                AtomEnum::ANY,
                0,
                1 << 16,
            )
            .ok()
            .and_then(|c| c.reply().ok())
            .filter(|r| r.format == 32)
            .and_then(|r| r.value32().map(Iterator::collect::<Vec<u32>>));
        let Some(asked) = words.as_deref().and_then(parse_pairs) else {
            return self.notify(&req, NONE);
        };
        if asked.len() > MAX_PAIRS {
            return self.notify(&req, NONE);
        }
        // Each target once, however often it is asked: one name lookup, one writer run.
        let mut answers: HashMap<Atom, Option<Answer>> = HashMap::new();
        let mut lazy = Vec::new();
        for target in distinct(&asked) {
            let answer = if target == self.atoms.MULTIPLE {
                Some(Answer::Refused)
            } else if target == self.atoms.TARGETS {
                let mut list = vec![
                    self.atoms.TARGETS,
                    self.atoms.TIMESTAMP,
                    self.atoms.MULTIPLE,
                ];
                let mimes: Vec<String> = spec.mimes().map(str::to_owned).collect();
                list.extend(mimes.iter().filter_map(|m| self.atom(m)));
                Some(Answer::Words(self.atoms.ATOM, list))
            } else if target == self.atoms.TIMESTAMP {
                Some(Answer::Words(self.atoms.INTEGER, vec![time]))
            } else {
                match self
                    .name(target)
                    .and_then(|m| spec.writer(&m).cloned().map(|w| (m, w)))
                {
                    None => Some(Answer::Refused),
                    Some((mime, writer)) => {
                        self.events.push(Event::SourceAsked { source: id, mime });
                        match writer.ready() {
                            Some(Some(bytes)) => Some(Answer::Bytes(bytes)),
                            Some(None) => Some(Answer::Refused),
                            None => {
                                lazy.push((target, writer));
                                None
                            }
                        }
                    }
                }
            };
            answers.insert(target, answer);
        }
        let pairs = asked
            .iter()
            .map(|&(target, property)| Pair {
                target,
                property,
                answer: if property == NONE {
                    Some(Answer::Refused)
                } else {
                    answers.get(&target).cloned().flatten()
                },
            })
            .collect();
        let key = self.next_multi;
        self.next_multi += 1;
        self.multis.push(Multi {
            key,
            req,
            pairs,
            deadline: Instant::now() + WAIT,
        });
        // One worker per distinct lazy type, however many pairs ask for it (so at most the
        // copy's lazy types): a writer that never returns holds its own worker only, the
        // others answer, and the request is answered by its deadline.
        for (target, writer) in lazy {
            let mailbox = self.mailbox.clone();
            let spawned = std::thread::Builder::new()
                .name("transfer-write".into())
                .spawn(move || {
                    let bytes = writer.get();
                    let _ = mailbox.post(Cmd::MultiReady { key, target, bytes });
                });
            if spawned.is_err() {
                self.multi_ready(key, target, None);
            }
        }
        self.settle_if_done(key);
    }

    /// A writer of `MULTIPLE` request `key` finished: every pair asking for `target` has its
    /// answer.
    pub(super) fn multi_ready(&mut self, key: u64, target: Atom, bytes: Option<Arc<[u8]>>) {
        let Some(m) = self.multis.iter_mut().find(|m| m.key == key) else {
            return;
        };
        let answer = bytes.map_or(Answer::Refused, Answer::Bytes);
        for pair in m.pairs.iter_mut().filter(|p| p.target == target) {
            pair.answer.get_or_insert_with(|| answer.clone());
        }
        self.settle_if_done(key);
    }

    /// The `MULTIPLE` requests whose deadline passed: their pairs still waiting are refused,
    /// and they are answered.
    pub(super) fn expire_multis(&mut self, now: Instant) {
        let late: Vec<u64> = self
            .multis
            .iter()
            .filter(|m| m.deadline <= now)
            .map(|m| m.key)
            .collect();
        for key in late {
            self.refuse_waiting(key);
        }
    }

    fn refuse_waiting(&mut self, key: u64) {
        if let Some(m) = self.multis.iter_mut().find(|m| m.key == key) {
            for pair in &mut m.pairs {
                pair.answer.get_or_insert(Answer::Refused);
            }
        }
        self.settle_if_done(key);
    }

    /// Answers request `key` once every pair has its answer.
    fn settle_if_done(&mut self, key: u64) {
        let Some(m) = self.multis.iter().position(|m| m.key == key) else {
            return;
        };
        if self.multis[m].pairs.iter().any(|p| p.answer.is_none()) {
            return;
        }
        let multi = self.multis.swap_remove(m);
        let requestor = multi.req.requestor;
        let mut asked = Vec::with_capacity(multi.pairs.len());
        let mut refused = Vec::with_capacity(multi.pairs.len());
        for pair in multi.pairs {
            asked.push((pair.target, pair.property));
            match pair.answer.unwrap_or(Answer::Refused) {
                Answer::Refused => refused.push(true),
                Answer::Words(ty, words) => {
                    let _ = self.conn.change_property32(
                        PropMode::REPLACE,
                        requestor,
                        pair.property,
                        ty,
                        &words,
                    );
                    refused.push(false);
                }
                Answer::Bytes(data) => {
                    self.put(requestor, pair.property, pair.target, data);
                    refused.push(false);
                }
            }
        }
        let list = answered_pairs(&asked, &refused);
        let _ = self.conn.change_property32(
            PropMode::REPLACE,
            requestor,
            multi.req.property,
            self.atoms.ATOM_PAIR,
            &list,
        );
        self.notify(&multi.req, multi.req.property);
    }
}

#[cfg(test)]
mod tests;
