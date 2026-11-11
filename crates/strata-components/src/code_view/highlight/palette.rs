//! Scope-stack classes and their colours.
//!
//! Every token's colour is a function of its scope stack alone (syntect's
//! `Highlighter::style_for_stack` is the same computation `HighlightState` makes one push at a
//! time), so the highlighter keeps no `HighlightState`. Each distinct stack becomes a class,
//! and a theme turns the classes into a palette. A theme switch re-resolves the palette, a few
//! thousand stacks at most, and never re-parses.

use std::collections::HashMap;

use rune::Color;
use rune::color::srgb_decode;
use syntect::highlighting::{Highlighter, Theme};
use syntect::parsing::Scope;

/// A distinct scope stack, by index into its interner. Class 0 is the empty stack.
pub(crate) type Class = u32;

/// Interns scope stacks as classes. A push from a class is cached as a transition and a pop
/// follows parent links, so tokenizing a line hashes a scope at most once per push.
pub(crate) struct Interner {
    by_stack: HashMap<Vec<Scope>, Class>,
    push: HashMap<(Class, Scope), Class>,
    parent: Vec<Class>,
    stacks: Vec<Vec<Scope>>,
}

impl Interner {
    pub(crate) fn new() -> Interner {
        let mut by_stack = HashMap::new();
        by_stack.insert(Vec::new(), 0);
        Interner {
            by_stack,
            push: HashMap::new(),
            parent: vec![0],
            stacks: vec![Vec::new()],
        }
    }

    /// The class of `stack`, interning it and its prefixes when new.
    pub(crate) fn class_of(&mut self, stack: &[Scope]) -> Class {
        if let Some(&c) = self.by_stack.get(stack) {
            return c;
        }
        let parent = match stack.split_last() {
            Some((_, rest)) => self.class_of(rest),
            None => 0,
        };
        let c = Class::try_from(self.stacks.len()).unwrap_or(Class::MAX);
        self.stacks.push(stack.to_vec());
        self.parent.push(parent);
        self.by_stack.insert(stack.to_vec(), c);
        c
    }

    /// The class after pushing `scope` onto `from`, where `after` is the stack with it pushed.
    pub(crate) fn push(&mut self, from: Class, scope: Scope, after: &[Scope]) -> Class {
        if let Some(&c) = self.push.get(&(from, scope)) {
            return c;
        }
        let c = self.class_of(after);
        self.push.insert((from, scope), c);
        c
    }

    /// The class one scope shorter: what a pop leaves.
    pub(crate) fn parent(&self, c: Class) -> Class {
        self.parent.get(c as usize).copied().unwrap_or(0)
    }

    pub(crate) fn len(&self) -> usize {
        self.stacks.len()
    }

    /// The colours of classes `from..` under `theme` (see [`resolve`]).
    pub(crate) fn resolve_from(&self, theme: &Theme, from: usize) -> Vec<Option<Color>> {
        resolve(theme, self.stacks.get(from..).unwrap_or_default())
    }
}

/// Each stack's foreground under `theme`, or `None` where it is the theme's default
/// foreground: those tokens draw in the host's own text colour and get no span.
pub(crate) fn resolve(theme: &Theme, stacks: &[Vec<Scope>]) -> Vec<Option<Color>> {
    let hl = Highlighter::new(theme);
    let default = hl.get_default().foreground;
    stacks
        .iter()
        .map(|s| {
            let fg = hl.style_for_stack(s).foreground;
            (fg != default).then(|| to_rune(fg))
        })
        .collect()
}

/// A syntect colour (sRGB bytes) as a Rune colour (linear light).
pub(crate) fn to_rune(c: syntect::highlighting::Color) -> Color {
    let ch = |v: u8| srgb_decode(f32::from(v) / 255.0);
    Color {
        r: ch(c.r),
        g: ch(c.g),
        b: ch(c.b),
        a: f32::from(c.a) / 255.0,
    }
}
