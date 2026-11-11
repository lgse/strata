//! The override file: TOML parsed line by line where it breaks, then each row read against
//! the registry.
//!
//! toml's own recovery (`DeTable::parse_recoverable`, toml 1.2.1) isn't enough to keep every
//! good row: after a line of garbage it drops every row below it, a bad value comes back as an
//! empty string, an unclosed `[header` leaves its rows at the top level, and an unclosed `{`,
//! `[` or `"""` swallows every line after it. So [`salvage`] blanks the lines toml reports and
//! parses again until nothing is reported:
//!
//! - Blanking writes spaces over a line, keeping its line break, so every span still points
//!   into the original text. A broken table header takes the rows under it with it.
//! - An unclosed array, inline table or multi-line string reports knock-on errors on every
//!   line it swallowed. So when toml reports one, the round blanks only the line where the
//!   value holding the first error opens (or that error's own line), reports the unclosed
//!   error there, and parses again: the lines after it parse on their own. After
//!   [`OPENER_ROUNDS`] such rounds, the rest go in batches, so a file of nothing but unclosed
//!   arrays stays fast.
//! - Lines are found from an index built once, so the loop and the walk stay near linear.
//! - A row whose value runs over a blanked line (a multi-line string with a bad escape)
//!   doesn't apply.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::OnceLock;

use toml::Spanned;
use toml::de::{DeTable, DeValue};

use super::{OverrideError, Overrides, Problem, Row, Scope, value};
use crate::theme::Location;
use crate::tokens::{self, Token};

/// How many rounds may each blank one unclosed value's opening line before the rest are
/// blanked in batches.
const OPENER_ROUNDS: usize = 64;

pub(super) fn parse(src: &str) -> Overrides {
    let lines = Lines::new(src);
    let salvaged = salvage(src, &lines);
    let (doc, _) = DeTable::parse_recoverable(&salvaged.text);
    let mut out = Overrides::default();
    let mut walk = Walk {
        src,
        lines: &lines,
        blanked: &salvaged.blanked,
        cutoff: salvaged.cutoff,
        out: &mut out,
    };
    walk.table(doc.get_ref(), "", Scope::All);
    let mut errors = salvaged.errors;
    errors.append(&mut out.errors);
    errors.sort_by_key(|e| (e.at.line, e.at.column));
    out.errors = errors;
    out
}

/// Where each line of a text starts, to find a byte's line and column without rescanning.
pub(super) struct Lines {
    starts: Vec<usize>,
    len: usize,
}

impl Lines {
    pub(super) fn new(text: &str) -> Lines {
        let starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        Lines {
            starts,
            len: text.len(),
        }
    }

    fn count(&self) -> usize {
        self.starts.len()
    }

    /// The index of the line holding byte `at`.
    fn index(&self, at: usize) -> usize {
        self.starts.partition_point(|&s| s <= at).saturating_sub(1)
    }

    /// Line `i`'s bytes, without its line break.
    fn range(&self, i: usize) -> Range<usize> {
        let end = self.starts.get(i + 1).map_or(self.len, |&next| next - 1);
        self.starts[i]..end
    }

    /// The 1-based line and column of byte `at` in `text`, moved back onto a character
    /// boundary.
    pub(super) fn locate(&self, text: &str, at: usize) -> Location {
        let mut at = at.min(text.len());
        while !text.is_char_boundary(at) {
            at -= 1;
        }
        let i = self.index(at);
        Location {
            line: i + 1,
            column: text[self.starts[i]..at].chars().count() + 1,
        }
    }
}

/// What [`salvage`] leaves to parse.
struct Salvaged {
    /// The text with every broken line blanked.
    text: String,
    /// Which lines were blanked, by index.
    blanked: Vec<bool>,
    /// The byte from which nothing applies: the end, unless an error couldn't be blanked.
    cutoff: usize,
    /// The syntax errors.
    errors: Vec<OverrideError>,
}

/// One error toml reported in a round.
struct Found {
    at: usize,
    message: String,
}

impl Found {
    /// toml says an array, inline table or multi-line string is never closed: it swallowed
    /// the lines after it, and reported knock-on errors on them.
    fn unclosed(&self) -> bool {
        // Not `unclosed table`: a broken header's own line is the one to blank.
        let m = &self.message;
        m.starts_with("unclosed array")
            || m.starts_with("unclosed inline table")
            || m.contains("multi-line")
    }
}

