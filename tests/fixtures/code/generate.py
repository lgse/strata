#!/usr/bin/env python3
"""Generate the syntax-highlighting perf fixtures in this directory.

The fixtures are synthetic: every line comes from the templates below, filled in by a
fixed-seed PRNG, so the output is identical on every machine and Python version. No third-party
code is copied. See README.md for what each file exercises.

    python3 tests/fixtures/code/generate.py           # rewrite the fixtures
    python3 tests/fixtures/code/generate.py --check   # fail if a fixture differs
"""

import argparse
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
MASK = (1 << 64) - 1


class Rng:
    """xorshift64*: small, fast and the same everywhere (unlike `random`, which may change)."""

    def __init__(self, seed):
        self.state = (seed & MASK) or 0x9E3779B97F4A7C15

    def next(self):
        x = self.state
        x ^= x >> 12
        x ^= (x << 25) & MASK
        x ^= x >> 27
        self.state = x
        return (x * 0x2545F4914F6CDD1D) & MASK

    def below(self, n):
        return (self.next() >> 11) % n

    def between(self, lo, hi):
        return lo + self.below(hi - lo + 1)

    def choice(self, seq):
        return seq[self.below(len(seq))]

    def chance(self, percent):
        return self.below(100) < percent


# Real English words that are not keywords or builtins in any fixture language, so identifiers
# read naturally and the spell checker passes.
WORDS = """
able account acorn active actor adapter address advice agent album alert almond amber anchor
angle animal answer anvil apple april arch archive area arrow artist aspect atlas attic author
autumn avenue badge baker balance ballot bamboo banner barrel basket beacon beaker berry bishop
blanket blossom border bottle branch breeze bridge bright broker bucket budget buffer builder
bundle burden butter button cabin cable cactus camera campus candle canvas canyon carbon cargo
carpet carrot castle cattle cellar cement center chalk chamber channel chapter charge cherry
circle citizen clerk client climate clover coast cobalt coffee collar column comet comfort
copper corner cotton county cradle crater credit crystal culture current cursor cushion custom
dancer debate decade desert detail device diamond digest dinner doctor domain donkey dragon
drawer driver eagle editor effort elbow ember empire engine entry envoy era estate evening
fabric falcon family farmer feather fellow fence ferry fiber figure finger forest fossil
fountain frame freight frost galaxy garage garden garlic gather gentle giant ginger glacier
global golden gossip grain granite gravel harbor harvest helmet herald hollow honey horizon
hunter island ivory jacket jungle kernel kettle kitchen ladder lantern laser leader lemon
lesson letter level lizard locker lumber magnet mammal manner marble margin market meadow
medal member mentor merchant meteor method middle mirror model monkey mosaic motion mountain
museum nation needle nephew network noodle novel number ocean office olive onion orbit orchard
outlet oxygen paddle palace panel parcel parent pastry pebble pencil pepper picnic pilot pistol
planet pocket poem portal potato powder prairie prism profit puzzle quarry rabbit radar ranger
rapid raven reader record region ribbon ridge rocket saddle salmon sample satin scholar season
second sector seller shadow shelter signal silver singer sketch socket soldier sparrow spider
spirit spring squad stable station steel stream studio summit sunset surface tablet tailor
talent target temple tender thunder ticket timber token tower trader travel treaty tunnel
turtle valley vapor velvet vendor vessel violet visitor volume voyage wagon walnut weaver
window winter wizard yellow zebra
""".split()

VERBS = """
build check clear close collect compute convert count create decode encode extend fetch filter
find flush format gather handle insert load measure merge parse place prepare publish push
query reduce refresh render reset resolve restore rotate route sample scan schedule search
select send shift sort split store submit sync trace update validate verify wrap
""".split()

ADJECTIVES = """
active blue brave calm clever cold dark eager early fair fast gentle golden green happy heavy
honest kind large late light little loud lucky narrow noble old pale plain proud quick quiet
rapid rare rich rough round sharp short silent simple slow small smooth soft solid strong sweet
swift tall thin tiny warm wide wild wise young
""".split()


def lines_of(text):
    return text.split("\n")


