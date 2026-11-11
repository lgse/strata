//! Unit tests for the highlighter.
//!
//! Colours are checked against syntect's own highlighter (`HighlightState` with
//! `RangedHighlightIterator`) over a full parse, so they also check that colouring by scope
//! stack class gives syntect's colours. Most tests drive the worker on the test thread
//! through [`Pump`], so they are deterministic; a few run the real thread.

mod perf;

use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rune::{Color, Span};
use strata_kit::look::Look;
use strata_kit::theme::syntax::syntect_theme;
use syntect::highlighting::{
    HighlightState, Highlighter as SynHighlighter, RangedHighlightIterator, Theme,
};
use syntect::parsing::{ParseState, ScopeStack, SyntaxReference};

use super::detect::{detect, syntaxes};
use super::engine::Lines;
use super::palette::to_rune;
use super::worker::{Request, Worker};
use super::*;

/// How long a test waits on the real thread. Generous: debug builds parse slowly.
const WAIT: Duration = Duration::from_secs(30);

pub(super) fn fixture(name: &str) -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/code/");
    std::fs::read_to_string(format!("{path}{name}")).expect("fixture")
}

/// The first `n` lines of a fixture.
fn head(name: &str, n: usize) -> String {
    fixture(name).split_inclusive('\n').take(n).collect()
}

pub(super) fn light() -> Arc<Theme> {
    Arc::new(syntect_theme(&Look::fallback(false)))
}

fn dark() -> Arc<Theme> {
    Arc::new(syntect_theme(&Look::fallback(true)))
}

type Runs = Vec<(Range<usize>, Color)>;

/// syntect's own colours for every line of `text`, after a full parse, as runs that join
/// where they touch, with default-coloured text left out. Lines longer than `long_line`
/// are skipped as the highlighter skips them: plain, with the state carried over.
pub(super) fn reference(
    text: &str,
    syntax: &SyntaxReference,
    theme: &Theme,
    long_line: usize,
) -> Vec<Runs> {
    let set = syntaxes().expect("grammars");
    let hl = SynHighlighter::new(theme);
    let default = hl.get_default().foreground;
    let mut ps = ParseState::new(syntax);
    let mut hs = HighlightState::new(&hl, ScopeStack::new());
    let lines = Lines::new(text.into());
    (0..lines.len())
        .map(|i| {
            let content = lines.content(i);
            if content.len() > long_line {
                return Vec::new();
            }
            let line = format!("{content}\n");
            let ops = ps.parse_line(&line, set).expect("parse");
            let mut runs: Runs = Vec::new();
            for (style, _, r) in RangedHighlightIterator::new(&mut hs, &ops, &line, &hl) {
                let r = r.start.min(content.len())..r.end.min(content.len());
                if r.is_empty() || style.foreground == default {
                    continue;
                }
                let c = to_rune(style.foreground);
                match runs.last_mut() {
                    Some(last) if last.0.end == r.start && last.1 == c => last.0.end = r.end,
                    _ => runs.push((r, c)),
                }
            }
            runs
        })
        .collect()
}

fn runs_of(spans: &[Span]) -> Runs {
    spans
        .iter()
        .map(|s| {
            (
                s.start as usize..s.end as usize,
                s.color.expect("colour-only span"),
            )
        })
        .collect()
}

fn syntax_for(file_name: &str, text: &str) -> &'static SyntaxReference {
    detect(syntaxes().expect("grammars"), None, Some(file_name), text)
}

/// A highlighter worker driven on the test thread: requests queue until [`Pump::run`].
struct Pump {
    worker: Worker,
    rx: Receiver<Request>,
    tx: Sender<Request>,
    blocks_asked: usize,
    shows_asked: usize,
}

impl Pump {
    fn new(theme: Arc<Theme>) -> Pump {
        let (tx, rx) = mpsc::channel();
        Pump {
            worker: Worker::new(theme, Options::default(), Arc::default()),
            rx,
            tx,
            blocks_asked: 0,
            shows_asked: 0,
        }
    }

    fn open(&self, src: Source) -> CodeDoc {
        CodeDoc::open(Some(self.tx.clone()), src, Options::default())
    }

    fn blocks(&self) -> CodeBlocks {
        CodeBlocks::new(Some(self.tx.clone()))
    }

    fn set_theme(&self, theme: Arc<Theme>) {
        self.tx.send(Request::SetTheme(theme)).expect("send");
    }

    /// Hands the queued requests to the worker, and sends their replies.
    fn requests(&mut self) {
        while let Ok(req) = self.rx.try_recv() {
            match req {
                Request::Block { .. } => self.blocks_asked += 1,
                Request::Show { .. } => self.shows_asked += 1,
                _ => {}
            }
            self.worker.handle(req);
        }
        self.worker.flush();
    }

    /// One unit of work, and its replies.
    fn step(&mut self) -> bool {
        let did = self.worker.engine.step();
        self.worker.flush();
        did
    }

    /// Everything queued, to the end of every pass.
    /// Everything queued, to the end of every pass. Requests only come from the test
    /// thread, so none arrives while a step runs.
    fn run(&mut self) {
        loop {
            self.requests();
            if !self.step() {
                break;
            }
        }
    }

    fn parsed(&self, doc: &CodeDoc) -> u64 {
        self.worker.engine.doc(doc.id).map_or(0, |d| d.parsed)
    }
}

/// Every line's colours, after asking for all of them.
fn all_lines(pump: &mut Pump, doc: &mut CodeDoc) -> Vec<Runs> {
    doc.show(0..doc.line_count());
    pump.run();
    doc.poll();
    (0..doc.line_count())
        .map(|i| {
            runs_of(
                &doc.line_spans(i)
                    .unwrap_or_else(|| panic!("line {i} has no colours")),
            )
        })
        .collect()
}

fn assert_lines_eq(got: &[Runs], want: &[Runs], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: line count");
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert_eq!(g, w, "{what}: line {i}");
    }
}

// Detection.

fn detected(name: Option<&str>, text: &str) -> String {
    detect(syntaxes().expect("grammars"), None, name, text)
        .name
        .clone()
}

#[test]
fn detects_by_file_name() {
    assert_eq!(detected(Some("Makefile"), ""), "Makefile");
    assert_eq!(detected(Some("PKGBUILD"), ""), "Bourne Again Shell (bash)");
    assert_eq!(detected(Some("Cargo.lock"), ""), "TOML");
    assert_eq!(
        detected(Some("/home/me/.bashrc"), ""),
        "Bourne Again Shell (bash)"
    );
}

#[test]
fn detects_by_extension_in_any_case() {
    assert_eq!(detected(Some("main.rs"), ""), "Rust");
    assert_eq!(detected(Some("SETUP.PY"), ""), "Python");
    assert_eq!(detected(Some("src/lib/io.c"), ""), "C");
}

#[test]
fn detects_shebangs_from_the_first_line() {
    assert_eq!(
        detected(None, "#!/usr/bin/env bash\necho hi\n"),
        "Bourne Again Shell (bash)"
    );
    assert_eq!(
        detected(Some("run"), "#!/usr/bin/python3\nprint(1)\n"),
        "Python"
    );
    assert_eq!(
        detected(None, "\u{feff}#!/bin/sh\n"),
        "Bourne Again Shell (bash)"
    );
}

#[test]
fn detects_vim_and_emacs_modelines() {
    assert_eq!(
        detected(None, "// vim: set ft=rust:\nfn main() {}\n"),
        "Rust"
    );
    assert_eq!(detected(None, "x = 1\n\n# vi: filetype=python\n"), "Python");
    assert_eq!(
        detected(None, "echo\n# ex: ts=4 ft=sh.doc\n"),
        "Bourne Again Shell (bash)"
    );
    let tail = format!("{}# vim: ft=python\n", "line\n".repeat(30));
    assert_eq!(detected(None, &tail), "Python");
    assert_eq!(detected(None, "/* -*- c++ -*- */\nint x;\n"), "C++");
    assert_eq!(
        detected(None, "# -*- mode: ruby; coding: utf-8 -*-\n"),
        "Ruby"
    );
    // Vim wants the marker after white space, and reads only the first and last 5 lines.
    assert_eq!(detected(None, "novim: ft=rust\n"), "Plain Text");
    let middle = format!("{}// vim: ft=rust\n{}", "a\n".repeat(10), "b\n".repeat(10));
    assert_eq!(detected(None, &middle), "Plain Text");
}

#[test]
fn the_name_beats_the_first_line_and_an_explicit_grammar_beats_both() {
    assert_eq!(detected(Some("tool.py"), "#!/usr/bin/env bash\n"), "Python");
    let s = detect(
        syntaxes().expect("grammars"),
        Some("rust"),
        Some("tool.py"),
        "#!/usr/bin/env bash\n",
    );
    assert_eq!(s.name, "Rust");
    let s = detect(
        syntaxes().expect("grammars"),
        Some("Bourne Again Shell (bash)"),
        None,
        "",
    );
    assert_eq!(s.name, "Bourne Again Shell (bash)");
}

#[test]
fn unknown_text_falls_back_to_plain_text() {
    assert_eq!(
        detected(Some("notes.unknownext"), "just words\n"),
        "Plain Text"
    );
    assert_eq!(detected(None, ""), "Plain Text");
    let s = detect(
        syntaxes().expect("grammars"),
        Some("no-such-grammar"),
        None,
        "",
    );
    assert_eq!(s.name, "Plain Text");
}

// Checkpoints.