fn salvage(src: &str, lines: &Lines) -> Salvaged {
    let mut text = src.as_bytes().to_vec();
    let mut blanked = vec![false; lines.count()];
    let mut errors = Vec::new();
    let mut reported = HashSet::new();
    let mut opener_rounds = 0;
    let mut report = |errors: &mut Vec<OverrideError>, at: usize, message: String| {
        if reported.insert(at) {
            errors.push(OverrideError {
                at: lines.locate(src, at),
                scope: Scope::All,
                problem: Problem::Syntax(message),
            });
        }
    };
    // Each round blanks at least one line or stops, so this bounds the loop.
    for _ in 0..=lines.count() {
        let (mut found, values) = round(&text);
        if found.is_empty() {
            return Salvaged {
                text: into_string(text),
                blanked,
                cutoff: usize::MAX,
                errors,
            };
        }
        found.sort_by_key(|f| f.at);
        // The outermost value that holds byte `at`, or runs to it: where it opens.
        let opener = |at: usize| {
            values
                .iter()
                .filter(|v| v.start < at && at <= v.end)
                .map(|v| v.start)
                .min()
        };

        // An unclosed value: blank only the line where the first error's value opens (or the
        // first error's own line), so the lines it swallowed parse next round.
        if opener_rounds < OPENER_ROUNDS && found.iter().any(Found::unclosed) {
            let first = &found[0];
            let open = opener(first.at).unwrap_or(first.at);
            let i = lines.index(open);
            if !blanked[i] && blank(&mut text, lines, &mut blanked, i).changed {
                opener_rounds += 1;
                let named = found.iter().find(|f| f.unclosed()).unwrap_or(first);
                report(&mut errors, open, named.message.clone());
                continue;
            }
        }

        let mut progressed = false;
        let mut stuck = Vec::new();
        for f in &found {
            let i = lines.index(f.at);
            if blanked[i] {
                // A consequence of a line blanked this round, such as a row under a broken
                // header.
                continue;
            }
            let b = blank(&mut text, lines, &mut blanked, i);
            if !b.changed {
                stuck.push(f);
                continue;
            }
            progressed = true;
            let message = if b.rows > 0 {
                format!("{}; the rows under it don't apply", f.message)
            } else {
                f.message.clone()
            };
            report(&mut errors, f.at, message);
        }
        if progressed {
            continue;
        }
        // Nothing on a reported line is left to blank, such as a string left open at the end
        // of the file: blank where its value opens if that is known, else stop there.
        let Some(f) = stuck.first().copied().or(found.first()) else {
            break;
        };
        match opener(f.at).filter(|&open| !blanked[lines.index(open)]) {
            Some(open) => {
                blank(&mut text, lines, &mut blanked, lines.index(open));
                report(&mut errors, open, f.message.clone());
            }
            None => {
                report(&mut errors, f.at, f.message.clone());
                return Salvaged {
                    text: into_string(text),
                    blanked,
                    cutoff: f.at,
                    errors,
                };
            }
        }
    }
    Salvaged {
        text: into_string(text),
        blanked,
        cutoff: 0,
        errors,
    }
}

/// Parses `text` once: each error, and the span of every value it read.
fn round(text: &[u8]) -> (Vec<Found>, Vec<Range<usize>>) {
    let s = std::str::from_utf8(text).expect("blanking keeps UTF-8");
    let (doc, errs) = DeTable::parse_recoverable(s);
    let mut values = Vec::new();
    spans(doc.get_ref(), &mut values);
    let found = errs
        .iter()
        .map(|e| Found {
            at: e.span().map_or(s.len(), |r| r.start),
            message: e.message().trim().to_owned(),
        })
        .collect();
    (found, values)
}

/// The spans of every value in `table`, nested ones included. A dotted or header table's
/// span is its key's, which reaches no error.
fn spans(table: &DeTable<'_>, out: &mut Vec<Range<usize>>) {
    fn value(v: &Spanned<DeValue<'_>>, out: &mut Vec<Range<usize>>) {
        out.push(v.span());
        match v.get_ref() {
            DeValue::Table(t) => spans(t, out),
            DeValue::Array(a) => a.iter().for_each(|v| value(v, out)),
            _ => {}
        }
    }
    for (_, v) in table.iter() {
        value(v, out);
    }
}

fn into_string(text: Vec<u8>) -> String {
    String::from_utf8(text).expect("blanking keeps UTF-8")
}

/// What [`blank`] did.
struct Blanked {
    /// Some byte changed.
    changed: bool,
    /// Rows under a broken table header blanked with it.
    rows: usize,
}

/// Blanks line `i`, and when it is a table header, the rows under it up to the next header:
/// they would otherwise land in the table above.
fn blank(text: &mut [u8], lines: &Lines, blanked: &mut [bool], i: usize) -> Blanked {
    let header = text[lines.range(i)].trim_ascii_start().starts_with(b"[");
    let mut changed = spaces(text, lines.range(i));
    blanked[i] = true;
    let mut rows = 0;
    if header {
        for j in i + 1..lines.count() {
            let l = text[lines.range(j)].trim_ascii();
            if l.starts_with(b"[") {
                break;
            }
            let row = !l.is_empty() && !l.starts_with(b"#");
            if spaces(text, lines.range(j)) {
                changed = true;
                rows += usize::from(row);
            }
            blanked[j] = true;
        }
    }
    Blanked { changed, rows }
}

