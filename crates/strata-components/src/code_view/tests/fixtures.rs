// SPDX-License-Identifier: MIT

use std::fmt::Write;
use std::sync::OnceLock;

const WORDS: &[&str] = &[
    "acorn", "amber", "anchor", "apple", "archive", "arrow", "atlas", "badge", "balance", "bamboo",
    "barrel", "beacon", "berry", "border", "branch", "breeze", "bridge", "bucket", "buffer",
    "bundle", "button", "cabin", "cable", "cactus", "camera", "candle", "canvas", "canyon",
    "carbon", "carpet", "castle", "cedar", "center", "chalk", "channel", "cherry", "circle",
    "client", "clover", "cobalt", "collar", "column", "comet", "copper", "corner", "cotton",
    "cradle", "crater", "crystal", "cursor", "desert", "detail", "device", "digest", "domain",
    "dragon", "drawer", "driver", "eagle", "editor", "ember", "engine", "entry", "fabric",
];

const VERBS: &[&str] = &[
    "build", "check", "clear", "close", "collect", "compute", "convert", "count", "create",
    "decode", "encode", "fetch", "filter", "find", "flush", "format", "gather", "handle", "insert",
    "load", "measure", "merge", "parse", "place", "prepare", "publish", "query", "reduce",
    "refresh", "render", "reset", "resolve", "restore", "rotate", "route", "sample", "scan",
    "search", "select", "shift", "sort", "split", "store", "sync", "trace", "update", "verify",
];

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        ((self.next() >> 11) % n as u64) as usize
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    fn word(&mut self) -> &'static str {
        self.pick(WORDS)
    }

    fn verb(&mut self) -> &'static str {
        self.pick(VERBS)
    }

    fn num(&mut self) -> u32 {
        *self.pick(&[0, 1, 2, 4, 8, 16, 32, 64, 128, 256, 1024, 4096])
    }

    fn pascal(&mut self) -> String {
        let mut out = String::new();
        for _ in 0..2 {
            let w = self.word();
            let mut chars = w.chars();
            if let Some(first) = chars.next() {
                out.push(first.to_ascii_uppercase());
                out.push_str(chars.as_str());
            }
        }
        out
    }
}