/// Replies from the engine for one document, applied as the UI side does.
fn engine_lines(
    engine_out: Vec<(u64, super::engine::Reply)>,
    palette: &mut Vec<Option<Color>>,
) -> Vec<(usize, Runs)> {
    let mut lines = Vec::new();
    for (_, reply) in engine_out {
        match reply {
            super::engine::Reply::Palette { from, colors } => {
                palette.truncate(from);
                palette.extend(colors);
            }
            super::engine::Reply::Lines {
                first, lines: l, ..
            } => {
                for (i, tokens) in l.into_iter().enumerate() {
                    let mut runs = Vec::new();
                    spans::colored(&tokens, palette, 0, &mut runs);
                    lines.push((first + i, runs));
                }
            }
            _ => {}
        }
    }
    lines
}

#[test]
fn a_viewport_resumed_from_a_checkpoint_matches_a_full_parse() {
    let theme = light();
    for (name, n) in [
        ("rust-50k.rs", 1500),
        ("c-50k.c", 1500),
        ("js-50k.js", 1500),
        ("python-50k.py", 1500),
        ("markdown-50k.md", 1500),
        ("shell-5k.sh", 5000),
    ] {
        let text = head(name, n);
        let want = reference(&text, syntax_for(name, &text), &theme, LONG_LINE);
        let mut engine = super::engine::Engine::new(theme.clone(), Options::default());
        let open = |text: &str| super::engine::OpenArgs {
            text: text.into(),
            file_name: Some(name.into()),
            syntax: None,
            mode: Mode::Preview,
        };
        engine.open(1, 1, open(&text));
        // A jump to the end before the pass gets there: the lines land when it does.
        let lines = want.len();
        engine.show(1, 2, lines - 80..lines);
        while engine.step() {}
        let mut palette = Vec::new();
        let landed = engine_lines(engine.take_out(), &mut palette);
        assert!(
            landed.iter().any(|(i, _)| *i == lines - 1),
            "{name}: the end landed"
        );
        for (i, runs) in &landed {
            assert_eq!(runs, &want[*i], "{name}: line {i}, deferred to the pass");
        }
        // Each window resumes from the checkpoint at or above it.
        for start in [0, 63, 64, 65, 700, lines - 80] {
            engine.show(1, 3, start..start + 80);
            while engine.step() {}
            let landed = engine_lines(engine.take_out(), &mut palette);
            let covered: Vec<usize> = landed.iter().map(|(i, _)| *i).collect();
            assert!(
                (start..start + 80).all(|i| covered.contains(&i)),
                "{name}: window {start}"
            );
            for (i, runs) in &landed {
                assert_eq!(runs, &want[*i], "{name}: line {i}, window from {start}");
            }
        }
    }
}

#[test]
fn a_line_over_the_cap_draws_plain_and_carries_the_state_over() {
    let theme = light();
    let long = format!("let s = \"{}\";\n", "x".repeat(LONG_LINE + 1024));
    let text = format!("fn a() {{ let n = 1; }}\n{long}fn b() {{ let m = \"two\"; }}\n// done\n");
    let mut pump = Pump::new(theme.clone());
    let mut doc = pump.open(Source::new(text.as_str()).file_name("long.rs"));
    let got = all_lines(&mut pump, &mut doc);
    let want = reference(&text, syntax_for("long.rs", &text), &theme, LONG_LINE);
    assert_lines_eq(&got, &want, "long line");
    assert!(got[1].is_empty(), "the long line is plain");
    assert!(!got[2].is_empty(), "the line after it is coloured");
}

#[test]
fn crlf_lines_colour_like_lf_lines() {
    let lf = "fn main() {\n    let s = \"hi\"; // greet\n}\n";
    let crlf = lf.replace('\n', "\r\n");
    let mut pump = Pump::new(light());
    let mut a = pump.open(Source::new(lf).file_name("a.rs"));
    let mut b = pump.open(Source::new(crlf.as_str()).file_name("a.rs"));
    let (la, lb) = (all_lines(&mut pump, &mut a), all_lines(&mut pump, &mut b));
    assert_eq!(la, lb);
    assert!(la.iter().any(|l| !l.is_empty()));
}

#[test]
fn lines_draw_plain_until_their_colours_land() {
    let mut pump = Pump::new(light());
    let mut doc = pump.open(Source::new("fn main() {}\n").file_name("a.rs"));
    doc.show(0..1);
    assert!(doc.line_spans(0).is_none());
    pump.run();
    assert!(doc.line_spans(0).is_none(), "nothing lands before poll");
    assert!(doc.poll());
    assert_eq!(doc.syntax(), Some("Rust"));
    assert!(doc.line_spans(0).is_some_and(|s| !s.is_empty()));
}

// Edits (S4's workaround 1).

/// Applies `new` to an editable document that has every line cached, runs the thread to the
/// end of the background pass, and checks every line against a full parse of `new`.
fn edit_and_check(
    pump: &mut Pump,
    doc: &mut CodeDoc,
    name: &str,
    new: &str,
    visible: Range<usize>,
    what: &str,
) {
    let old = doc.text().clone();
    doc.edit(new.into(), LineEdit::diff(&old, new), visible);
    pump.run();
    doc.poll();
    assert_eq!(doc.provisional_from(), None, "{what}: the pass ended");
    let got = all_lines(pump, doc);
    let want = reference(new, syntax_for(name, new), &light(), LONG_LINE);
    assert_lines_eq(&got, &want, what);
}

fn replace_line(text: &str, line: usize, f: impl Fn(&str) -> String) -> String {
    text.split_inclusive('\n')
        .enumerate()
        .map(|(i, l)| if i == line { f(l) } else { l.to_owned() })
        .collect()
}

fn line_index(text: &str, from: usize, pred: impl Fn(&str) -> bool) -> usize {
    text.lines()
        .enumerate()
        .skip(from)
        .find(|(_, l)| pred(l))
        .map(|(i, _)| i)
        .expect("a matching line")
}

#[test]
fn edits_are_exact_once_the_background_pass_ends() {
    let name = "shell-5k.sh";
    let text = head(name, 700);
    let mut pump = Pump::new(light());
    let mut doc = pump.open(
        Source::new(text.as_str())
            .file_name(name)
            .mode(Mode::Editable),
    );
    all_lines(&mut pump, &mut doc);

    let at = line_index(&text, 100, |l| l.contains("echo"));
    let renamed = replace_line(&text, at, |l| l.replacen("echo", "printf", 1));
    edit_and_check(
        &mut pump,
        &mut doc,
        name,
        &renamed,
        at - 10..at + 70,
        "rename",
    );

    // An unmatched quote cascades to the end of the text.
    let at = line_index(&renamed, 200, |l| l.trim_start().starts_with("local"));
    let quoted = replace_line(&renamed, at, |l| format!("\"{l}"));
    edit_and_check(
        &mut pump,
        &mut doc,
        name,
        &quoted,
        at - 10..at + 70,
        "unmatched quote",
    );
    let closed = replace_line(&quoted, at, |l| l[1..].to_owned());
    edit_and_check(
        &mut pump,
        &mut doc,
        name,
        &closed,
        at - 10..at + 70,
        "quote closed",
    );

    // A heredoc's terminator changes: the body runs on until a line says the new one.
    let at = line_index(&closed, 0, |l| l.contains("<<EOF") || l.contains("<<'EOF'"));
    let renamed_tag = replace_line(&closed, at, |l| l.replace("EOF", "EOT"));
    edit_and_check(
        &mut pump,
        &mut doc,
        name,
        &renamed_tag,
        at..at + 80,
        "heredoc terminator",
    );

    // Whole lines in and out.
    let at = 300;
    let mut lines: Vec<&str> = renamed_tag.split_inclusive('\n').collect();
    lines.splice(
        at..at,
        ["if [[ -n \"$x\" ]]; then\n", "  echo \"$x\"\n", "fi\n"],
    );
    let inserted: String = lines.concat();
    edit_and_check(
        &mut pump,
        &mut doc,
        name,
        &inserted,
        at..at + 80,
        "insert lines",
    );
    let mut lines: Vec<&str> = inserted.split_inclusive('\n').collect();
    lines.drain(400..402);
    let deleted: String = lines.concat();
    edit_and_check(
        &mut pump,
        &mut doc,
        name,
        &deleted,
        390..470,
        "delete lines",
    );
    edit_and_check(
        &mut pump,
        &mut doc,
        name,
        &format!("{deleted}echo end"),
        640..710,
        "append a line",
    );
}

#[test]
fn a_markdown_fence_opener_edit_is_exact_once_the_pass_ends() {
    let name = "markdown-50k.md";
    let text = head(name, 400);
    let mut pump = Pump::new(light());
    let mut doc = pump.open(
        Source::new(text.as_str())
            .file_name(name)
            .mode(Mode::Editable),
    );
    all_lines(&mut pump, &mut doc);
    let at = line_index(&text, 0, |l| l.starts_with("```") && l.len() > 3);
    let opener = replace_line(&text, at, |l| format!("`{l}"));
    edit_and_check(
        &mut pump,
        &mut doc,
        name,
        &opener,
        at..at + 80,
        "fence opener",
    );
    let dropped = replace_line(&opener, at, |_| "\n".into());
    edit_and_check(
        &mut pump,
        &mut doc,
        name,
        &dropped,
        at..at + 80,
        "fence opener removed",
    );
}