class Gen:
    """Shared word helpers; one subclass per language."""

    def __init__(self, seed):
        self.r = Rng(seed)

    def w(self):
        # A small pool keeps the committed fixtures compressible; the highlighter's cost does
        # not depend on which words appear.
        return WORDS[self.r.below(64) * len(WORDS) // 64]

    def v(self):
        return self.r.choice(VERBS)

    def adj(self):
        return self.r.choice(ADJECTIVES)

    def snake(self, n=None):
        n = n or self.r.between(1, 3)
        parts = [self.w() for _ in range(n)]
        return "_".join(parts)

    def fn_name(self):
        return self.v() + "_" + self.snake(self.r.between(1, 2))

    def camel(self, n=None):
        n = n or self.r.between(1, 3)
        parts = [self.v()] + [self.w().capitalize() for _ in range(n - 1)]
        return "".join(parts)

    def var(self):
        return self.w() + (self.w().capitalize() if self.r.chance(50) else "")

    def pascal(self, n=None):
        n = n or self.r.between(1, 3)
        return "".join(self.w().capitalize() for _ in range(n))

    def upper(self):
        return self.snake(self.r.between(1, 3)).upper()

    def sentence(self, lo=4, hi=12):
        words = []
        for i in range(self.r.between(lo, hi)):
            pick = self.r.below(10)
            if pick < 6:
                words.append(self.w())
            elif pick < 8:
                words.append(self.adj())
            else:
                words.append(self.v())
        words[0] = words[0].capitalize()
        return " ".join(words)

    def num(self):
        return str(self.r.choice([0, 1, 2, 3, 4, 8, 10, 16, 32, 64, 100, 255, 1024, 4096]))

    def block(self):
        raise NotImplementedError

    def header(self):
        return []

    def filler(self):
        raise NotImplementedError

    def build(self, total):
        out = self.header()
        while True:
            chunk = self.block()
            if len(out) + len(chunk) > total:
                break
            out.extend(chunk)
        while len(out) < total:
            out.append(self.filler())
        return out


# --------------------------------------------------------------------------------------------
# Rust


class Rust(Gen):
    TYPES = ["u8", "u16", "u32", "u64", "usize", "i32", "i64", "f32", "f64", "bool", "char",
             "String", "&'a str", "Vec<u8>", "Option<u32>", "Box<[u8]>"]

    def ty(self):
        return self.r.choice(self.TYPES)

    def header(self):
        return [
            "//! Synthetic Rust for syntax-highlighting benchmarks. Generated by generate.py.",
            "",
            "#![allow(dead_code, unused_variables)]",
            "",
        ]

    def filler(self):
        return "// " + self.sentence() if self.r.chance(50) else ""

    def expr(self):
        a, b = self.snake(1), self.snake(1)
        return self.r.choice([
            f"{a}.len() + {self.num()}",
            f"{a}.iter().map(|x| x * {self.num()}).sum::<u64>()",
            f"{a}.saturating_sub({b})",
            f"({a} as f64 * 1.5e3) as u64",
            f"{a}.unwrap_or_default()",
            f"{a}.get({self.num()}).copied().unwrap_or(0)",
            f"if {a} > {b} {{ {a} }} else {{ {b} }}",
            f"{a}.checked_mul({b}).ok_or(Error::{self.pascal(1)})?",
            f"{self.pascal(1)}::{self.v()}(&{a})",
            f"0x{self.r.below(256):02x}_u8 as u32 | {a}",
        ])

    def string(self):
        s = self.sentence(2, 6)
        return self.r.choice([
            f'"{s}"',
            f'"{s}\\n\\t{{}}"',
            f'r#"{s} "quoted" text"#',
            f'b"{s.lower()}"',
            f'"{s} \\u{{1F600}}"',
        ])

    def stmt(self, indent):
        p = " " * indent
        a = self.snake(1)
        return self.r.choice([
            [f"{p}let {a} = {self.expr()};"],
            [f"{p}let mut {a}: {self.ty().replace(chr(39) + 'a ', '')} = Default::default();"],
            [f"{p}// {self.sentence()}"],
            [f'{p}log::{self.r.choice(["debug", "info", "warn"])}!({self.string()}, {a});'],
            [f"{p}{a}.push({self.expr()});"],
            [f"{p}for (index, {a}) in {self.snake(1)}.iter().enumerate() {{",
             f"{p}    total += {a}.{self.v()}(index)?;",
             f"{p}}}"],
            [f"{p}while let Some({a}) = queue.pop_front() {{",
             f"{p}    if {a}.is_empty() {{",
             f"{p}        continue;",
             f"{p}    }}",
             f"{p}    seen.insert({a}.clone());",
             f"{p}}}"],
            [f"{p}let {a} = {self.snake(1)}",
             f"{p}    .iter()",
             f"{p}    .filter(|&&x| x % {self.r.between(2, 9)} == 0)",
             f"{p}    .map(|x| x.to_string())",
             f"{p}    .collect::<Vec<_>>();"],
            [f"{p}assert!({a} <= {self.num()}, {self.string()});"],
            [f"{p}let {a} = format!(\"{{}}-{{:>8}}-{{:#x}}\", {self.snake(1)}, {self.snake(1)}, {self.num()});"],
        ])

    def match_block(self, indent):
        p = " " * indent
        enum = self.pascal(1)
        out = [f"{p}match {self.snake(1)} {{"]
        for _ in range(self.r.between(2, 5)):
            arm = self.r.choice([
                f"{enum}::{self.pascal(1)} => {self.expr()},",
                f"{enum}::{self.pascal(1)}({self.snake(1)}) if {self.snake(1)} > {self.num()} => {{",
                f"{enum}::{self.pascal(1)} {{ {self.snake(1)}, .. }} => return Err(Error::{self.pascal(1)}),",
                f"Some(ref {self.snake(1)}) => {self.expr()},",
                f"{self.r.between(0, 9)}..={self.r.between(10, 99)} => {self.string()}.len() as u64,",
            ])
            out.append(f"{p}    {arm}")
            if arm.endswith("{"):
                out.append(f"{p}        {self.expr()}")
                out.append(f"{p}    }}")
        out.append(f"{p}    _ => unreachable!(\"{self.sentence(2, 5)}\"),")
        out.append(f"{p}}}")
        return out

    def body(self, indent, n):
        out = []
        for _ in range(n):
            if self.r.chance(15):
                out.extend(self.match_block(indent))
            else:
                out.extend(self.stmt(indent))
        return out

    def doc(self, indent, example=False):
        p = " " * indent
        out = [f"{p}/// {self.sentence()}."]
        if self.r.chance(50):
            out += [f"{p}///", f"{p}/// {self.sentence(8, 16)}, see [`{self.pascal(1)}`]."]
        if example:
            out += [f"{p}///", f"{p}/// # Examples", f"{p}///", f"{p}/// ```",
                    f"{p}/// let {self.snake(1)} = {self.pascal(2)}::new({self.num()});",
                    f"{p}/// assert!({self.snake(1)}.is_ok());", f"{p}/// ```"]
        return out

    def fn(self, indent, method):
        p = " " * indent
        name = self.fn_name()
        recv = self.r.choice(["&self, ", "&mut self, ", "self, ", ""]) if method else ""
        params = ", ".join(f"{self.snake(1)}: {self.ty()}" for _ in range(self.r.between(0, 3)))
        if "'a" in params and not method:
            gen = "<'a>"
        else:
            gen = self.r.choice(["", "", "<U: Into<u64>>", "<const N: usize>"])
        ret = self.r.choice(["", " -> u64", " -> Result<(), Error>", " -> Option<&str>", " -> impl Iterator<Item = u32> + '_"])
        asyn = "async " if self.r.chance(15) else ""
        vis = self.r.choice(["pub ", "pub(crate) ", ""])
        out = self.doc(indent) if self.r.chance(60) else []
        if self.r.chance(25):
            out.append(f"{p}#[{self.r.choice(['inline', 'must_use', 'cold', 'track_caller'])}]")
        out.append(f"{p}{vis}{asyn}fn {name}{gen}({recv}{params}){ret}")
        if self.r.chance(20):
            out.append(f"{p}where")
            out.append(f"{p}    Self: Sized,")
        out[-1] += " {"
        out.extend(self.body(indent + 4, self.r.between(2, 9)))
        if asyn:
            out.append(f"{p}    {self.snake(1)}.{self.v()}().await?;")
        out.append(f"{p}    todo!()" if self.r.chance(5) else f"{p}    Default::default()")
        out.append(f"{p}}}")
        return out

    def block(self):
        kind = self.r.below(10)
        name = self.pascal(2)
        if kind == 0:
            return [f"use std::{self.r.choice(['collections', 'sync', 'io', 'fmt'])}::{{{self.pascal(1)}, {self.pascal(1)}}};",
                    f"use crate::{self.snake(1)}::{self.snake(1)}::{{self, {self.pascal(1)} as {self.pascal(1)}}};", ""]
        if kind == 1:
            out = self.doc(0, example=True)
            out.append(f"#[derive(Debug, Clone, PartialEq, Eq, Hash)]")
            out.append(f"pub struct {name}<'a, T: Clone + Send + 'static> {{")
            for _ in range(self.r.between(2, 7)):
                if self.r.chance(30):
                    out.append(f"    /// {self.sentence()}.")
                out.append(f"    pub {self.snake()}: {self.r.choice(['&' + chr(39) + 'a str', 'Vec<T>', 'Option<Box<dyn Fn(u32) -> u64 + ' + chr(39) + 'a>>', 'HashMap<String, T>', 'u64', 'Arc<Mutex<T>>'])},")
            out += ["}", ""]
            return out
        if kind == 2:
            out = [f"impl<'a, T: Clone + Send + 'static> {name}<'a, T> {{"]
            for i in range(self.r.between(2, 6)):
                if i:
                    out.append("")
                out.extend(self.fn(4, True))
            out += ["}", ""]
            return out
        if kind == 3:
            out = self.doc(0)
            out.append("#[derive(Debug, Clone, Copy, serde::Serialize)]")
            out.append(f"#[serde(rename_all = \"{self.r.choice(['snake_case', 'kebab-case'])}\")]")
            out.append(f"pub enum {name} {{")
            for _ in range(self.r.between(2, 6)):
                v = self.pascal(1)
                out.append(self.r.choice([
                    f"    {v},",
                    f"    {v}(u32),",
                    f"    {v} {{ {self.snake(1)}: i64, {self.snake(1)}: bool }},",
                    f"    #[serde(rename = \"{self.w()}\")]\n    {v} = {self.r.between(1, 99)},",
                ]))
            out += ["}", ""]
            return lines_of("\n".join(out))
        if kind == 4:
            m = self.snake(2)
            return [
                f"macro_rules! {m} {{",
                f"    ($name:ident, $({self.w()}:expr),* $(,)?) => {{",
                f"        let $name = vec![$({self.w()} * {self.num()}),*];",
                "    };",
                f"    (@{self.w()} $value:literal) => {{",
                f"        concat!(\"{self.w()}-\", stringify!($value))",
                "    };",
                "}",
                "",
            ]
        if kind == 5:
            out = self.doc(0)
            out.append(f"pub trait {name}: Send + Sync {{")
            out.append(f"    type {self.pascal(1)}: Iterator<Item = u32>;")
            out.append(f"    const {self.upper()}: usize = {self.num()};")
            out.append("")
            out.append(f"    fn {self.fn_name()}(&self) -> Self::{self.pascal(1)};")
            out.extend(self.fn(4, True))
            out += ["}", ""]
            return out
        if kind == 6:
            return [
                f"pub const {self.upper()}: u64 = {self.r.between(1, 999)}_{self.r.between(100, 999)}_000;",
                f"static {self.upper()}: &[(&str, f64)] = &[(\"{self.w()}\", 1.5e-3), (\"{self.w()}\", 2.0f64)];",
                f"const {self.upper()}: char = '{self.r.choice(['a', 'z', '\\n', '\\\\', '\\u{7f}'])}';",
                "",
            ]
        if kind == 7:
            out = ["#[cfg(test)]", f"mod {self.snake(1)}_tests {{", "    use super::*;", ""]
            for _ in range(self.r.between(1, 3)):
                out.append("    #[test]")
                out.append(f"    fn {self.fn_name()}() {{")
                out.extend(self.body(8, self.r.between(1, 4)))
                out.append(f"        assert_eq!({self.expr()}, {self.num()}, {self.string()});")
                out.append("    }")
                out.append("")
            out[-1] = "}"
            out.append("")
            return out
        if kind == 8:
            return [
                "/*",
                f" * {self.sentence(8, 14)}.",
                f" * /* {self.sentence()} */ {self.sentence()}.",
                " */",
            ] + self.fn(0, False) + [""]
        return self.fn(0, False) + [""]


# --------------------------------------------------------------------------------------------
# C


class C(Gen):
    TYPES = ["int", "unsigned int", "size_t", "uint8_t", "uint32_t", "int64_t", "double",
             "const char *", "void *", "char", "long", "bool"]

    def ty(self):
        return self.r.choice(self.TYPES)

    def header(self):
        return [
            "/* Synthetic C for syntax-highlighting benchmarks. Generated by generate.py. */",
            "",
            "#include <stdbool.h>",
            "#include <stdint.h>",
            "#include <stdio.h>",
            "#include <stdlib.h>",
            "#include <string.h>",
            "",
        ]

    def filler(self):
        return "/* " + self.sentence() + " */" if self.r.chance(50) else ""

    def expr(self):
        a, b = self.snake(1), self.snake(1)
        return self.r.choice([
            f"{a} + {b} * {self.num()}",
            f"({a} << {self.r.between(1, 7)}) & 0x{self.r.below(65536):04x}u",
            f"{a}->{b}[{self.num()}]",
            f"*({a} + {b})",
            f"{a} ? {b} : {self.num()}",
            f"sizeof({self.ty()}) * {a}",
            f"(int)({a} / 2.5e1)",
            f"strlen({a}) - {b}",
            f"{self.fn_name()}({a}, &{b})",
        ])

    def string(self):
        return self.r.choice([
            f'"{self.sentence(2, 6)}\\n"',
            f'"{self.w()}: %d\\t%s\\n"',
            f'"\\x1b[{self.r.between(30, 37)}m{self.w()}\\x1b[0m"',
            f'"{self.w()} \\"{self.w()}\\" {self.w()}"',
            f'"{self.w()}" "{self.w()}"',
        ])

    def stmt(self, indent):
        p = " " * indent
        a = self.snake(1)
        return self.r.choice([
            [f"{p}{self.ty()} {a} = {self.expr()};"],
            [f"{p}{a} += {self.expr()};"],
            [f"{p}// {self.sentence()}"],
            [f"{p}fprintf(stderr, {self.string()}, {a}, {self.snake(1)});"],
            [f"{p}if ({a} == NULL || {self.expr()} > {self.num()}) {{",
             f"{p}    return -{self.r.between(1, 9)};",
             f"{p}}}"],
            [f"{p}for (size_t i = 0; i < {self.snake(1)}_len; ++i) {{",
             f"{p}    {a}[i] = (uint8_t)(i ^ 0xff);",
             f"{p}}}"],
            [f"{p}while (--{a} > 0) {{",
             f"{p}    if ({self.expr()})",
             f"{p}        break;",
             f"{p}}}"],
            [f"{p}switch ({a}) {{",
             f"{p}case {self.upper()}:",
             f"{p}    {self.snake(1)} = '{self.r.choice(['a', '\\0', '\\n', '\\\\', '\\x7f'])}';",
             f"{p}    break;",
             f"{p}case {self.r.between(0, 99)}:",
             f"{p}    /* fall through */",
             f"{p}default:",
             f"{p}    {self.snake(1)} = {self.expr()};",
             f"{p}}}"],
            [f"{p}memcpy({a}, {self.snake(1)}, sizeof(*{a}));"],
            [f"{p}{a} = ({self.ty()})malloc({self.num()} * sizeof({self.ty()}));"],
        ])

    def fn(self):
        name = self.fn_name()
        params = ", ".join(f"{self.ty()} {self.snake(1)}" for _ in range(self.r.between(0, 4))) or "void"
        out = []
        if self.r.chance(50):
            out += ["/**", f" * {self.sentence()}.", " *",
                    f" * @param {self.snake(1)} {self.sentence(3, 8)}.",
                    f" * @return {self.sentence(3, 6)}.", " */"]
        out.append(f"{self.r.choice(['static ', 'static inline ', ''])}{self.ty()} {name}({params})")
        out.append("{")
        for _ in range(self.r.between(2, 10)):
            out.extend(self.stmt(4))
        out.append(f"    return {self.expr()};")
        out += ["}", ""]
        return out

    def block(self):
        kind = self.r.below(9)
        name = self.snake(2)
        if kind == 0:
            guard = self.upper() + "_H"
            return [f"#ifndef {guard}", f"#define {guard} {self.r.between(1, 9)}",
                    f"#define {self.upper()} ({self.num()}u << {self.r.between(1, 8)})",
                    f"#define {self.upper()} \"{self.sentence(2, 4)}\"", f"#endif /* {guard} */", ""]
        if kind == 1:
            m = self.upper()
            return [
                f"#define {m}(x, y) \\",
                "    do { \\",
                f"        if ((x) > (y)) \\",
                f"            {self.fn_name()}((x), (y), \"{self.w()}\"); \\",
                "    } while (0)",
                "",
            ]
        if kind == 2:
            out = [f"typedef struct {name} {{"]
            for _ in range(self.r.between(2, 7)):
                out.append(self.r.choice([
                    f"    {self.ty()} {self.snake(1)};",
                    f"    unsigned {self.snake(1)} : {self.r.between(1, 7)};",
                    f"    char {self.snake(1)}[{self.num()}];",
                    f"    struct {self.snake(1)} *{self.snake(1)}; /* {self.sentence(2, 5)} */",
                    f"    union {{ int {self.w()}; float {self.w()}; }} {self.w()};",
                ]))
            out += [f"}} {name}_t;", ""]
            return out
        if kind == 3:
            out = [f"enum {name} {{"]
            for i in range(self.r.between(2, 6)):
                out.append(f"    {self.upper()} = {i * self.r.between(1, 4)},")
            out += ["};", ""]
            return out
        if kind == 4:
            fp = name + "_fn"
            return [
                f"typedef int (*{fp})(void *{self.w()}, const char *{self.w()}, size_t {self.w()});",
                f"static const {fp} {self.snake(1)}_table[] = {{",
                f"    {self.fn_name()}, {self.fn_name()},",
                f"    [{self.r.between(2, 9)}] = NULL,",
                "};",
                "",
            ]
        if kind == 5:
            return ["#if defined(__linux__) && !defined(" + self.upper() + ")",
                    f"#    define {self.upper()} {self.num()}",
                    f"#elif defined({self.upper()})",
                    f"#    include \"{self.snake(1)}.h\"",
                    "#else",
                    f"#    error \"{self.sentence(2, 5)}\"",
                    "#endif", ""]
        if kind == 6:
            vals = ", ".join(f"0x{self.r.below(256):02x}" for _ in range(self.r.between(4, 12)))
            return [f"static const uint8_t {name}[] = {{ {vals} }};",
                    f"static const char *const {self.snake(1)}_names[] = {{ {self.string()}, {self.string()}, NULL }};", ""]
        if kind == 7:
            return ["/*", f" * {self.sentence(8, 14)}.", " *", f" * {self.sentence(6, 12)}.", " */"] + self.fn()
        return self.fn()


# --------------------------------------------------------------------------------------------
# JavaScript


class Js(Gen):
    def header(self):
        return [
            "// Synthetic JavaScript for syntax-highlighting benchmarks. Generated by generate.py.",
            "",
            "'use strict';",
            "",
        ]

    def filler(self):
        return "// " + self.sentence() if self.r.chance(50) else ""

    def expr(self):
        a, b = self.var(), self.var()
        return self.r.choice([
            f"{a}?.{b} ?? {self.num()}",
            f"{a}.map((x) => x * {self.num()}).filter(Boolean)",
            f"`{self.w()} ${{{a}}} and ${{{b}.{self.w()} + {self.num()}}}`",
            f"/^[a-z]+-\\d{{{self.r.between(1, 4)}}}$/i.test({a})",
            f"{a}.replace(/\\s+/g, '{self.w()}')",
            f"{{ ...{a}, {b}: [{self.num()}, {self.num()}] }}",
            f"await {self.camel()}({a}, {{ {b} }})",
            f"{a} === undefined ? null : {b}",
            f"new {self.pascal(1)}({a}, '{self.w()}')",
            f"Object.entries({a}).reduce((acc, [key, value]) => acc + value.length, 0)",
            f"{a}.{self.v()}({b}) | 0x{self.r.below(256):02x}",
        ])

    def stmt(self, indent):
        p = " " * indent
        a = self.var()
        return self.r.choice([
            [f"{p}const {a} = {self.expr()};"],
            [f"{p}let {{ {self.var()}, {self.var()} = {self.num()} }} = {self.expr()};"],
            [f"{p}// {self.sentence()}"],
            [f"{p}console.{self.r.choice(['log', 'warn', 'error'])}('{self.sentence(2, 5)}', {a});"],
            [f"{p}if ({a} && !{self.var()}) {{", f"{p}  return {self.expr()};", f"{p}}}"],
            [f"{p}for (const [{self.w()}, {self.w()}] of {a}.entries()) {{",
             f"{p}  {self.var()} += {self.expr()};", f"{p}}}"],
            [f"{p}try {{", f"{p}  {a} = {self.expr()};",
             f"{p}}} catch (error) {{", f"{p}  throw new Error(`{self.w()}: ${{error.message}}`);",
             f"{p}}} finally {{", f"{p}  {self.var()}.{self.v()}();", f"{p}}}"],
            [f"{p}switch ({a}) {{",
             f"{p}  case '{self.w()}':",
             f"{p}    {self.var()} = {self.expr()};",
             f"{p}    break;",
             f"{p}  default:",
             f"{p}    {self.var()} = null;",
             f"{p}}}"],
            [f"{p}const {a} = `",
             f"{p}  <section class=\"{self.w()}\">",
             f"{p}    ${{{self.var()}.map((item) => `<p>${{item.{self.w()}}}</p>`).join('')}}",
             f"{p}  </section>`;"],
            [f"{p}{a}.addEventListener('{self.w()}', async (event) => {{",
             f"{p}  event.preventDefault();", f"{p}  await {self.camel()}(event.target);",
             f"{p}}});"],
        ])

    def long_line(self):
        items = []
        while sum(len(i) + 2 for i in items) < 2000:
            items.append(self.r.choice([f"'{self.w()}'", self.num(), f"{{ {self.w()}: {self.num()} }}", f"/{self.w()}+/u"]))
        return [f"export const {self.camel()} = [{', '.join(items)}];", ""]

    def fn(self, indent, method):
        p = " " * indent
        name = self.camel()
        params = ", ".join(self.var() for _ in range(self.r.between(0, 3)))
        out = []
        if self.r.chance(40):
            out += [f"{p}/**", f"{p} * {self.sentence()}.",
                    f"{p} * @param {{{self.r.choice(['string', 'number', 'Object'])}}} {self.var()} {self.sentence(2, 6)}",
                    f"{p} * @returns {{Promise<void>}}", f"{p} */"]
        asyn = "async " if self.r.chance(30) else ""
        arrow = not method and self.r.chance(40)
        if method:
            stat = "static " if self.r.chance(10) else ""
            out.append(f"{p}{stat}{asyn}{name}({params}) {{")
        elif arrow:
            out.append(f"{p}export const {name} = {asyn}({params}) => {{")
        else:
            out.append(f"{p}export {asyn}function {name}({params}) {{")
        for _ in range(self.r.between(2, 8)):
            out.extend(self.stmt(indent + 2))
        out.append(f"{p}  return {self.expr()};")
        out.append(f"{p}}};" if arrow else f"{p}}}")
        return out

    def block(self):
        kind = self.r.below(100)
        if kind < 1:
            return self.long_line()
        if kind < 10:
            return [f"import {{ {self.camel()}, {self.camel()} as {self.var()} }} from './{self.w()}/{self.w()}.js';",
                    f"import {self.pascal(1)} from '{self.w()}';", ""]
        if kind < 40:
            name = self.pascal(2)
            out = [f"export class {name} extends {self.pascal(1)} {{",
                   f"  #{self.var()} = new Map();",
                   f"  static {self.var()} = Object.freeze({{ {self.w()}: {self.num()} }});",
                   "",
                   f"  constructor({self.var()}, {{ {self.var()} = '{self.w()}', ...rest }} = {{}}) {{",
                   "    super(rest);",
                   f"    this.{self.var()} = {self.expr()};",
                   "  }",
                   "",
                   f"  get {self.var()}() {{",
                   f"    return this.#{self.var()}.size;",
                   "  }"]
            for _ in range(self.r.between(1, 4)):
                out.append("")
                out.extend(self.fn(2, True))
            out += ["}", ""]
            return out
        if kind < 50:
            return [f"const {self.upper()} = Object.freeze({{",
                    f"  {self.w()}: '{self.sentence(2, 4)}',",
                    f"  '{self.w()}-{self.w()}': {self.num()},",
                    f"  [{self.w()}Key]: /\\b{self.w()}\\b/g,",
                    f"  nested: {{ {self.w()}: [true, false, null], {self.w()}: 1.5e3 }},",
                    "});", ""]
        if kind < 55:
            return [f"const {self.var()} = {self.camel()}`",
                    f"  {self.sentence(3, 8)} ${{{self.var()}}}",
                    f"  ${{{self.var()} ? `{self.w()} ${{{self.var()}}}` : ''}} {self.sentence(2, 5)}",
                    "`;", ""]
        if kind < 60:
            return ["/*", f" * {self.sentence(8, 14)}.", " */", f"{self.var()}: for (let i = 0; i < {self.num()}; i++) {{",
                    f"  for (let j = i; j > 0; j -= {self.r.between(1, 4)}) {{",
                    f"    if (j % {self.r.between(2, 7)} === 0) continue {self.var()};", "  }", "}", ""]
        return self.fn(0, False) + [""]

    def build(self, total):
        out = super().build(total)
        # Make sure at least a few ~2 KB lines exist even if the dice were unkind.
        for i in range(3):
            out[(i + 1) * total // 4] = self.long_line()[0]
        return out


# --------------------------------------------------------------------------------------------
# Python


class Python(Gen):
    def header(self):
        return [
            '"""Synthetic Python for syntax-highlighting benchmarks. Generated by generate.py."""',
            "",
            "from __future__ import annotations",
            "",
            "import asyncio",
            "import dataclasses",
            "from collections.abc import Iterable, Mapping",
            "",
        ]

    def filler(self):
        return "# " + self.sentence() if self.r.chance(50) else ""

    def expr(self):
        a, b = self.snake(1), self.snake(1)
        return self.r.choice([
            f"[x * {self.num()} for x in {a} if x % {self.r.between(2, 7)}]",
            f"{{key: value for key, value in {a}.items() if value is not None}}",
            f"{{{a} for {a} in {b}}}",
            f"sum(x for x in {a})",
            f'f"{self.w()} {{{a}!r:>{self.r.between(4, 12)}}} and {{{b}.{self.w()}:.2f}}"',
            f"{a}.get('{self.w()}', {self.num()})",
            f"lambda {a}, {b}=None: {a} or {b}",
            f"{a} if {b} else {self.num()}",
            f"await {self.fn_name()}({a}, {b}={self.num()})",
            f"r'\\d+-{self.w()}'",
            f"b'{self.w()}\\x00'",
            f"{a}[{self.r.between(0, 4)}:{self.r.between(5, 9)}:2]",
        ])

    def stmt(self, indent):
        p = " " * indent
        a = self.snake(1)
        return self.r.choice([
            [f"{p}{a} = {self.expr()}"],
            [f"{p}{a}: {self.r.choice(['int', 'str', 'list[str]', 'dict[str, int] | None'])} = {self.expr()}"],
            [f"{p}# {self.sentence()}"],
            [f"{p}if {a} is None:", f"{p}    raise ValueError(f\"{self.sentence(2, 5)}: {{{a}!r}}\")"],
            [f"{p}for index, {a} in enumerate({self.snake(1)}):", f"{p}    {self.snake(1)} += {self.expr()}"],
            [f"{p}try:", f"{p}    {a} = {self.expr()}", f"{p}except (KeyError, {self.pascal(1)}Error) as error:",
             f"{p}    log.warning('%s failed: %s', {a}, error)", f"{p}finally:", f"{p}    {self.snake(1)}.{self.v()}()"],
            [f"{p}with open({self.snake(1)}, encoding='utf-8') as handle:", f"{p}    {a} = handle.read().splitlines()"],
            [f"{p}match {a}:", f"{p}    case {{'{self.w()}': {self.w()}}}:", f"{p}        {self.snake(1)} = {self.w()}",
             f"{p}    case [{self.w()}, *rest]:", f"{p}        {self.snake(1)} = rest", f"{p}    case _:", f"{p}        pass"],
            [f"{p}async with {self.snake(1)}.{self.v()}() as {a}:", f"{p}    async for item in {a}:",
             f"{p}        yield item"],
            [f"{p}{a} = (", f"{p}    {self.expr()}", f"{p}    + {self.expr()}", f"{p})"],
        ])

    def fn(self, indent, method):
        p = " " * indent
        out = []
        if method and self.r.chance(30):
            out.append(f"{p}@{self.r.choice(['property', 'staticmethod', 'classmethod', 'functools.cache'])}")
        elif self.r.chance(20):
            out.append(f"{p}@{self.snake(1)}.{self.v()}('{self.w()}', retries={self.r.between(1, 5)})")
        asyn = "async " if self.r.chance(20) else ""
        params = ["self"] if method else []
        params += [f"{self.snake(1)}: {self.r.choice(['int', 'str', 'Iterable[str]', 'Mapping[str, int]'])}" for _ in range(self.r.between(0, 3))]
        if self.r.chance(30):
            params.append(f"*, {self.snake(1)}: bool = False")
        out.append(f"{p}{asyn}def {self.fn_name()}({', '.join(params)}) -> {self.r.choice(['None', 'int', 'str', 'list[int]'])}:")
        if self.r.chance(60):
            if self.r.chance(50):
                out.append(f'{p}    """{self.sentence()}."""')
            else:
                out += [f'{p}    """{self.sentence()}.', "", f"{p}    {self.sentence(8, 14)}.", f'{p}    """']
        for _ in range(self.r.between(1, 7)):
            out.extend(self.stmt(indent + 4))
        out.append(f"{p}    return {self.expr()}")
        return out

    def block(self):
        kind = self.r.below(10)
        name = self.pascal(2)
        if kind < 4:
            out = []
            if self.r.chance(40):
                out += ["@dataclasses.dataclass(frozen=True, slots=True)"]
            out.append(f"class {name}({self.pascal(1)}):")
            out.append(f'    """{self.sentence()}."""')
            out.append("")
            for _ in range(self.r.between(1, 4)):
                out.append(f"    {self.snake(1)}: {self.r.choice(['int', 'str', 'list[str]', 'float'])} = {self.r.choice([self.num(), repr(self.w()), 'dataclasses.field(default_factory=list)'])}")
            for _ in range(self.r.between(1, 5)):
                out.append("")
                out.extend(self.fn(4, True))
            return out + ["", ""]
        if kind == 4:
            return [f"{self.upper()} = \"\"\"", f"{self.sentence(6, 12)}", f"    {self.sentence(4, 8)} '{self.w()}' \"{self.w()}\"",
                    f"{self.sentence(3, 6)}", '"""', ""]
        if kind == 5:
            return [f"{self.upper()}: dict[str, tuple[int, ...]] = {{",
                    f"    '{self.w()}': ({self.num()}, {self.num()}),",
                    f"    '{self.w()}': (0x{self.r.below(256):02x}, 0o{self.r.below(64):o}, 1_000),", "}", ""]
        return self.fn(0, False) + ["", ""]


# --------------------------------------------------------------------------------------------
# Shell


class Shell(Gen):
    def header(self):
        return [
            "#!/usr/bin/env bash",
            "# Synthetic shell script for syntax-highlighting benchmarks. Generated by generate.py.",
            "set -euo pipefail",
            "",
            "declare -A config=()",
            "declare -a queue=()",
            "",
        ]

    def filler(self):
        return "# " + self.sentence() if self.r.chance(50) else ""

    def word(self):
        a = self.snake(1)
        return self.r.choice([
            f'"${{{a}:-{self.w()}}}"',
            f'"${{#{a}[@]}}"',
            f'"$({self.w()}_{self.v()} "${a}" | tr -d \'\\n\')"',
            f'"$(dirname "$(readlink -f "${{BASH_SOURCE[0]}}")")"',
            f"\"{self.w()} 'single' \\\"double\\\" ${a}\"",
            f"'{self.sentence(2, 4)}'",
            f'"${{{a}//{self.w()}/{self.w()}}}"',
            f"$(( {a} * {self.r.between(2, 9)} + {self.num()} ))",
            f'"${{config[{self.w()}]}}"',
        ])

    def stmt(self, indent):
        p = " " * indent
        a = self.snake(1)
        return self.r.choice([
            [f"{p}local {a}={self.word()}"],
            [f"{p}# {self.sentence()}"],
            [f"{p}printf '%s: %d\\n' {self.word()} \"${a}\""],
            [f"{p}if [[ -f \"${a}\" && ${self.snake(1)} =~ ^[a-z]+-[0-9]+$ ]]; then",
             f"{p}    {self.fn_name()} {self.word()}", f"{p}elif (( {a} > {self.num()} )); then",
             f"{p}    return 1", f"{p}fi"],
            [f"{p}for {a} in \"${{queue[@]}}\"; do", f"{p}    config[${a}]={self.word()}", f"{p}done"],
            [f"{p}while IFS='=' read -r key value; do", f"{p}    config[$key]=\"$value\"",
             f"{p}done < <(grep -v '^#' \"${a}\" || true)"],
            [f"{p}case \"${a}\" in", f"{p}    {self.w()}|{self.w()})", f"{p}        {self.fn_name()} {self.word()}",
             f"{p}        ;;", f"{p}    *.{self.w()})", f"{p}        queue+=(\"${a}\")", f"{p}        ;;",
             f"{p}    *)", f"{p}        echo \"{self.sentence(2, 5)}: ${a}\" >&2", f"{p}        ;;", f"{p}esac"],
            [f"{p}cat <<EOF", f"{self.sentence()} ${a}", f"$( date +%s ) {self.sentence(2, 5)}", "EOF"],
            [f"{p}cat <<'EOF' > \"${a}\"", f"{self.sentence()} $not_expanded", "EOF"],
            [f"{p}{self.fn_name()} {self.word()} | sort -u | head -n {self.num()} || true"],
            [f"{p}[[ -n \"${{{a}:-}}\" ]] && {a}=\"$({self.fn_name()} \"${a}\")\""],
        ])

    def block(self):
        if self.r.chance(15):
            a = self.snake(1)
            return [f"{self.upper()}={self.word()}", f"readonly {a}=({self.word()} {self.word()} '{self.w()}')",
                    f"trap 'rm -f \"${{{a}}}\"' EXIT", ""]
        out = [f"{self.fn_name()}() {{"]
        for _ in range(self.r.between(2, 8)):
            out.extend(self.stmt(4))
        out += ["}", ""]
        return out


# --------------------------------------------------------------------------------------------
# Markdown


class Markdown(Gen):
    def __init__(self, seed):
        super().__init__(seed)
        self.langs = {
            "rust": Rust(seed + 1), "c": C(seed + 2), "js": Js(seed + 3),
            "python": Python(seed + 4), "sh": Shell(seed + 5),
        }

    def header(self):
        return ["# Synthetic Markdown for syntax-highlighting benchmarks", "",
                "Generated by `generate.py`.", ""]

    def filler(self):
        return ""

    def inline(self):
        parts = []
        for _ in range(self.r.between(1, 4)):
            parts.append(self.r.choice([
                self.sentence(4, 10).lower(),
                f"**{self.w()} {self.w()}**",
                f"*{self.adj()}*",
                f"`{self.snake()}()`",
                f"[{self.w()} {self.w()}](https://example.com/{self.w()}/{self.w()})",
                f"![{self.w()}](images/{self.w()}.png \"{self.w()}\")",
                f"<kbd>{self.w()}</kbd>",
            ]))
        text = ", ".join(parts)
        return text[0].upper() + text[1:] + "."

    def fence(self):
        lang = self.r.choice(["rust", "c", "js", "python", "sh", "json", "yaml"])
        if lang == "json":
            body = ["{", f'  "{self.w()}": "{self.sentence(2, 4)}",', f'  "{self.w()}": [{self.num()}, {self.num()}, true, null],',
                    f'  "{self.w()}": {{ "{self.w()}": {self.r.between(0, 99)}.5 }}', "}"]
        elif lang == "yaml":
            body = [f"{self.w()}:", f"  {self.w()}: {self.sentence(2, 4)}", f"  {self.w()}:",
                    f"    - {self.w()}", f"    - '{self.w()} {self.w()}'", f"  {self.w()}: {self.num()}  # {self.sentence(2, 4)}"]
        else:
            gen = self.langs[lang]
            body = gen.block()
            while len(body) < 4:
                body += gen.block()
            # Rust doc examples carry their own fences; keep them out of the Markdown block.
            body = [line for line in body[:60] if "```" not in line]
            while body and body[-1] == "":
                body.pop()
        return [f"```{lang}"] + body + ["```", ""]

    def block(self):
        kind = self.r.below(20)
        if kind < 6:
            return self.fence()
        if kind < 8:
            return ["#" * self.r.between(2, 4) + " " + self.sentence(2, 5), ""]
        if kind < 12:
            return [self.inline() for _ in range(self.r.between(1, 4))] + [""]
        if kind < 14:
            out = []
            for _ in range(self.r.between(2, 6)):
                out.append(f"{self.r.choice(['-', '*', '1.'])} {self.inline()}")
                if self.r.chance(30):
                    out.append(f"   - {self.inline()}")
            return out + [""]
        if kind < 16:
            out = [f"| {self.w()} | {self.w()} | {self.w()} |", "| --- | :---: | ---: |"]
            for _ in range(self.r.between(2, 6)):
                out.append(f"| `{self.snake(1)}` | {self.sentence(2, 4)} | {self.num()} |")
            return out + [""]
        if kind == 16:
            return [f"> {self.inline()}", ">", f"> {self.inline()}", ""]
        if kind == 17:
            return ["---", ""]
        if kind == 18:
            return [f"<details><summary>{self.w()}</summary>", "", self.inline(), "", "</details>", ""]
        return [f"1. {self.inline()}", f"2. {self.inline()}", ""]


FIXTURES = [
    ("rust-50k.rs", Rust, 50_000, 1),
    ("c-50k.c", C, 50_000, 2),
    ("js-50k.js", Js, 50_000, 3),
    ("python-50k.py", Python, 50_000, 4),
    ("markdown-50k.md", Markdown, 50_000, 5),
    ("shell-5k.sh", Shell, 5_000, 6),
]


def render(cls, total, seed):
    lines = cls(seed).build(total)
    assert len(lines) == total, (cls.__name__, len(lines))
    assert all("\n" not in line for line in lines), cls.__name__
    return ("\n".join(line.rstrip() for line in lines) + "\n").encode("utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--check", action="store_true", help="fail if a committed fixture differs")
    args = parser.parse_args()
    stale = []
    for name, cls, total, seed in FIXTURES:
        data = render(cls, total, seed)
        path = HERE / name
        if args.check:
            if not path.exists() or path.read_bytes() != data:
                stale.append(name)
        else:
            path.write_bytes(data)
            print(f"wrote {name}: {total} lines, {len(data)} bytes")
    if stale:
        print("stale fixtures (run generate.py): " + ", ".join(stale), file=sys.stderr)
        return 1
    if args.check:
        print("fixtures match generate.py")
    return 0


if __name__ == "__main__":
    sys.exit(main())