fn finish(mut lines: Vec<String>, total: usize, filler: &str) -> String {
    lines.truncate(total);
    while lines.len() < total {
        lines.push(filler.to_owned());
    }
    let cap: usize = lines.iter().map(|l| l.len() + 1).sum();
    let mut out = String::with_capacity(cap);
    for line in &lines {
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

fn push_lines(out: &mut Vec<String>, block: &str) {
    out.extend(block.lines().map(str::to_owned));
}

fn generate_rust(total: usize, seed: u64) -> String {
    let mut r = Rng::new(seed);
    let mut out = vec![
        "//! Synthetic Rust for syntax-highlighting tests.".into(),
        String::new(),
        "#![allow(dead_code, unused_variables)]".into(),
        String::new(),
    ];
    while out.len() < total {
        let ty = r.pascal();
        let a = r.word();
        let b = r.word();
        let v = r.verb();
        let n = r.num();
        let block = match r.below(4) {
            0 => format!(
                "/// Holds `{a}` and `{b}` state.\n\
                 #[derive(Debug, Clone, PartialEq, Eq)]\n\
                 pub struct {ty}<'a> {{\n\
                 \x20   pub {a}: &'a str,\n\
                 \x20   pub {b}: Vec<u8>,\n\
                 \x20   pub limit: Option<u32>,\n\
                 }}\n\n"
            ),
            1 => format!(
                "/*\n\
                 \x20* Handles {a} updates.\n\
                 \x20* /* nested {b} note */ end of note.\n\
                 \x20*/\n\
                 pub fn {v}_{a}(input: &[u8], {b}: u64) -> Result<u64, &'static str> {{\n\
                 \x20   // Compute a weighted sum for {a}.\n\
                 \x20   let label = r#\"{a} \"quoted\" {b}\"#;\n\
                 \x20   let raw = b\"{b}\\n\\t\";\n\
                 \x20   let mut total: u64 = {b}.saturating_add({n});\n\
                 \x20   for (index, byte) in input.iter().enumerate() {{\n\
                 \x20       total += (*byte as u64) + index as u64;\n\
                 \x20   }}\n\
                 \x20   if total == 0 && !label.is_empty() && !raw.is_empty() {{\n\
                 \x20       return Err(\"{a} failed\");\n\
                 \x20   }}\n\
                 \x20   Ok(total)\n\
                 }}\n\n"
            ),
            2 => format!(
                "#[derive(Debug, Clone, Copy)]\n\
                 pub enum {ty} {{\n\
                 \x20   Empty,\n\
                 \x20   Count(u32),\n\
                 \x20   Named {{ {a}: i64, {b}: bool }},\n\
                 }}\n\
                 pub const MAX_{}: u64 = 0x{:02x}_u64 + {n};\n\n",
                a.to_ascii_uppercase(),
                n & 0xff,
            ),
            _ => format!(
                "impl<'a> {ty}<'a> {{\n\
                 \x20   pub fn {v}_{b}(&self) -> usize {{\n\
                 \x20       match self.limit {{\n\
                 \x20           Some(v) if v > {n} => self.{b}.len() + v as usize,\n\
                 \x20           Some(0) => 0,\n\
                 \x20           _ => self.{a}.len(),\n\
                 \x20       }}\n\
                 \x20   }}\n\
                 }}\n\n"
            ),
        };
        push_lines(&mut out, &block);
    }
    finish(out, total, "// trailing rust filler")
}

fn generate_c(total: usize, seed: u64) -> String {
    let mut r = Rng::new(seed);
    let mut out = vec![
        "/* Synthetic C for syntax-highlighting tests. */".into(),
        String::new(),
        "#include <stdbool.h>".into(),
        "#include <stdint.h>".into(),
        "#include <stdio.h>".into(),
        "#include <stdlib.h>".into(),
        String::new(),
    ];
    while out.len() < total {
        let a = r.word();
        let b = r.word();
        let v = r.verb();
        let n = r.num();
        let upper = a.to_ascii_uppercase();
        let block = match r.below(3) {
            0 => format!(
                "#ifndef {upper}_H\n\
                 #define {upper}_H ({n}u << 2)\n\
                 #define {upper}_LABEL \"{a}: %d\\n\"\n\
                 #endif /* {upper}_H */\n\n"
            ),
            1 => format!(
                "typedef struct {a}_{b} {{\n\
                 \x20   size_t {a};\n\
                 \x20   uint32_t {b};\n\
                 \x20   const char *label;\n\
                 }} {a}_{b}_t;\n\n"
            ),
            _ => format!(
                "/**\n\
                 \x20* Process {a} values.\n\
                 \x20*/\n\
                 static uint32_t {v}_{a}(const uint8_t *{a}, size_t {b}) {{\n\
                 \x20   // Walk every entry in {a}.\n\
                 \x20   uint32_t total = 0x{:04x}u;\n\
                 \x20   for (size_t i = 0; i < {b}; ++i) {{\n\
                 \x20       total += (uint32_t){a}[i] + {n};\n\
                 \x20   }}\n\
                 \x20   if ({a} == NULL) {{\n\
                 \x20       fprintf(stderr, \"{a} missing: %zu\\n\", {b});\n\
                 \x20       return 0;\n\
                 \x20   }}\n\
                 \x20   return total;\n\
                 }}\n\n",
                n & 0xffff,
            ),
        };
        push_lines(&mut out, &block);
    }
    finish(out, total, "/* trailing c filler */")
}

fn long_js_line(r: &mut Rng) -> String {
    let mut line = format!("export const {}List = [", r.word());
    while line.len() < 2050 {
        let _ = write!(line, "'{}', {}, ", r.word(), r.num());
    }
    line.push_str("null];");
    line
}

fn generate_js(total: usize, seed: u64) -> String {
    let mut r = Rng::new(seed);
    let mut out = vec![
        "// Synthetic JavaScript for syntax-highlighting tests.".into(),
        String::new(),
        "'use strict';".into(),
        String::new(),
    ];
    while out.len() < total {
        let ty = r.pascal();
        let a = r.word();
        let b = r.word();
        let v = r.verb();
        let n = r.num();
        let upper = a.to_ascii_uppercase();
        let block = match r.below(3) {
            0 => format!(
                "const {upper} = Object.freeze({{\n\
                 \x20 {a}: '{a} {b}',\n\
                 \x20 count: {n},\n\
                 \x20 pattern: /^[{a}]+-\\d+$/i,\n\
                 \x20 flags: [true, false, null],\n\
                 }});\n\n"
            ),
            1 => format!(
                "/**\n\
                 \x20* Build a {a} summary.\n\
                 \x20*/\n\
                 export async function {v}{ty}({a}, {b} = {n}) {{\n\
                 \x20 // Combine entries and format a template string.\n\
                 \x20 const note = `\n\
                 \x20   <section class=\"{a}\">\n\
                 \x20     ${{{a} ?? '{b}'}} (${{{b} + 1}})\n\
                 \x20   </section>`;\n\
                 \x20 try {{\n\
                 \x20   return note.replace(/\\s+/g, ' ');\n\
                 \x20 }} catch (error) {{\n\
                 \x20   throw new Error(`{a}: ${{error.message}}`);\n\
                 \x20 }}\n\
                 }}\n\n"
            ),
            _ => format!(
                "export class {ty} {{\n\
                 \x20 constructor({a}, {b}) {{\n\
                 \x20   this.{a} = {a} ?? {n};\n\
                 \x20   this.{b} = {b} === undefined ? null : {b};\n\
                 \x20 }}\n\
                 \x20 {v}() {{\n\
                 \x20   return [this.{a}, this.{b}].filter(Boolean);\n\
                 \x20 }}\n\
                 }}\n\n"
            ),
        };
        push_lines(&mut out, &block);
    }
    out.truncate(total);
    for quarter in 1..=3 {
        let idx = quarter * total / 4;
        if idx < out.len() {
            out[idx] = long_js_line(&mut r);
        }
    }
    finish(out, total, "// trailing js filler")
}

fn generate_python(total: usize, seed: u64) -> String {
    let mut r = Rng::new(seed);
    let mut out = vec![
        "\"\"\"Synthetic Python for syntax-highlighting tests.\"\"\"".into(),
        String::new(),
        "from __future__ import annotations".into(),
        "import dataclasses".into(),
        String::new(),
    ];
    while out.len() < total {
        let ty = r.pascal();
        let a = r.word();
        let b = r.word();
        let v = r.verb();
        let n = r.num();
        let upper = a.to_ascii_uppercase();
        let block = match r.below(3) {
            0 => format!(
                "@dataclasses.dataclass(frozen=True)\n\
                 class {ty}:\n\
                 \x20   \"\"\"Represents {a} and {b}.\"\"\"\n\
                 \x20   {a}: str = '{a}'\n\
                 \x20   {b}: int = {n}\n\n"
            ),
            1 => format!(
                "async def {v}_{a}({a}: list[int], {b}: int = {n}) -> int:\n\
                 \x20   \"\"\"Compute {a} totals.\n\n\
                 \x20   Uses {b} as the baseline.\n\
                 \x20   \"\"\"\n\
                 \x20   # Filter and sum positive entries.\n\
                 \x20   values = [x * {b} for x in {a} if x % 2 == 0]\n\
                 \x20   label = f\"{a} {{{b}!r}} and {{len(values)}}\"\n\
                 \x20   if not label:\n\
                 \x20       raise ValueError(r'\\d+-{a}')\n\
                 \x20   return sum(values)\n\n"
            ),
            _ => format!(
                "{upper} = \"\"\"\n\
                 {a} {b} multiline block\n\
                 \"\"\"\n\
                 {upper}_MAP: dict[str, tuple[int, int]] = {{\n\
                 \x20   '{a}': ({n}, 0x{:02x}),\n\
                 }}\n\n",
                n & 0xff,
            ),
        };
        push_lines(&mut out, &block);
    }
    finish(out, total, "# trailing python filler")
}

fn generate_shell(total: usize, seed: u64) -> String {
    let mut r = Rng::new(seed);
    let mut out = vec![
        "#!/usr/bin/env bash".into(),
        "# Synthetic shell script for syntax-highlighting tests.".into(),
        "set -euo pipefail".into(),
        String::new(),
    ];
    while out.len() < total {
        let a = r.word();
        let b = r.word();
        let v = r.verb();
        let n = r.num();
        // Keep double quotes balanced on every line so prepending a quote at any code line
        // cascades cleanly across the rest of the script.
        let pick = if out.len() >= 90 && out.len() <= 110 {
            0
        } else {
            r.below(6)
        };
        let block = match pick {
            0..=2 => format!(
                "{v}_{a}() {{\n\
                 \x20   # Prepare {a} and log progress.\n\
                 \x20   local {a}=\"${{{b}:-{a}}}\"\n\
                 \x20   local count=$(( {n} + 1 ))\n\
                 \x20   echo \"{a}: ${{{a}}} ($count)\"\n\
                 \x20   if [[ -n \"${{{a}}}\" ]]; then\n\
                 \x20       printf '%s: %d\\n' '{b}' \"$count\"\n\
                 \x20   fi\n\
                 }}\n\n"
            ),
            3..=4 => format!(
                "{v}_{b}() {{\n\
                 \x20   local {b}=\"$(printf '%s' '{a}' | tr -d '\\n')\"\n\
                 \x20   case \"${{{b}}}\" in\n\
                 \x20       {a}|{b})\n\
                 \x20           echo \"matched {a}: ${{{b}}}\"\n\
                 \x20           ;;\n\
                 \x20       *)\n\
                 \x20           echo \"other {b}: ${{{b}}}\" >&2\n\
                 \x20           ;;\n\
                 \x20   esac\n\
                 }}\n\n"
            ),
            _ => format!(
                "write_{a}() {{\n\
                 \x20   local {a}='{b}'\n\
                 \x20   cat <<EOF\n\
                 {a} value is ${{{a}}}\n\
                 EOF\n\
                 \x20   cat <<'EOF' > \"/dev/null\"\n\
                 {b} literal $not_expanded\n\
                 EOF\n\
                 }}\n\n"
            ),
        };
        push_lines(&mut out, &block);
    }
    finish(out, total, "true")
}

fn markdown_fence(lang: &str, a: &str, b: &str, v: &str, n: u32) -> String {
    match lang {
        "rust" => format!(
            "```rust\n\
             pub fn {v}_{a}(x: u64) -> u64 {{\n\
             \x20   let msg = \"{a} {b}\";\n\
             \x20   x + msg.len() as u64 + {n}\n\
             }}\n\
             ```\n\n"
        ),
        "c" => format!(
            "```c\n\
             #define LIMIT {n}\n\
             static int {v}_{a}(void) {{ return LIMIT; }}\n\
             ```\n\n"
        ),
        "js" => format!(
            "```js\n\
             export const {a} = '{b}';\n\
             export const count = {n};\n\
             ```\n\n"
        ),
        "python" => format!(
            "```python\n\
             def {v}_{a}() -> str:\n\
             \x20   return \"{a} {b}\"\n\
             ```\n\n"
        ),
        "sh" => format!(
            "```sh\n\
             local {a}=\"{b}\"\n\
             echo \"${{{a}}}\"\n\
             ```\n\n"
        ),
        "json" => format!(
            "```json\n\
             {{\n\
             \x20 \"{a}\": \"{b}\",\n\
             \x20 \"count\": {n}\n\
             }}\n\
             ```\n\n"
        ),
        _ => format!(
            "```yaml\n\
             {a}:\n\
             \x20 {b}: {n}\n\
             ```\n\n"
        ),
    }
}

fn generate_markdown(total: usize, seed: u64) -> String {
    let mut r = Rng::new(seed);
    let mut out = vec![
        "# Synthetic Markdown for syntax-highlighting tests".into(),
        String::new(),
        "Generated in memory for highlighter tests.".into(),
        String::new(),
    ];
    let langs = ["rust", "c", "js", "python", "sh", "json", "yaml"];
    let mut block_idx = 0usize;
    while out.len() < total {
        let a = r.word();
        let b = r.word();
        let v = r.verb();
        let n = r.num();
        let block = match block_idx % 4 {
            0 => markdown_fence(langs[(block_idx / 4) % langs.len()], a, b, v, n),
            1 => format!(
                "## {a} and {b}\n\n\
                 Paragraph with **{a}**, *{b}*, `{v}()` and [{a}](https://example.com/{a}/{b}).\n\n"
            ),
            2 => format!(
                "- First {a} item with `{b}`\n\
                 - Second {b} item with {n}\n\n\
                 > Quoted note about {a} and {b}.\n\n"
            ),
            _ => format!(
                "| {a} | {b} | count |\n\
                 | --- | :---: | ---: |\n\
                 | `{v}` | {b} | {n} |\n\n"
            ),
        };
        block_idx += 1;
        push_lines(&mut out, &block);
    }
    finish(out, total, "")
}

pub(crate) fn fixture(name: &str) -> &'static str {
    static RUST: OnceLock<String> = OnceLock::new();
    static C_SRC: OnceLock<String> = OnceLock::new();
    static JS: OnceLock<String> = OnceLock::new();
    static PYTHON: OnceLock<String> = OnceLock::new();
    static MARKDOWN: OnceLock<String> = OnceLock::new();
    static SHELL: OnceLock<String> = OnceLock::new();

    match name {
        "rust-50k.rs" => RUST.get_or_init(|| generate_rust(50_000, 1)),
        "c-50k.c" => C_SRC.get_or_init(|| generate_c(50_000, 2)),
        "js-50k.js" => JS.get_or_init(|| generate_js(50_000, 3)),
        "python-50k.py" => PYTHON.get_or_init(|| generate_python(50_000, 4)),
        "markdown-50k.md" => MARKDOWN.get_or_init(|| generate_markdown(50_000, 5)),
        "shell-5k.sh" => SHELL.get_or_init(|| generate_shell(5_000, 6)),
        other => panic!("unknown code fixture: {other}"),
    }
}