#[test]
fn quick_edits_before_and_after_the_frontier_settle_exact() {
    let name = "shell-5k.sh";
    let text = head(name, 700);
    let mut pump = Pump::new(light());
    let mut doc = pump.open(
        Source::new(text.as_str())
            .file_name(name)
            .mode(Mode::Editable),
    );
    all_lines(&mut pump, &mut doc);
    // An unmatched quote at 100: run only its foreground, so the pass stops near 210.
    let one = replace_line(&text, 100, |l| format!("\"{l}"));
    doc.edit(one.as_str().into(), LineEdit::diff(&text, &one), 90..170);
    pump.requests();
    pump.step();
    // Then edits before the frontier and after it, while the pass is still running.
    let two = replace_line(&one, 50, |l| format!("# {l}"));
    doc.edit(two.as_str().into(), LineEdit::diff(&one, &two), 40..120);
    pump.requests();
    pump.step();
    let three = replace_line(&two, 500, |l| format!("x=1; {l}"));
    edit_and_check(
        &mut pump,
        &mut doc,
        name,
        &three,
        490..570,
        "three quick edits",
    );
}

#[test]
fn an_edit_re_parses_to_the_viewport_bottom_then_finishes_in_the_background() {
    let name = "shell-5k.sh";
    let text = head(name, 1000);
    let mut pump = Pump::new(light());
    let mut doc = pump.open(
        Source::new(text.as_str())
            .file_name(name)
            .mode(Mode::Editable),
    );
    all_lines(&mut pump, &mut doc);
    let before = pump.parsed(&doc);
    let new = replace_line(&text, 100, |l| format!("\"{l}"));
    doc.edit(new.as_str().into(), LineEdit::diff(&text, &new), 90..170);
    pump.requests();
    pump.step();
    doc.poll();
    let foreground = pump.parsed(&doc) - before;
    assert!(
        foreground <= 170 + OVERSCAN as u64 - 100,
        "foreground parsed {foreground} lines"
    );
    assert_eq!(doc.provisional_from(), Some(100));
    assert!(
        doc.is_provisional(600),
        "a line past the frontier is provisional"
    );
    assert!(!doc.is_provisional(150), "a line in view is exact");
    pump.run();
    doc.poll();
    assert_eq!(doc.provisional_from(), None);
    assert!(!doc.is_provisional(600));
    assert!(
        pump.parsed(&doc) - before > 800,
        "the cascade ran to the end in the background"
    );
}

#[test]
fn an_edit_below_an_unfinished_cascade_is_coloured_at_once_then_made_exact() {
    let name = "shell-5k.sh";
    let text = head(name, 1500);
    let mut pump = Pump::new(light());
    let mut doc = pump.open(
        Source::new(text.as_str())
            .file_name(name)
            .mode(Mode::Editable),
    );
    all_lines(&mut pump, &mut doc);
    let one = replace_line(&text, 100, |l| format!("\"{l}"));
    doc.edit(one.as_str().into(), LineEdit::diff(&text, &one), 90..170);
    pump.requests();
    pump.step();
    let before = pump.parsed(&doc);
    let two = replace_line(&one, 1200, |l| format!("x=1; {l}"));
    doc.edit(two.as_str().into(), LineEdit::diff(&one, &two), 1190..1270);
    pump.requests();
    pump.step();
    doc.poll();
    let foreground = pump.parsed(&doc) - before;
    assert!(
        foreground <= 200,
        "the foreground parsed {foreground} lines, not the cascade"
    );
    assert!(doc.line_spans(1200).is_some() && doc.is_provisional(1200));
    pump.run();
    doc.poll();
    assert!(!doc.is_provisional(1200));
    let got = all_lines(&mut pump, &mut doc);
    assert_lines_eq(
        &got,
        &reference(&two, syntax_for(name, &two), &light(), LONG_LINE),
        "two edits",
    );
}

#[test]
fn a_rename_outside_a_capture_context_converges_within_a_few_lines() {
    let text: String = (0..300)
        .map(|i| format!("run_step {i} --flag value_{i}\n"))
        .collect();
    let mut pump = Pump::new(light());
    let mut doc = pump.open(
        Source::new(text.as_str())
            .file_name("s.sh")
            .mode(Mode::Editable),
    );
    pump.run();
    let before = pump.parsed(&doc);
    let new = replace_line(&text, 150, |l| l.replace("run_step", "go_step"));
    doc.edit(new.as_str().into(), LineEdit::diff(&text, &new), 140..220);
    pump.run();
    let reparsed = pump.parsed(&doc) - before;
    assert!(reparsed <= 8, "the edit re-parsed {reparsed} lines");
    doc.poll();
    let got = all_lines(&mut pump, &mut doc);
    assert_lines_eq(
        &got,
        &reference(&new, syntax_for("s.sh", &new), &light(), LONG_LINE),
        "rename",
    );
}

#[test]
fn a_preview_starts_over_on_an_edit_and_ends_exact() {
    let name = "rust-50k.rs";
    let text = head(name, 300);
    let mut pump = Pump::new(light());
    let mut doc = pump.open(Source::new(text.as_str()).file_name(name));
    all_lines(&mut pump, &mut doc);
    let new = replace_line(&text, 10, |l| format!("/* {l}"));
    doc.edit(new.as_str().into(), LineEdit::diff(&text, &new), 0..80);
    assert!(doc.line_spans(200).is_none(), "the cache started over");
    pump.run();
    doc.poll();
    let got = all_lines(&mut pump, &mut doc);
    assert_lines_eq(
        &got,
        &reference(&new, syntax_for(name, &new), &light(), LONG_LINE),
        "preview edit",
    );
}

#[test]
fn line_edit_diff_finds_the_changed_lines() {
    assert_eq!(
        LineEdit::diff("a\nb\nc", "a\nB\nc"),
        LineEdit {
            first: 1,
            removed: 1,
            inserted: 1
        }
    );
    assert_eq!(
        LineEdit::diff("a\nc", "a\nb\nc"),
        LineEdit {
            first: 1,
            removed: 0,
            inserted: 1
        }
    );
    assert_eq!(
        LineEdit::diff("a\nb\nc", "a\nc"),
        LineEdit {
            first: 1,
            removed: 1,
            inserted: 0
        }
    );
    assert_eq!(
        LineEdit::diff("a\na", "a\na\na"),
        LineEdit {
            first: 2,
            removed: 0,
            inserted: 1
        }
    );
}

// Themes.

#[test]
fn a_theme_switch_recolours_cached_lines_without_re_parsing() {
    let name = "rust-50k.rs";
    let text = head(name, 300);
    let mut pump = Pump::new(light());
    let mut doc = pump.open(Source::new(text.as_str()).file_name(name));
    let before = all_lines(&mut pump, &mut doc);
    let parsed = pump.parsed(&doc);
    pump.set_theme(dark());
    pump.run();
    assert!(doc.poll(), "the palette landed");
    assert_eq!(pump.parsed(&doc), parsed, "no line was parsed again");
    let after: Vec<Runs> = (0..doc.line_count())
        .map(|i| runs_of(&doc.line_spans(i).expect("still cached")))
        .collect();
    assert_lines_eq(
        &after,
        &reference(&text, syntax_for(name, &text), &dark(), LONG_LINE),
        "dark",
    );
    assert_ne!(before, after);
}

// The worker.

#[test]
fn the_newest_viewport_wins() {
    let name = "rust-50k.rs";
    let text = head(name, 1200);
    let mut pump = Pump::new(light());
    let mut doc = pump.open(Source::new(text.as_str()).file_name(name));
    pump.run();
    doc.show(0..10);
    doc.show(500..510);
    doc.show(1000..1010);
    pump.run();
    doc.poll();
    assert!(doc.line_spans(1000).is_some());
    assert!(
        doc.line_spans(0).is_none(),
        "a superseded viewport is skipped"
    );
    assert!(
        doc.line_spans(500).is_none(),
        "a superseded viewport is skipped"
    );
}

#[test]
fn an_edit_cuts_in_on_another_texts_background_pass() {
    let mut pump = Pump::new(light());
    let big = head("markdown-50k.md", 3000);
    let _preview = pump.open(Source::new(big.as_str()).file_name("big.md"));
    let script = "echo one\necho two\n";
    let mut doc = pump.open(Source::new(script).file_name("s.sh").mode(Mode::Editable));
    pump.requests();
    pump.step();
    pump.step();
    let new = "echo one\nprintf two\n";
    doc.edit(new.into(), LineEdit::diff(script, new), 0..2);
    pump.requests();
    pump.step();
    doc.poll();
    assert!(
        doc.line_spans(1).is_some(),
        "the edited line is coloured after one step"
    );
    assert!(
        pump.worker.engine.has_work(),
        "while the preview's pass still runs"
    );
}

#[test]
fn closing_a_text_stops_its_pass() {
    let mut pump = Pump::new(light());
    let big = head("rust-50k.rs", 3000);
    let doc = pump.open(Source::new(big.as_str()).file_name("big.rs"));
    pump.requests();
    pump.step();
    assert!(pump.worker.engine.has_work());
    drop(doc);
    pump.requests();
    assert!(!pump.worker.engine.has_work());
}

// The thread.