/// Writes spaces over `range`, which holds no line break. A carriage return just before the
/// line break stays, so a CRLF line stays one; TOML takes no other control character, so
/// every other byte becomes a space. True when anything changed.
fn spaces(text: &mut [u8], range: Range<usize>) -> bool {
    let crlf = text.get(range.end) == Some(&b'\n') && range.end > range.start;
    let end = if crlf && text[range.end - 1] == b'\r' {
        range.end - 1
    } else {
        range.end
    };
    let mut changed = false;
    for b in &mut text[range.start..end] {
        if *b != b' ' {
            *b = b' ';
            changed = true;
        }
    }
    changed
}

/// Every token by name, and every name that starts tokens' names (`list.row`).
fn registry() -> &'static (HashMap<&'static str, &'static Token>, HashSet<&'static str>) {
    static REGISTRY: OnceLock<(HashMap<&'static str, &'static Token>, HashSet<&'static str>)> =
        OnceLock::new();
    REGISTRY.get_or_init(|| {
        let names = tokens::all().map(|t| (t.name(), t)).collect();
        let groups = tokens::all()
            .flat_map(|t| t.name().match_indices('.').map(|(i, _)| &t.name()[..i]))
            .collect();
        (names, groups)
    })
}

/// Reads a parsed table's rows into `out`.
struct Walk<'a> {
    src: &'a str,
    lines: &'a Lines,
    blanked: &'a [bool],
    cutoff: usize,
    out: &'a mut Overrides,
}

impl Walk<'_> {
    fn error(&mut self, at: usize, scope: Scope, problem: Problem) {
        self.out.errors.push(OverrideError {
            at: self.lines.locate(self.src, at),
            scope,
            problem,
        });
    }

    /// A problem with a key: reported where its line's text starts, so a dotted key
    /// (`list.row.length = 3`) or a table header (`[list.roww]`) reads from its first segment.
    fn key_error(&mut self, at: usize, scope: Scope, problem: Problem) {
        let line = self.lines.range(self.lines.index(at));
        let indent = self.src.as_bytes()[line.clone()]
            .iter()
            .take_while(|b| b.is_ascii_whitespace())
            .count();
        self.error((line.start + indent).min(at), scope, problem);
    }

    /// True when `span` runs into a blanked line or past the cutoff: a value toml read from
    /// what was left of a broken one.
    fn broken(&self, span: Range<usize>) -> bool {
        if span.end >= self.cutoff {
            return true;
        }
        let (first, last) = (
            self.lines.index(span.start),
            self.lines.index(span.end.max(span.start)),
        );
        self.blanked[first..=last].iter().any(|&b| b)
    }

    /// The rows of `table`, whose keys continue `prefix` (empty at the top of the file or of
    /// a scope).
    fn table(&mut self, table: &DeTable<'_>, prefix: &str, scope: Scope) {
        for (key, value) in table.iter() {
            if self.broken(value.span()) {
                continue;
            }
            let k: &str = key.get_ref();
            // A token's segments are snake_case words. Any other key was quoted ("a.b" is one
            // key, not two), and names no token.
            let segment = !k.is_empty()
                && k.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
            let written = if segment {
                k.to_owned()
            } else {
                format!("{k:?}")
            };
            let name = if prefix.is_empty() {
                written
            } else {
                format!("{prefix}.{written}")
            };
            let at = key.span().start;
            if prefix.is_empty() && matches!(k, "light" | "dark") {
                match (scope, value.get_ref()) {
                    (Scope::All, DeValue::Table(t)) => {
                        let inner = if k == "dark" {
                            Scope::Dark
                        } else {
                            Scope::Light
                        };
                        self.table(t, "", inner);
                    }
                    _ => self.key_error(at, scope, Problem::Scope(k.to_owned())),
                }
                continue;
            }
            self.row(&name, at, value, scope);
        }
    }

    fn row(&mut self, name: &str, at: usize, value: &Spanned<DeValue<'_>>, scope: Scope) {
        let (names, groups) = registry();
        if let Some(&token) = names.get(name) {
            match value::read(token, value.get_ref()) {
                Ok(v) => self.out.rows.push(Row {
                    scope,
                    token,
                    value: v,
                    at: self.lines.locate(self.src, at),
                }),
                Err(problem) => self.error(value.span().start, scope, problem),
            }
            return;
        }
        let group = groups.contains(name);
        match value.get_ref() {
            DeValue::Table(t) if group => self.table(t, name, scope),
            _ if group => self.key_error(at, scope, Problem::Group(name.to_owned())),
            _ => self.key_error(at, scope, Problem::Unknown(name.to_owned())),
        }
    }
}
