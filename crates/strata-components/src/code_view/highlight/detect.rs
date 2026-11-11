//! Picking a grammar for a text, with no I/O.
//!
//! syntect's `find_syntax_for_file` opens the file to read its first line, so this follows its
//! steps on data instead: the file name, then its extension, then the text's first line (for
//! shebangs and the modelines grammars declare). Then come Vim and Emacs modelines, which
//! syntect doesn't read, and last plain text.

use std::path::Path;
use std::sync::OnceLock;

use syntect::parsing::{SyntaxReference, SyntaxSet};

/// The grammar set, loaded once per process on the first highlighter thread that needs it
/// (S4: `extra_newlines`, so lines are fed with their `\n`). It is never freed. `None` when
/// loading it panicked: every text and block is then plain.
pub(crate) fn syntaxes() -> Option<&'static SyntaxSet> {
    static SET: OnceLock<Option<SyntaxSet>> = OnceLock::new();
    SET.get_or_init(|| std::panic::catch_unwind(two_face::syntax::extra_newlines).ok())
        .as_ref()
}

/// How many lines at each end of a text Vim reads modelines from (its `modelines` default).
const MODELINE_LINES: usize = 5;

/// The grammar for a text: `explicit` (a fence token, a grammar name, or the host's own guess
/// such as one from a MIME type), else by `file_name`, else from the text, else plain text.
pub(crate) fn detect<'a>(
    set: &'a SyntaxSet,
    explicit: Option<&str>,
    file_name: Option<&str>,
    text: &str,
) -> &'a SyntaxReference {
    explicit
        .and_then(|t| by_token(set, t))
        .or_else(|| file_name.and_then(|n| by_file_name(set, n)))
        .or_else(|| set.find_syntax_by_first_line(first_line(text)))
        .or_else(|| by_modeline(set, text))
        .unwrap_or_else(|| set.find_syntax_plain_text())
}

/// A fence's language or a grammar name: an extension or token first (`rs`, `bash`), then a
/// grammar's name in any case (`Rust`, `bourne again shell (bash)`).
pub(crate) fn by_token<'a>(set: &'a SyntaxSet, token: &str) -> Option<&'a SyntaxReference> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    set.find_syntax_by_token(token)
}

/// syntect's file steps: the whole file name (`Makefile`, `PKGBUILD`, `.bashrc`), then the
/// extension.
fn by_file_name<'a>(set: &'a SyntaxSet, name: &str) -> Option<&'a SyntaxReference> {
    let path = Path::new(name);
    let file = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let ext = path.extension().and_then(|x| x.to_str()).unwrap_or("");
    [file, ext]
        .into_iter()
        .filter(|s| !s.is_empty())
        .find_map(|s| set.find_syntax_by_extension(s))
}

fn first_line(text: &str) -> &str {
    text.split_inclusive('\n').next().unwrap_or("")
}

/// A Vim modeline in the first or last five lines, or an Emacs one in the first two.
fn by_modeline<'a>(set: &'a SyntaxSet, text: &str) -> Option<&'a SyntaxReference> {
    let lines: Vec<&str> = text.lines().collect();
    let head = &lines[..lines.len().min(MODELINE_LINES)];
    let tail = &lines[lines.len().saturating_sub(MODELINE_LINES)..];
    let vim = head.iter().chain(tail).find_map(|l| vim_filetype(l));
    let emacs = || lines.iter().take(2).find_map(|l| emacs_mode(l));
    let name = vim.or_else(emacs)?;
    by_token(set, name).or_else(|| by_token(set, name.strip_suffix("-mode")?))
}

/// The filetype a Vim modeline sets: `vim: set ft=rust:`, `vi: filetype=python`,
/// `ex: ts=4 ft=sh`. Vim wants the marker at the start of the line or after white space.
/// A dotted filetype (`c.doxygen`) gives its first part.
pub(crate) fn vim_filetype(line: &str) -> Option<&str> {
    let mut rest = line;
    let options = loop {
        let at = ["vim:", "vi:", "ex:"]
            .iter()
            .filter_map(|m| rest.find(m).map(|i| (i, m.len())))
            .min()?;
        let (i, len) = at;
        let after_space = rest[..i]
            .chars()
            .next_back()
            .is_none_or(char::is_whitespace);
        if after_space {
            break &rest[i + len..];
        }
        rest = &rest[i + len..];
    };
    let options = options.trim_start();
    let options = options
        .strip_prefix("set ")
        .or_else(|| options.strip_prefix("se "))
        .unwrap_or(options);
    options
        .split(|c: char| c == ':' || c.is_whitespace())
        .find_map(|o| {
            o.strip_prefix("ft=")
                .or_else(|| o.strip_prefix("filetype="))
        })
        .map(|ft| ft.split('.').next().unwrap_or(ft))
        .filter(|ft| !ft.is_empty())
}

/// The mode an Emacs first-line modeline names: `-*- mode: ruby -*-`, `-*- c++ -*-`, or
/// `-*- mode: python; coding: utf-8 -*-`.
pub(crate) fn emacs_mode(line: &str) -> Option<&str> {
    let start = line.find("-*-")? + 3;
    let len = line[start..].find("-*-")?;
    let inner = line[start..start + len].trim();
    if !inner.contains(':') {
        return Some(inner).filter(|m| !m.is_empty());
    }
    inner.split(';').find_map(|kv| {
        let (k, v) = kv.split_once(':')?;
        k.trim()
            .eq_ignore_ascii_case("mode")
            .then(|| v.trim())
            .filter(|m| !m.is_empty())
    })
}