/// Waits up to `WAIT` for `cond`: the thread wakes the window just after sending replies, so
/// a wake can land just after `settle` returns.
fn eventually(cond: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + WAIT;
    while !cond() {
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    true
}

#[test]
fn the_thread_wakes_the_window_and_an_idle_text_stays_idle() {
    let wakes = Arc::new(AtomicUsize::new(0));
    let h = Highlighter::new(light());
    let w = wakes.clone();
    h.set_waker(Arc::new(move || {
        w.fetch_add(1, Ordering::SeqCst);
    }));
    let mut doc = h.open(Source::new("fn main() { let x = 1; }\n").file_name("a.rs"));
    doc.show(0..1);
    assert!(doc.settle(WAIT));
    assert!(eventually(|| wakes.load(Ordering::SeqCst) > 0));
    assert!(doc.line_spans(0).is_some_and(|s| !s.is_empty()));
    // The thread wakes the window after sending its replies, so the last wake can land just
    // after `settle` returns.
    std::thread::sleep(Duration::from_millis(50));
    let after = wakes.load(Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(50));
    assert!(!doc.poll(), "nothing more lands");
    assert_eq!(wakes.load(Ordering::SeqCst), after, "no more wakes");
}

#[test]
fn the_thread_starts_on_first_use_and_ends_with_the_last_handle() {
    let h = Highlighter::new(light());
    assert!(!h.join(), "no thread before first use");
    let h = Highlighter::new(light());
    let mut doc = h.open(Source::new("x\n"));
    assert!(doc.settle(WAIT));
    drop(doc);
    assert!(h.join(), "the thread ended");
}

// Adapters.

const BLOCK: &str = "fn main() {\n    let s = \"hi\"; // greet\n    println!(\"{s}\");\n}";

fn block_reference(lang: &str, code: &str, theme: &Theme) -> Runs {
    let syntax = super::detect::by_token(syntaxes().expect("grammars"), lang).expect("grammar");
    let lines = Lines::new(code.into());
    let mut out = Vec::new();
    for (i, runs) in reference(code, syntax, theme, LONG_LINE)
        .into_iter()
        .enumerate()
    {
        let at = lines.start(i);
        for (r, c) in runs {
            out.push((r.start + at..r.end + at, c));
        }
    }
    out
}

fn highlight_runs(h: &[rune_kit::syntax::Highlight]) -> Runs {
    h.iter()
        .map(|(r, p)| match p {
            rune_kit::syntax::Paint::Color(c) => (r.clone(), *c),
            rune_kit::syntax::Paint::Kind(k) => panic!("a kind, not a colour: {k:?}"),
        })
        .collect()
}

#[test]
fn a_markdown_block_answers_plain_then_coloured_once_it_lands() {
    let mut pump = Pump::new(light());
    let mut blocks = pump.blocks();
    let first = blocks.markdown();
    assert!(
        first.highlight(Some("rust"), BLOCK).is_empty(),
        "a miss is plain"
    );
    assert!(
        blocks.highlight(Some("rust"), BLOCK).is_none(),
        "still pending"
    );
    pump.run();
    assert_eq!(pump.blocks_asked, 1, "one request for the same block");
    assert!(blocks.poll());
    let again = blocks.markdown();
    let got = highlight_runs(&again.highlight(Some("rust"), BLOCK));
    let want = block_reference("rust", BLOCK, &light());
    assert!(!want.is_empty());
    assert_eq!(got, want);
    assert!(!blocks.poll(), "nothing more lands");
}

#[test]
fn blocks_with_no_language_or_an_unknown_one_stay_plain() {
    let mut pump = Pump::new(light());
    let mut blocks = pump.blocks();
    assert_eq!(blocks.highlight(None, BLOCK), Some(Vec::new()));
    assert_eq!(blocks.highlight(Some("  "), BLOCK), Some(Vec::new()));
    pump.run();
    assert_eq!(pump.blocks_asked, 0, "no language asks nothing");
    assert!(blocks.highlight(Some("no-such-lang"), BLOCK).is_none());
    pump.run();
    blocks.poll();
    assert_eq!(
        blocks.highlight(Some("no-such-lang"), BLOCK),
        Some(Vec::new())
    );
}

#[test]
fn a_theme_switch_rekeys_and_recolours_blocks() {
    let mut pump = Pump::new(light());
    let mut blocks = pump.blocks();
    blocks.highlight(Some("rust"), BLOCK);
    pump.run();
    blocks.poll();
    let light_runs = highlight_runs(&blocks.highlight(Some("rust"), BLOCK).expect("landed"));
    let before = blocks.markdown();
    assert_eq!(
        highlight_runs(&before.highlight(Some("rust"), BLOCK)),
        light_runs
    );
    pump.set_theme(dark());
    pump.run();
    assert!(blocks.poll());
    let after = blocks.markdown();
    let dark_runs = highlight_runs(&after.highlight(Some("rust"), BLOCK));
    assert_ne!(light_runs, dark_runs);
    assert_eq!(pump.blocks_asked, 1, "no block was parsed again");
    // The old key's cached (light) answer stays apart from the new key's.
    assert_eq!(
        highlight_runs(&before.highlight(Some("rust"), BLOCK)),
        light_runs
    );
}

#[test]
fn wait_on_miss_answers_coloured_on_the_first_call() {
    let h = Highlighter::new(light());
    let mut blocks = h.blocks();
    blocks.wait_on_miss(Some(WAIT));
    let got = highlight_runs(
        &blocks
            .markdown()
            .highlight(Some("c"), "#include <stdio.h>\nint x = 1;"),
    );
    assert!(!got.is_empty());
}

#[test]
fn text_spans_are_offset_into_the_whole_text() {
    let text = "fn a() {}\n\nlet s = \"x\"; // c\n";
    let mut pump = Pump::new(light());
    let mut doc = pump.open(Source::new(text).file_name("a.rs").mode(Mode::Editable));
    let per_line = all_lines(&mut pump, &mut doc);
    let lines = Lines::new(text.into());
    let mut want = Vec::new();
    for (i, runs) in per_line.into_iter().enumerate() {
        want.extend(
            runs.into_iter()
                .map(|(r, c)| (r.start + lines.start(i)..r.end + lines.start(i), c)),
        );
    }
    let got = runs_of(&doc.text_spans(0..doc.line_count()));
    assert_eq!(got, want);
    // Default-coloured text (the variable `s`) gets no span, and touching runs of one colour
    // are joined.
    let s_at = text.find("let s").expect("let") + 4;
    assert!(got.iter().all(|(r, _)| !r.contains(&s_at)));
    assert!(!got.is_empty());
    assert!(
        got.windows(2)
            .all(|w| w[0].0.end != w[1].0.start || w[0].1 != w[1].1)
    );
}

#[test]
fn text_spans_stop_below_runes_span_limit() {
    // About 2,000 coloured numbers a line (the commas between them draw as text), so 40
    // lines hold well over the limit.
    let line = format!("x = [{}]\n", "1,".repeat(2000));
    let text = line.repeat(40);
    let mut pump = Pump::new(light());
    let mut doc = pump.open(
        Source::new(text.as_str())
            .file_name("n.py")
            .mode(Mode::Editable),
    );
    doc.show(0..40);
    pump.run();
    doc.poll();
    assert_eq!(doc.text_spans(0..40).len(), MAX_SPANS);
}

/// K1's done-when, "the syntect themes colour the S4 fixtures": the look's syntect theme
/// (strata-kit's `SCOPES`) gives each fixture's first 200 lines the colours of the roles
/// they hold.
#[test]
fn the_look_theme_colours_the_s4_fixtures_by_role() {
    use strata_kit::theme::syntax::Role;

    let look = strata_kit::theme::bundled_theme("catppuccin-mocha")
        .expect("bundled")
        .look();
    let mut pump = Pump::new(Arc::new(syntect_theme(&look)));
    let mut blocks = pump.blocks();
    let code = [
        (
            "rs",
            "rust-50k.rs",
            &[Role::Comment, Role::String, Role::Keyword, Role::Type][..],
        ),
        (
            "c",
            "c-50k.c",
            &[
                Role::Comment,
                Role::String,
                Role::Keyword,
                Role::Preprocessor,
            ],
        ),
        (
            "js",
            "js-50k.js",
            &[Role::Comment, Role::String, Role::Keyword, Role::Constant],
        ),
        (
            "py",
            "python-50k.py",
            &[Role::Comment, Role::String, Role::Keyword],
        ),
        (
            "sh",
            "shell-5k.sh",
            &[Role::Comment, Role::String, Role::Keyword],
        ),
        ("md", "markdown-50k.md", &[Role::Heading, Role::String]),
    ]
    .map(|(lang, name, roles)| (lang, head(name, 200), roles));
    for (lang, text, _) in &code {
        blocks.highlight(Some(lang), text);
    }
    pump.run();
    blocks.poll();
    let near = |a: Color, b: Color| {
        (a.r - b.r).abs() < 0.01 && (a.g - b.g).abs() < 0.01 && (a.b - b.b).abs() < 0.01
    };
    for (lang, text, roles) in &code {
        let runs = highlight_runs(&blocks.highlight(Some(lang), text).expect("landed"));
        for role in *roles {
            assert!(
                runs.iter().any(|(_, c)| near(*c, role.color(&look))),
                "{lang}: {role:?}"
            );
        }
        assert!(
            !runs.iter().any(|(_, c)| near(*c, Role::Text.color(&look))),
            "{lang}: text-role tokens get no span"
        );
    }
}

#[test]
fn a_waker_set_after_the_thread_starts_is_used() {
    let h = Highlighter::new(light());
    let mut doc = h.open(Source::new("x = 1\n").file_name("a.py"));
    assert!(doc.settle(WAIT));
    let woke = Arc::new(Mutex::new(false));
    let w = woke.clone();
    h.set_waker(Arc::new(move || {
        if let Ok(mut b) = w.lock() {
            *b = true;
        }
    }));
    doc.show(0..1);
    assert!(doc.settle(WAIT));
    assert!(eventually(|| woke.lock().is_ok_and(|b| *b)));
}

/// A grammar the random-edit test runs on, with lines and line prefixes that open and close
/// capture contexts (heredocs, fences, strings, comments).
struct Lang {
    name: &'static str,
    fixture: &'static str,
    lines: &'static [&'static str],
    prefixes: &'static [&'static str],
}

