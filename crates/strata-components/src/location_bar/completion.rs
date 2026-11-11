//! The completion model: which suggestions show, which is highlighted, the listing in flight,
//! and the keys that wait for it. A port of Strata's `PathCompletion`
//! (`src/ui/browser/location/completion.rs`), without the listing, which is the host's.
//!
//! It knows no Rune: the state layer turns its [`Out`] into text edits and intents.

use super::data::{Candidate, CompletionQuery};

#[cfg(test)]
mod tests;

/// A key the model handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    /// Up, Down by one (wrapping), Page Up and Page Down by five.
    Step(i32),
    /// Down: steps while the list shows, reopens it otherwise.
    Down,
    /// Tab (or another complete key).
    Complete,
    /// Enter.
    Activate,
}

/// What the entry should do after a command or an arrival.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Out {
    /// Replace the entry's text (and put the caret at its end), asking for no suggestions.
    pub text: Option<String>,
    /// Submit this text.
    pub submit: Option<String>,
    /// Ask the host for suggestions.
    pub queries: Vec<CompletionQuery>,
    /// A pending listing was dropped.
    pub cancelled: bool,
}

impl Out {
    fn merge(&mut self, other: Out) {
        if other.text.is_some() {
            self.text = other.text;
        }
        if other.submit.is_some() {
            self.submit = other.submit;
        }
        self.queries.extend(other.queries);
        self.cancelled |= other.cancelled;
    }
}

#[derive(Clone, Debug)]
struct Pending {
    generation: u64,
    text: String,
    deferred: Vec<Command>,
}

/// The suggestions and the listing in flight.
#[derive(Clone, Debug, Default)]
pub(crate) struct Completion {
    generation: u64,
    /// The generation of the list shown.
    shown_generation: u64,
    shown: Vec<Candidate>,
    highlighted: Option<usize>,
    open: bool,
    pending: Option<Pending>,
}

impl Completion {
    pub fn shown(&self) -> &[Candidate] {
        &self.shown
    }

    /// The generation of the list shown: its rows' memo version.
    pub fn shown_generation(&self) -> u64 {
        self.shown_generation
    }

    pub fn highlighted(&self) -> Option<usize> {
        self.highlighted.filter(|&i| i < self.shown.len())
    }

    pub fn set_highlighted(&mut self, i: Option<usize>) {
        self.highlighted = i.filter(|&i| i < self.shown.len());
    }

    /// Whether the list shows.
    pub fn is_open(&self) -> bool {
        self.open && !self.shown.is_empty()
    }

    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Ask for suggestions for `text`. The list stays as it is until they land, with nothing
    /// highlighted; an earlier listing still in flight is superseded.
    pub fn request(&mut self, text: &str) -> CompletionQuery {
        self.generation = self.generation.wrapping_add(1);
        self.highlighted = None;
        self.pending = Some(Pending {
            generation: self.generation,
            text: text.to_owned(),
            deferred: Vec::new(),
        });
        CompletionQuery {
            generation: self.generation,
            text: text.to_owned(),
        }
    }

    /// Drops the listing in flight, if any. True when there was one.
    pub fn cancel(&mut self) -> bool {
        self.pending.take().is_some()
    }

    /// Hides the list and drops the listing in flight; the suggestions stay for Down.
    pub fn dismiss(&mut self) -> bool {
        self.open = false;
        self.cancel()
    }

    /// Forgets everything: the edit ended.
    pub fn clear(&mut self) -> bool {
        self.shown.clear();
        self.highlighted = None;
        self.open = false;
        self.cancel()
    }

    /// An answer landed for `generation`, with the entry holding `text`. A stale answer (another
    /// generation, or text that changed since) is dropped and returns `None`. Otherwise the list
    /// shows it, and the keys that waited for it run in order.
    pub fn arrive(
        &mut self,
        generation: u64,
        candidates: Vec<Candidate>,
        text: &str,
    ) -> Option<Out> {
        if self
            .pending
            .as_ref()
            .is_none_or(|p| p.generation != generation)
        {
            return None;
        }
        let pending = self.pending.take()?;
        if pending.text != text {
            return None;
        }
        self.show(candidates);
        self.shown_generation = generation;
        let mut out = Out::default();
        let mut text = text.to_owned();
        let mut commands = pending.deferred.into_iter();
        while let Some(command) = commands.next() {
            let relist = text != pending.text;
            let (step, proceeds) = self.press(command, relist, &text);
            if let Some(t) = &step.text {
                text.clone_from(t);
            }
            let activated = matches!(command, Command::Activate) && proceeds;
            out.merge(step);
            if activated {
                out.submit = Some(text.clone());
            }
            if let Some(next) = self.pending.as_mut() {
                next.deferred.extend(commands);
                break;
            }
        }
        Some(out)
    }