const LANGS: [Lang; 3] = [
    Lang {
        name: "r.sh",
        fixture: "shell-5k.sh",
        lines: &[
            "cat <<EOF\n",
            "EOF\n",
            "x='\n",
            "'\n",
            "echo \"hi\" # x\r\n",
        ],
        prefixes: &["\"", "'", "#", "$(", "`"],
    },
    Lang {
        name: "r.md",
        fixture: "markdown-50k.md",
        lines: &["```rust\n", "```\n", "<!--\n", "-->\n", "    code\n"],
        prefixes: &["`", "<!--", "**", "# "],
    },
    Lang {
        name: "r.rs",
        fixture: "rust-50k.rs",
        lines: &["/*\n", "*/\n", "let s = r#\"\n", "\"#;\n", "fn x() {\r\n"],
        prefixes: &["\"", "/*", "*/", "//", "r#\""],
    },
];

/// Seeds the committed random-edit test runs, per grammar and mode. Some gave wrong lines
/// before the fixes they now guard (review rounds 1 and 2); sweep more locally by running
/// `random_edit_run` over a range.
const RANDOM_RUNS: &[(usize, Mode, u64)] = &[
    (0, Mode::Editable, 1),
    (0, Mode::Editable, 2),
    (1, Mode::Editable, 1),
    (2, Mode::Editable, 1),
    (0, Mode::Preview, 1),
    (2, Mode::Preview, 1),
];

/// The first cached line that isn't provisional and doesn't match a full parse.
fn wrong_exact_line(doc: &CodeDoc, text: &str, name: &str) -> Option<usize> {
    let want = reference(text, syntax_for(name, text), &light(), LONG_LINE);
    (0..want.len()).find(|&i| {
        !doc.is_provisional(i) && doc.line_spans(i).is_some_and(|s| runs_of(&s) != want[i])
    })
}

/// One seed of random edits on `lang` in `mode`: up to three queued before any work,
/// viewports in between, then a random amount of work, sometimes the whole pass with the host
/// not polling before the next edit. After every poll, no line marked exact may be wrong;
/// every few rounds the pass runs out and every line must match a full parse.
fn random_edit_run(seed: u64, lang: &Lang, mode: Mode) -> Result<(), String> {
    let mut text = head(lang.fixture, 250);
    let mut pump = Pump::new(light());
    let mut doc = pump.open(Source::new(text.as_str()).file_name(lang.name).mode(mode));
    all_lines(&mut pump, &mut doc);
    let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    let mut rand = |n: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % n.max(1) as u64) as usize
    };
    for round in 0..30 {
        let at_round =
            |what: &str, line: usize| format!("round {round}, {what}: line {line} wrong");
        for _ in 0..1 + rand(3) {
            let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_owned).collect();
            let at = rand(lines.len());
            match rand(8) {
                0 | 1 => lines[at].insert_str(0, lang.prefixes[rand(lang.prefixes.len())]),
                2 => {
                    let p = lang.prefixes[rand(lang.prefixes.len())];
                    lines[at] = lines[at].replacen(p, "", 1);
                }
                3 | 4 => lines.insert(at, lang.lines[rand(lang.lines.len())].into()),
                5 if lines.len() > 10 => {
                    let end = (at + 1 + rand(3)).min(lines.len());
                    lines.drain(at..end);
                }
                6 => lines[at] = format!("\"{}\n", "x".repeat(LONG_LINE + rand(64) - 20)),
                _ => lines[at] = lines[at].replacen(' ', "\"", 1),
            }
            let new: String = lines.concat();
            let line = at.min(lines.len().saturating_sub(1));
            let visible = if rand(2) == 0 {
                line.saturating_sub(20)..line + 60
            } else {
                let v = rand(lines.len());
                v..v + 60
            };
            doc.edit(new.as_str().into(), LineEdit::diff(&text, &new), visible);
            text = new;
            if rand(3) == 0 {
                let v = rand(doc.line_count());
                doc.show(v..v + 60);
            }
        }
        match rand(4) {
            // The pass runs to its end and the host doesn't poll before the next edit.
            0 => pump.run(),
            _ => {
                pump.requests();
                for _ in 0..rand(4) {
                    pump.step();
                    if rand(2) == 0 {
                        doc.poll();
                        if let Some(i) = wrong_exact_line(&doc, &text, lang.name) {
                            return Err(at_round("mid-pass", i));
                        }
                    }
                }
                doc.poll();
                if let Some(i) = wrong_exact_line(&doc, &text, lang.name) {
                    return Err(at_round("after a poll", i));
                }
            }
        }
        if round % 5 == 4 {
            pump.run();
            doc.poll();
            if let Some(i) = wrong_exact_line(&doc, &text, lang.name) {
                return Err(at_round("after the pass", i));
            }
            if doc.provisional_from().is_some() {
                return Err(format!("round {round}: provisional after the pass"));
            }
            let got = all_lines(&mut pump, &mut doc);
            let want = reference(&text, syntax_for(lang.name, &text), &light(), LONG_LINE);
            if let Some(i) = (0..want.len()).find(|&i| got[i] != want[i]) {
                return Err(at_round("all lines", i));
            }
        }
    }
    Ok(())
}

#[test]
fn random_queued_edits_and_viewports_end_exact() {
    let wrong: Vec<String> = RANDOM_RUNS
        .iter()
        .filter_map(|&(lang, mode, seed)| {
            let lang = &LANGS[lang];
            random_edit_run(seed, lang, mode)
                .err()
                .map(|e| format!("{} {mode:?} seed {seed}: {e}", lang.name))
        })
        .collect();
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// Lines that open a string, one after another: each edit flips everything below it.
fn quote_script(n: usize) -> String {
    (0..n)
        .map(|i| format!("x={i} && run_it --flag {i}\n"))
        .collect()
}

#[test]
fn queued_edits_before_any_work_settle_exact() {
    let text = quote_script(400);
    let mut pump = Pump::new(light());
    let mut doc = pump.open(
        Source::new(text.as_str())
            .file_name("p.sh")
            .mode(Mode::Editable),
    );
    all_lines(&mut pump, &mut doc);
    let one = replace_line(&text, 10, |l| format!("\"{l}"));
    doc.edit(one.as_str().into(), LineEdit::diff(&text, &one), 0..60);
    // The second quote closes the first's string: line 101 on is code again.
    let two = replace_line(&one, 100, |l| format!("\"{l}"));
    edit_and_check(&mut pump, &mut doc, "p.sh", &two, 100..140, "queued quotes");
}

#[test]
fn an_edit_before_the_host_saw_a_pass_end_still_drops_that_pass_stale_lines() {
    let text = quote_script(300);
    let mut pump = Pump::new(light());
    let mut doc = pump.open(
        Source::new(text.as_str())
            .file_name("p.sh")
            .mode(Mode::Editable),
    );
    all_lines(&mut pump, &mut doc);
    // Edit 1 opens a string at line 10: every line below changes. Its pass ends and sends
    // `Settled`, but the host doesn't poll before the next edit.
    let one = replace_line(&text, 10, |l| format!("\"{l}"));
    doc.edit(one.as_str().into(), LineEdit::diff(&text, &one), 0..60);
    pump.run();
    // Edit 2, a keystroke that changes no state, converges at once.
    let two = replace_line(&one, 200, |l| l.replace("--flag", "--flab"));
    doc.edit(two.as_str().into(), LineEdit::diff(&one, &two), 190..250);
    pump.run();
    doc.poll();
    assert_eq!(doc.provisional_from(), None);
    assert_eq!(wrong_exact_line(&doc, &two, "p.sh"), None);
    let got = all_lines(&mut pump, &mut doc);
    assert_lines_eq(
        &got,
        &reference(&two, syntax_for("p.sh", &two), &light(), LONG_LINE),
        "two",
    );
}

#[test]
fn an_insert_above_a_pass_the_host_never_saw_end_drops_its_stale_lines() {
    // Pass 1 runs to the end (or converges mid-text) unseen; edit 2 inserts lines above it
    // and converges at once: the lines the insert shifted under the old `from` are stale.
    let text = quote_script(300);
    for (k, at, closes) in [
        (5, 0, false),
        (5, 0, true),
        (20, 3, false),
        (1, 50, false),
        (40, 200, false),
    ] {
        let mut pump = Pump::new(light());
        let mut doc = pump.open(
            Source::new(text.as_str())
                .file_name("p.sh")
                .mode(Mode::Editable),
        );
        all_lines(&mut pump, &mut doc);
        let mut one = replace_line(&text, 10, |l| format!("\"{l}"));
        if closes {
            one = replace_line(&one, 150, |l| format!("\"{l}"));
        }
        doc.edit(one.as_str().into(), LineEdit::diff(&text, &one), 0..60);
        pump.run();
        let mut lines: Vec<String> = one.split_inclusive('\n').map(str::to_owned).collect();
        lines.splice(at..at, (0..k).map(|i| format!("y{i}=1\n")));
        let two = lines.concat();
        doc.edit(two.as_str().into(), LineEdit::diff(&one, &two), 0..60);
        pump.run();
        doc.poll();
        assert_eq!(
            wrong_exact_line(&doc, &two, "p.sh"),
            None,
            "{k} lines at {at}, closes {closes}"
        );
    }
}

#[test]
fn an_insert_above_an_unseen_pass_end_then_forgotten_still_drops_its_stale_lines() {
    // As above, then 300 keystrokes push the shifting edit out of the edit history.
    let text = quote_script(300);
    let mut pump = Pump::new(light());
    let mut doc = pump.open(
        Source::new(text.as_str())
            .file_name("p.sh")
            .mode(Mode::Editable),
    );
    all_lines(&mut pump, &mut doc);
    let one = replace_line(&text, 10, |l| format!("\"{l}"));
    doc.edit(one.as_str().into(), LineEdit::diff(&text, &one), 0..60);
    pump.run();
    let mut now: String = (0..5).map(|i| format!("y{i}=1\n")).collect::<String>() + &one;
    doc.edit(now.as_str().into(), LineEdit::diff(&one, &now), 0..60);
    for i in 0..300 {
        let next = replace_line(&now, 3, |l| {
            let l = l.trim_end();
            if i % 2 == 0 {
                format!("{l}1\n")
            } else {
                format!("{}\n", &l[..l.len() - 1])
            }
        });
        doc.edit(next.as_str().into(), LineEdit::diff(&now, &next), 0..60);
        now = next;
    }
    assert_eq!(
        doc.history.len(),
        EDIT_HISTORY,
        "the shifting edit was forgotten"
    );
    pump.run();
    doc.poll();
    assert_eq!(wrong_exact_line(&doc, &now, "p.sh"), None);
}

#[test]
fn an_edit_below_an_unfinished_cascade_settles_exact() {
    let text = quote_script(1500);
    let mut pump = Pump::new(light());
    let mut doc = pump.open(
        Source::new(text.as_str())
            .file_name("p.sh")
            .mode(Mode::Editable),
    );
    all_lines(&mut pump, &mut doc);
    let one = replace_line(&text, 100, |l| format!("\"{l}"));
    doc.edit(one.as_str().into(), LineEdit::diff(&text, &one), 90..170);
    pump.requests();
    pump.step();
    let two = replace_line(&one, 1200, |l| format!("\"{l}"));
    edit_and_check(
        &mut pump,
        &mut doc,
        "p.sh",
        &two,
        1190..1270,
        "below the frontier",
    );
}

#[test]
fn a_viewport_across_an_unreached_edit_settles_exact() {
    let text = quote_script(1500);
    let mut pump = Pump::new(light());
    let mut doc = pump.open(
        Source::new(text.as_str())
            .file_name("p.sh")
            .mode(Mode::Editable),
    );
    all_lines(&mut pump, &mut doc);
    let one = replace_line(&text, 100, |l| format!("\"{l}"));
    doc.edit(one.as_str().into(), LineEdit::diff(&text, &one), 90..170);
    pump.requests();
    pump.step();
    // A second edit far below, with its viewport elsewhere, then a viewport across it.
    let two = replace_line(&one, 1200, |l| format!("\"{l}"));
    doc.edit(two.as_str().into(), LineEdit::diff(&one, &two), 1000..1010);
    pump.requests();
    pump.step();
    doc.poll();
    doc.show(1150..1230);
    pump.requests();
    pump.step();
    doc.poll();
    pump.run();
    doc.poll();
    assert_eq!(doc.provisional_from(), None);
    let got = all_lines(&mut pump, &mut doc);
    assert_lines_eq(
        &got,
        &reference(&two, syntax_for("p.sh", &two), &light(), LONG_LINE),
        "viewport across",
    );
}

#[test]
fn lines_in_view_are_never_evicted() {
    let name = "rust-50k.rs";
    let text = head(name, 12000);
    let mut pump = Pump::new(light());
    let mut doc = pump.open(Source::new(text.as_str()).file_name(name));
    pump.run();
    doc.show(2000..2080);
    pump.run();
    doc.poll();
    assert!(doc.line_spans(2000).is_some());
    // Scroll far, then straight back before the far lines land: the view is cached, so
    // nothing is asked, but it is still the view.
    doc.show(6000..10500);
    doc.show(2000..2080);
    pump.run();
    doc.poll();
    assert!(doc.line_spans(2000).is_some(), "a line in view was evicted");
}

#[test]
fn a_viewport_evicted_while_on_its_way_is_asked_for_again() {
    let name = "rust-50k.rs";
    let text = head(name, 12000);
    let mut pump = Pump::new(light());
    let mut doc = pump.open(Source::new(text.as_str()).file_name(name));
    pump.run();
    // 4,000 lines cached around B.
    doc.show(6000..9960);
    pump.run();
    doc.poll();
    // Ask for A, then scroll back to B before A lands: B is cached, so nothing is asked.
    doc.show(100..180);
    pump.requests();
    doc.show(8000..8080);
    pump.run();
    doc.poll();
    assert!(
        doc.line_spans(100).is_none(),
        "A landed, then went with the lines far from B"
    );
    // Back to A: the request that brought it is the last one, but its lines are gone.
    doc.show(100..180);
    pump.run();
    doc.poll();
    assert!(doc.line_spans(100).is_some(), "A was never asked for again");
}

#[test]
fn show_does_not_ask_again_for_lines_on_their_way() {
    let name = "rust-50k.rs";
    let text = head(name, 3000);
    let mut pump = Pump::new(light());
    let mut doc = pump.open(Source::new(text.as_str()).file_name(name));
    // The pass hasn't reached the view: its lines land as the pass passes them.
    doc.show(1000..1080);
    pump.requests();
    let asked = pump.shows_asked;
    while doc.line_spans(1119).is_none() {
        assert!(pump.step());
        doc.poll();
        // The host asks on every build; part of the view has landed.
        doc.show(1000..1080);
        pump.requests();
    }
    assert_eq!(
        pump.shows_asked, asked,
        "the shrinking rest of the view was asked again"
    );
}

#[test]
fn a_page_of_blocks_over_the_cache_size_is_asked_for_once() {
    let mut pump = Pump::new(light());
    let mut blocks = pump.blocks();
    // 160 blocks of about 34 KiB, over the 4 MiB cache. Their lines are over the long-line
    // cap, so the thread skips them and the test stays quick.
    let codes: Vec<String> = (0..160)
        .map(|i| {
            format!(
                "// block {i}\n{}\n{}",
                "x".repeat(17_000),
                "y".repeat(17_000)
            )
        })
        .collect();
    let mut asked = Vec::new();
    for _build in 0..4 {
        let before = pump.blocks_asked;
        for c in &codes {
            blocks.highlight(Some("rust"), c);
        }
        pump.run();
        blocks.poll();
        asked.push(pump.blocks_asked - before);
    }
    assert_eq!(asked, [160, 0, 0, 0], "blocks asked per build");
    assert!(
        codes
            .iter()
            .all(|c| blocks.highlight(Some("rust"), c).is_some())
    );
}

/// A block of about 17 KiB, over the long-line cap so the thread skips it quickly.
fn big_block(i: usize) -> String {
    format!("// block {i}\n{}\n", "x".repeat(17_000))
}

#[test]
fn the_block_cache_trims_to_its_cap_at_each_poll_and_stays_under_its_ceiling_without() {
    const CAP: usize = 4 << 20;
    const CEILING: usize = 16 << 20;
    // A build per block, each landing polled: the cache trims to the cap.
    let mut pump = Pump::new(light());
    let mut blocks = pump.blocks();
    for i in 0..400 {
        blocks.markdown().highlight(Some("rust"), &big_block(i));
        pump.run();
        blocks.poll();
    }
    assert!(
        blocks.cached_bytes() <= CAP,
        "{} bytes",
        blocks.cached_bytes()
    );
    // A host that asks and never polls: no trims, but the ceiling holds, and blocks past it
    // are plain rather than pending.
    let mut pump = Pump::new(light());
    let blocks = pump.blocks();
    for i in 0..1200 {
        blocks.markdown().highlight(Some("rust"), &big_block(i));
        pump.run();
    }
    assert!(
        blocks.cached_bytes() <= CEILING,
        "{} bytes",
        blocks.cached_bytes()
    );
    assert_eq!(
        blocks.highlight(Some("rust"), &big_block(5000)),
        Some(Vec::new())
    );
}

#[test]
fn the_block_cache_stays_under_its_ceiling_when_misses_wait() {
    // Waiting hosts (headless runs, goldens) never poll, so never trim.
    let h = Highlighter::new(light());
    let mut blocks = h.blocks();
    blocks.wait_on_miss(Some(WAIT));
    for i in 0..1200 {
        assert!(blocks.highlight(Some("rust"), &big_block(i)).is_some());
    }
    assert!(
        blocks.cached_bytes() <= 16 << 20,
        "{} bytes",
        blocks.cached_bytes()
    );
}

#[test]
fn a_build_over_the_ceiling_comes_to_rest() {
    // A host that rebuilds whenever `poll` says colours changed, over one document whose
    // blocks (each under rune-kit's 32 KiB cut) total more than the 16 MiB ceiling.
    let mut pump = Pump::new(light());
    let mut blocks = pump.blocks();
    let codes: Vec<String> = (0..600)
        .map(|i| format!("// b {i}\n{}\n", "q".repeat(30_000)))
        .collect();
    let mut asked = Vec::new();
    for _build in 0..12 {
        let before = pump.blocks_asked;
        let _ = blocks.markdown();
        for c in &codes {
            blocks.highlight(Some("rust"), c);
        }
        pump.run();
        asked.push(pump.blocks_asked - before);
        if !blocks.poll() {
            break;
        }
    }
    assert!(
        asked.len() <= 3,
        "builds never came to rest: blocks asked per build {asked:?}"
    );
    assert!(
        asked[1..].iter().all(|&n| n == 0),
        "blocks asked again: {asked:?}"
    );
    // Blocks past the ceiling answer plain, not pending.
    assert!(
        codes
            .iter()
            .all(|c| blocks.highlight(Some("rust"), c).is_some())
    );
    assert!(
        blocks.cached_bytes() <= 16 << 20,
        "{} bytes",
        blocks.cached_bytes()
    );
}

#[test]
fn a_block_is_coloured_up_to_the_32_kib_cut_and_asked_once() {
    // C11 calls `highlight` directly, with no cut of its own.
    let mut pump = Pump::new(light());
    let mut blocks = pump.blocks();
    let line = "let x = 1; // c\n";
    let code = line.repeat((17 << 20) / line.len());
    let mut answers = Vec::new();
    for _build in 0..3 {
        let _ = blocks.markdown();
        answers.push(blocks.highlight(Some("rust"), &code).map(|h| h.len()));
        pump.run();
        blocks.poll();
    }
    assert_eq!(pump.blocks_asked, 1, "answers {answers:?}");
    assert!(
        answers[1].is_some_and(|n| n > 0),
        "coloured once landed: {answers:?}"
    );
    let cut = 32 * 1024;
    let h = blocks.highlight(Some("rust"), &code).expect("landed");
    assert!(
        h.iter().all(|(r, _)| r.end <= cut),
        "colours stop at the 32 KiB cut"
    );
    // The cut code, plus the entry's own 96 bytes.
    assert!(blocks.cached_bytes() <= cut + 96);
}

/// A block of about 30 KB, under rune-kit's 32 KiB cut: a comment line the thread colours,
/// then one line past the long-line cap, which it skips, so the tests stay quick.
fn view_block(tag: &str, i: usize) -> String {
    format!("// {tag} {i}\n{}\n", "q".repeat(30_000))
}

/// One view's build through rune-kit's own hook and its cache: how many blocks drew plain.
fn build_view(blocks: &CodeBlocks, codes: &[String]) -> usize {
    let hook = blocks.markdown();
    codes
        .iter()
        .filter(|c| hook.highlight(Some("rust"), c).is_empty())
        .count()
}

/// Builds `views` each frame, as a host does, until `poll` has nothing new. The number of
/// builds and the blocks that drew plain in the last one.
fn build_until_rest(
    pump: &mut Pump,
    blocks: &mut CodeBlocks,
    views: &[&[String]],
) -> (usize, usize) {
    for build in 1..=8 {
        let plain = views.iter().map(|v| build_view(blocks, v)).sum();
        pump.run();
        if !blocks.poll() {
            return (build, plain);
        }
    }
    (usize::MAX, usize::MAX)
}

#[test]
fn two_markdown_views_on_one_pool_over_the_cache_come_to_rest() {
    // Two views, each with its own hook from `markdown()`, 3 MB each: 6 MB on screen.
    let mut pump = Pump::new(light());
    let mut blocks = pump.blocks();
    let a: Vec<String> = (0..100).map(|i| view_block("a", i)).collect();
    let b: Vec<String> = (0..100).map(|i| view_block("b", i)).collect();
    let (builds, plain) = build_until_rest(&mut pump, &mut blocks, &[&a, &b]);
    assert!(builds <= 3, "builds until rest: {builds}");
    assert_eq!(plain, 0, "blocks still plain at rest");
    assert_eq!(pump.blocks_asked, 200, "each block asked for once");
}

#[test]
fn an_edit_to_one_block_of_a_large_document_asks_for_that_block_only() {
    let mut pump = Pump::new(light());
    let mut blocks = pump.blocks();
    let mut doc: Vec<String> = (0..200).map(|i| view_block("d", i)).collect();
    let (builds, _) = build_until_rest(&mut pump, &mut blocks, &[&doc]);
    assert!(builds <= 3);
    let asked = pump.blocks_asked;
    for _keystroke in 0..3 {
        doc[0].push_str("let y = 2;\n");
        for _build in 0..3 {
            // In document order, as `Markdown` asks: the edited block first.
            let hook = blocks.markdown();
            hook.highlight(Some("rust"), &doc[0]);
            let plain = doc[1..]
                .iter()
                .filter(|c| hook.highlight(Some("rust"), c).is_empty())
                .count();
            assert_eq!(plain, 0, "the other blocks stay coloured");
            pump.run();
            if !blocks.poll() {
                break;
            }
        }
    }
    assert_eq!(
        pump.blocks_asked - asked,
        3,
        "one block asked per keystroke"
    );
}

#[test]
fn switching_back_to_a_large_document_keeps_its_blocks_until_asked() {
    let mut pump = Pump::new(light());
    let mut blocks = pump.blocks();
    let x: Vec<String> = (0..200).map(|i| view_block("x", i)).collect();
    let y: Vec<String> = (0..10).map(|i| view_block("y", i)).collect();
    build_until_rest(&mut pump, &mut blocks, &[&x]);
    build_until_rest(&mut pump, &mut blocks, &[&y]);
    let cached = x.iter().filter(|c| blocks.has_landed("rust", c)).count();
    let asked = pump.blocks_asked;
    let (builds, plain) = build_until_rest(&mut pump, &mut blocks, &[&x]);
    assert!(builds <= 3 && plain == 0, "builds {builds}, plain {plain}");
    assert_eq!(
        pump.blocks_asked - asked,
        200 - cached,
        "a cached block was asked for again"
    );
}

#[test]
fn the_thread_drops_block_jobs_the_cache_has_dropped() {
    let mut pump = Pump::new(light());
    let mut blocks = pump.blocks();
    let pool = blocks.pool_id();
    let small = |i: usize| format!("fn s{i}() {{}}\n");
    // Two small blocks, then a 4.5 MB document, all still queued.
    blocks.highlight(Some("rust"), &small(1));
    blocks.highlight(Some("rust"), &small(2));
    let doc: Vec<String> = (0..150).map(|i| view_block("d", i)).collect();
    for c in &doc {
        blocks.highlight(Some("rust"), c);
    }
    // The host moves on: a small block lands, then another, with new small blocks asked.
    for i in 3..6 {
        pump.requests();
        pump.step();
        blocks.poll();
        blocks.highlight(Some("rust"), &small(i));
        pump.requests();
        let pending = blocks.pending_keys();
        let queued = pump.worker.engine.queued_blocks(pool);
        let stray = queued.iter().filter(|k| !pending.contains(k)).count();
        assert_eq!(
            stray, 0,
            "{stray} jobs for blocks the cache dropped are still queued"
        );
    }
}

#[test]
fn a_theme_that_lands_during_a_wait_reaches_runes_cache() {
    // Headless runs wait on misses and never poll: a theme switch landing during a wait must
    // still change the hook's key, or rune-kit keeps the old theme's answers.
    let h = Highlighter::new(light());
    let mut blocks = h.blocks();
    blocks.wait_on_miss(Some(WAIT));
    let before = blocks.markdown().highlight(Some("rust"), BLOCK);
    assert!(!before.is_empty());
    h.set_theme(dark());
    blocks.markdown().highlight(Some("rust"), "fn other() {}");
    let after = blocks.markdown().highlight(Some("rust"), BLOCK);
    assert_ne!(
        highlight_runs(&before),
        highlight_runs(&after),
        "the old theme's colours"
    );
}

#[test]
fn blocks_without_a_thread_are_plain() {
    let blocks = CodeBlocks::new(None);
    assert_eq!(blocks.highlight(Some("rust"), BLOCK), Some(Vec::new()));
}

#[test]
fn without_grammars_everything_is_plain_and_nothing_waits() {
    let mut pump = Pump::new(light());
    pump.worker.engine = super::engine::Engine::with_syntaxes(light(), Options::default(), None);
    let mut doc = pump.open(Source::new("fn main() {}\n").file_name("a.rs"));
    let mut blocks = pump.blocks();
    assert!(blocks.highlight(Some("rust"), BLOCK).is_none());
    pump.run();
    doc.poll();
    assert!(
        doc.is_idle(),
        "a text that can't be coloured doesn't look pending"
    );
    assert!(doc.settle(Duration::ZERO));
    assert!(doc.line_spans(0).is_none(), "plain");
    assert!(blocks.settle(Duration::ZERO), "no block looks pending");
    assert_eq!(blocks.highlight(Some("rust"), BLOCK), Some(Vec::new()));
}

#[test]
fn a_failed_text_is_opened_again_on_the_next_edit() {
    let mut pump = Pump::new(light());
    pump.worker.engine = super::engine::Engine::with_syntaxes(light(), Options::default(), None);
    let text = "fn main() {}\n";
    let mut doc = pump.open(Source::new(text).file_name("a.rs").mode(Mode::Editable));
    pump.run();
    doc.poll();
    assert!(doc.is_idle() && doc.line_spans(0).is_none());
    // The grammars are back (a stand-in for whatever failed): the next edit tries again.
    pump.worker.engine = super::engine::Engine::new(light(), Options::default());
    let new = "fn main() { let x = 1; }\n";
    doc.edit(new.into(), LineEdit::diff(text, new), 0..1);
    assert!(!doc.is_idle());
    let got = all_lines(&mut pump, &mut doc);
    assert_lines_eq(
        &got,
        &reference(new, syntax_for("a.rs", new), &light(), LONG_LINE),
        "retried",
    );
    assert!(doc.settle(Duration::ZERO));
}

/// Every block under one key: what a crafted document gets against a fixed hash key.
fn one_key(_: &str, _: &str) -> u64 {
    7
}

#[test]
fn two_blocks_with_one_key_on_screen_come_to_rest() {
    // Review 5, SF1: each asked for the other's entry, so every landing replaced the other
    // and asked again, and a host rebuilding on `poll` never came to rest.
    let mut pump = Pump::new(light());
    let mut blocks = pump.blocks();
    blocks.key_with(one_key);
    let codes = ["fn a() {}\n".to_string(), "fn b() {}\n".to_string()];
    let (builds, _) = build_until_rest(&mut pump, &mut blocks, &[&codes]);
    assert!(builds <= 3, "builds until rest: {builds}");
    assert!(
        pump.blocks_asked <= 2,
        "blocks asked: {}",
        pump.blocks_asked
    );
    // The first is coloured; the one that lost the key draws plain rather than pending.
    assert!(blocks.has_landed("rust", &codes[0]));
    assert_eq!(blocks.highlight(Some("rust"), &codes[1]), Some(Vec::new()));
}

/// Blocks keyed by their first byte, so `a…` blocks share a key and `c…` ones another.
fn first_byte_key(_: &str, code: &str) -> u64 {
    u64::from(code.as_bytes()[0])
}

#[test]
fn a_reply_for_a_block_that_lost_its_key_is_ignored() {
    // Review 5, SF4: the reply carries the code's `Arc`, which tells a stale reply apart.
    let mut pump = Pump::new(light());
    let mut blocks = pump.blocks();
    blocks.key_with(first_byte_key);
    let (a, b, c) = ("a_one(1);\n", "a_two(\"s\");\n", "c();\n");
    // One build asks for `c` and `a`; `c` lands and bumps the epoch, then `a` finishes too,
    // its reply still on the channel.
    blocks.highlight(Some("rust"), c);
    blocks.highlight(Some("rust"), a);
    pump.requests();
    pump.step();
    assert!(blocks.poll(), "c landed");
    pump.step();
    // The next build asks for `b` only: `a` wasn't asked since the bump, so `b` takes its key.
    assert_eq!(blocks.highlight(Some("rust"), b), None);
    blocks.poll();
    assert_eq!(
        blocks.highlight(Some("rust"), b),
        None,
        "b took a's colours from a's late reply"
    );
    pump.run();
    blocks.poll();
    let got = blocks.highlight(Some("rust"), b).expect("b landed");
    assert_eq!(highlight_runs(&got), block_reference("rust", b, &light()));
}

/// Builds `view` each frame, with `polls` landings polled before each build, until a build
/// lands nothing. The builds, and whether it came to rest within `max`.
fn build_polling(
    pump: &mut Pump,
    blocks: &mut CodeBlocks,
    view: &[String],
    polls: usize,
    max: usize,
) -> (usize, bool) {
    for build in 1..=max {
        build_view(blocks, view);
        pump.requests();
        let mut changed = false;
        for _ in 0..polls {
            pump.step();
            pump.step();
            changed |= blocks.poll();
        }
        if !changed {
            pump.run();
            if !blocks.poll() {
                return (build, true);
            }
        }
    }
    (max, false)
}

#[test]
fn three_polls_between_builds_over_the_cap_come_to_rest() {
    // Review 5, SF2: Rune's resting wake polls, then the frame polls, then the view builds.
    // Each poll that landed something moved the build marks, so the blocks the last build
    // asked for fell out of the last two builds and were trimmed while on screen.
    let mut pump = Pump::new(light());
    let mut blocks = pump.blocks();
    let view: Vec<String> = (0..200).map(|i| view_block("a", i)).collect();
    let (builds, rested) = build_polling(&mut pump, &mut blocks, &view, 3, 300);
    assert!(rested, "never came to rest in {builds} builds");
    assert_eq!(pump.blocks_asked, 200, "each block asked for once");
}

#[test]
fn two_polls_between_builds_over_the_cap_come_to_rest() {
    // Review 5, SF3: the last-two-builds rule holds with two polls per build.
    let mut pump = Pump::new(light());
    let mut blocks = pump.blocks();
    let view: Vec<String> = (0..200).map(|i| view_block("a", i)).collect();
    let (builds, rested) = build_polling(&mut pump, &mut blocks, &view, 2, 300);
    assert!(rested, "never came to rest in {builds} builds");
    assert_eq!(pump.blocks_asked, 200, "each block asked for once");
}

#[test]
fn a_block_from_the_last_build_outlives_one_trim() {
    // Review 5, SF3: a block asked for in either of the last two builds is never trimmed,
    // so a document left for one build keeps its colours; the next trim takes it.
    let mut pump = Pump::new(light());
    let mut blocks = pump.blocks();
    let x: Vec<String> = (0..160).map(|i| view_block("x", i)).collect();
    let small = |i: usize| vec![format!("fn s{i}() {{}}\n")];
    build_view(&blocks, &x);
    pump.run();
    assert!(blocks.poll());
    // A build of something else, landing: x was asked for in the build before, so it stays.
    build_view(&blocks, &small(1));
    pump.run();
    assert!(blocks.poll());
    assert!(
        x.iter().all(|c| blocks.has_landed("rust", c)),
        "x was trimmed one build after it was on screen"
    );
    // Another: x is two builds back now, and the trim takes it down to the cap.
    build_view(&blocks, &small(2));
    pump.run();
    assert!(blocks.poll());
    let kept = x.iter().filter(|c| blocks.has_landed("rust", c)).count();
    assert!(kept < x.len(), "x outlived its two builds: {kept} kept");
    assert!(
        blocks.cached_bytes() <= 4 << 20,
        "{} bytes",
        blocks.cached_bytes()
    );
}

/// At most `8 n log2 n` entries looked at: what sorting once and keeping running totals cost.
fn n_log_n_bound(n: usize) -> u64 {
    8 * n as u64 * u64::from(usize::BITS - n.leading_zeros())
}

#[test]
fn misses_and_trims_cost_n_log_n() {
    // Review 5, SF5: each miss summed the current build's bytes over every entry, and each
    // trim looked for the oldest entry once per entry it dropped, both on the UI thread.
    let n = 5000;
    let mut pump = Pump::new(light());
    let mut blocks = pump.blocks();
    // 5000 blocks of about 1 KiB, over the 4 MiB cap, in a language the thread answers at
    // once.
    let codes: Vec<String> = (0..n)
        .map(|i| format!("// {i}\n{}\n", "x".repeat(1024)))
        .collect();
    for c in &codes {
        blocks.highlight(Some("zzz"), c);
    }
    let misses = blocks.visits();
    pump.run();
    blocks.poll();
    // Two builds of something else: the second trim drops the old document to the cap.
    for k in 0..2 {
        blocks.highlight(Some("zzz"), &format!("g({k});\n"));
        pump.run();
        blocks.poll();
    }
    let trims = blocks.visits() - misses;
    assert!(blocks.cached_bytes() <= 4 << 20, "trimmed to the cap");
    let bound = n_log_n_bound(n);
    assert!(
        misses <= bound,
        "{n} misses looked at {misses} entries (bound {bound})"
    );
    assert!(
        trims <= bound,
        "the trims looked at {trims} entries (bound {bound})"
    );
}

#[test]
fn tiny_blocks_count_their_entries_against_the_cap() {
    // Review 5, SF6: the cap counted code bytes only, so 50,000 blocks of a few bytes stayed
    // under it whatever their entries, tokens and keys cost. Each entry now counts 96 bytes
    // more, as rune-kit's hook cache does.
    let mut pump = Pump::new(light());
    let mut blocks = pump.blocks();
    for i in 0..50_000 {
        blocks.highlight(Some("zzz"), &format!("f({i});\n"));
    }
    pump.run();
    blocks.poll();
    for k in 0..2 {
        blocks.highlight(Some("zzz"), &format!("g({k});\n"));
        pump.run();
        blocks.poll();
    }
    let most = (4 << 20) / 96;
    assert!(
        blocks.entry_count() <= most,
        "{} entries kept, at most {most} fit the cap",
        blocks.entry_count()
    );
}

#[test]
fn a_build_past_the_ceiling_evicts_the_old_document_in_n_log_n() {
    // Review 1 of C11 on SF5: the first test stayed under the 16 MiB ceiling, so the miss
    // path's eviction (and its running total) went untested. Here an old document of about
    // 15 MB from an earlier build is still cached when a new build of about 12 MB arrives:
    // its misses go past the ceiling and must evict the old blocks, never refuse new ones,
    // and stay n log n however many misses there are.
    let mut pump = Pump::new(light());
    let mut blocks = pump.blocks();
    let old: Vec<String> = (0..500)
        .map(|i| format!("// old {i}\n{}\n", "x".repeat(30_000)))
        .collect();
    for c in &old {
        blocks.highlight(Some("zzz"), c);
    }
    pump.run();
    assert!(blocks.poll(), "the old document landed");
    let (visits, asked) = (blocks.visits(), pump.blocks_asked);
    let new: Vec<String> = (0..1500)
        .map(|i| format!("// new {i}\n{}\n", "y".repeat(8_000)))
        .collect();
    let mut refused = 0;
    for c in &new {
        if blocks.highlight(Some("zzz"), c).is_some() {
            refused += 1;
        }
        let (running, counted) = blocks.current_check();
        assert_eq!(running, counted, "the running total drifted");
    }
    pump.requests();
    assert_eq!(
        refused, 0,
        "new blocks answered plain instead of evicting old ones"
    );
    assert_eq!(pump.blocks_asked - asked, new.len());
    assert!(
        blocks.cached_bytes() <= 16 << 20,
        "{} bytes",
        blocks.cached_bytes()
    );
    let looked = blocks.visits() - visits;
    let bound = n_log_n_bound(old.len() + new.len());
    assert!(
        looked <= bound,
        "the misses looked at {looked} entries (bound {bound})"
    );
}

#[test]
fn each_pool_hashes_with_its_own_key() {
    // SF1: a fixed hash key let a crafted document collide its blocks in every pool.
    let pump = Pump::new(light());
    let (a, b) = (pump.blocks(), pump.blocks());
    assert_ne!(
        a.key_for("rust", "fn main() {}"),
        b.key_for("rust", "fn main() {}"),
        "two pools keyed a block alike"
    );
}