    fn show(&mut self, candidates: Vec<Candidate>) {
        self.highlighted = None;
        self.open = !candidates.is_empty();
        self.shown = candidates;
    }

    /// A key with the entry holding `text`. `relist` says the text changed since the list was
    /// asked for, so a hidden list must be asked for again. Returns what to do, and whether the
    /// key goes on (Enter with nothing highlighted submits the typed text; Up and the paging keys
    /// with no list move nothing).
    pub fn press(&mut self, command: Command, relist: bool, text: &str) -> (Out, bool) {
        let mut out = Out::default();
        if self.defer(command) {
            return (out, false);
        }
        let visible = self.is_open();
        match command {
            Command::Complete => {
                if relist && !visible {
                    out.queries.push(self.request(text));
                    if self.defer(Command::Complete) {
                        return (out, false);
                    }
                }
                out.merge(self.complete(text));
            }
            Command::Down if visible => self.step(1),
            Command::Down => {
                if relist && self.pending.is_none() {
                    out.queries.push(self.request(text));
                }
            }
            Command::Step(delta) if visible => self.step(delta),
            Command::Step(_) => return (out, true),
            Command::Activate => match self.highlighted().filter(|_| visible) {
                Some(i) => {
                    let replacement = self.shown[i].replacement.clone();
                    out.cancelled = self.dismiss();
                    out.text = Some(replacement.clone());
                    out.submit = Some(replacement);
                }
                None => {
                    out.cancelled = self.dismiss();
                    return (out, true);
                }
            },
        }
        (out, false)
    }

    /// Suggestion `i` chosen with the pointer: its replacement, submitted at once. The listing in
    /// flight and the keys waiting for it are dropped. `None` when there is no such suggestion.
    pub fn choose(&mut self, i: usize) -> Option<Out> {
        let replacement = self.shown.get(i)?.replacement.clone();
        let cancelled = self.dismiss();
        Some(Out {
            text: Some(replacement.clone()),
            submit: Some(replacement),
            queries: Vec::new(),
            cancelled,
        })
    }

    /// Whether `command` waits for the listing in flight: Tab always does, Down, Up and the
    /// paging keys do while the list shows, and Enter does behind any of those.
    fn defer(&mut self, command: Command) -> bool {
        let visible = self.is_open();
        let Some(pending) = self.pending.as_mut() else {
            return false;
        };
        let waits = !pending.deferred.is_empty()
            || match command {
                Command::Complete => true,
                Command::Down | Command::Step(_) => visible,
                Command::Activate => false,
            };
        if waits {
            pending.deferred.push(command);
        }
        waits
    }

    /// Tab: the highlighted suggestion, the only one, or a longer common prefix, else the next
    /// suggestion highlighted.
    fn complete(&mut self, text: &str) -> Out {
        let mut out = Out::default();
        if self.shown.is_empty() {
            return out;
        }
        let take = if let Some(i) = self.highlighted() {
            Some(self.shown[i].replacement.clone())
        } else if self.shown.len() == 1 {
            Some(self.shown[0].replacement.clone())
        } else {
            longest_common_prefix(self.shown.iter().map(|c| c.replacement.as_str()))
                .filter(|common| common.len() > text.len())
                .map(str::to_owned)
        };
        match take {
            Some(t) => {
                out.cancelled = self.dismiss();
                out.text = Some(t);
            }
            None => self.step(1),
        }
        out
    }

    /// Moves the highlight by `delta`, wrapping at the ends. With nothing highlighted, a step
    /// forward lands on the first row and a step back on the last.
    fn step(&mut self, delta: i32) {
        let count = self.shown.len();
        if count == 0 {
            return;
        }
        let n = count as i64;
        let next = match self.highlighted() {
            Some(current) => (current as i64 + i64::from(delta)).rem_euclid(n),
            None if delta > 0 => 0,
            None => n - 1,
        };
        self.highlighted = Some(next as usize);
    }
}

/// The longest prefix every string shares, cut at a character boundary; `None` when there are
/// no strings or they share nothing.
pub(crate) fn longest_common_prefix<'a>(
    mut strings: impl Iterator<Item = &'a str>,
) -> Option<&'a str> {
    let first = strings.next()?;
    let mut len = first.len();
    for s in strings {
        let shared = first
            .bytes()
            .zip(s.bytes())
            .take(len)
            .take_while(|(a, b)| a == b)
            .count();
        len = shared;
        while !first.is_char_boundary(len) {
            len -= 1;
        }
        if len == 0 {
            return None;
        }
    }
    (len > 0).then(|| &first[..len])
}
