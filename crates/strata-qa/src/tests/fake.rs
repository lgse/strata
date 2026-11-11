// SPDX-License-Identifier: MIT

//! A toolkit that is only a tree in memory, so the engine can be tested without a display.

use super::super::{Backend, FieldState, Labels, Mark, Orientation, Rect, Scroll, States};

#[derive(Clone, Default)]
pub(super) struct Node {
    pub(super) role: &'static str,
    pub(super) name: &'static str,
    pub(super) mark: Option<Mark>,
    pub(super) states: States,
    pub(super) text: Option<&'static str>,
    /// `[x, y, width, height]`; a node without one is not drawn.
    pub(super) frame: Option<[f64; 4]>,
    pub(super) hidden: bool,
    pub(super) field: bool,
}

#[derive(Default)]
pub(super) struct Fake {
    nodes: Vec<Node>,
    parents: Vec<Option<usize>>,
    pub(super) focus: Option<usize>,
}

pub(super) const WINDOW: usize = 0;

impl Fake {
    /// A window 800 by 600 points, as node 0.
    pub(super) fn new() -> Self {
        let mut fake = Self::default();
        fake.nodes.push(Node {
            role: "frame",
            frame: Some([0.0, 0.0, 800.0, 600.0]),
            ..Node::default()
        });
        fake.parents.push(None);
        fake
    }

    pub(super) fn add(&mut self, parent: usize, node: Node) -> usize {
        self.nodes.push(node);
        self.parents.push(Some(parent));
        self.nodes.len() - 1
    }
}

pub(super) fn drawn(role: &'static str, name: &'static str, frame: [f64; 4]) -> Node {
    Node {
        role,
        name,
        frame: Some(frame),
        ..Node::default()
    }
}

impl Backend for Fake {
    type Node = usize;
    type Kept = usize;

    fn window(&self) -> Option<usize> {
        Some(WINDOW)
    }

    fn toplevels(&self) -> Vec<usize> {
        vec![WINDOW]
    }

    fn title(&self, _window: &usize) -> Option<String> {
        Some("Strata".to_owned())
    }

    fn window_width(&self, _window: &usize) -> i32 {
        800
    }

    fn focus(&self, _window: &usize) -> Option<usize> {
        self.focus
    }

    fn parent(&self, node: &usize) -> Option<usize> {
        self.parents[*node]
    }

    fn children(&self, node: &usize) -> Vec<usize> {
        (0..self.nodes.len())
            .filter(|child| self.parents[*child] == Some(*node))
            .collect()
    }

    fn visible(&self, node: &usize) -> bool {
        !self.nodes[*node].hidden
    }

    fn attached(&self, _node: &usize) -> bool {
        true
    }

    fn role(&self, node: &usize) -> &'static str {
        self.nodes[*node].role
    }

    fn name(&self, node: &usize) -> String {
        self.nodes[*node].name.to_owned()
    }

    fn description(&self, _node: &usize) -> String {
        String::new()
    }

    fn mark(&self, node: &usize) -> Option<Mark> {
        self.nodes[*node].mark
    }

    fn states(&self, node: &usize) -> States {
        States {
            focused: self.focus == Some(*node),
            ..self.nodes[*node].states
        }
    }

    fn orientation(&self, _node: &usize) -> Option<Orientation> {
        None
    }

    fn text(&self, node: &usize) -> Option<String> {
        self.nodes[*node].text.map(str::to_owned)
    }

    fn field(&self, node: &usize) -> Option<FieldState> {
        let found = &self.nodes[*node];
        found.field.then(|| FieldState {
            name: found.name.to_owned(),
            text: found.text.unwrap_or_default().to_owned(),
            focused: self.focus == Some(*node),
            showing: found.frame.is_some(),
            ..FieldState::default()
        })
    }

    fn value(&self, _node: &usize) -> Option<f64> {
        None
    }

    fn frame(&self, node: &usize) -> Option<Rect> {
        self.nodes[*node].frame.map(|[x, y, width, height]| Rect {
            x,
            y,
            width,
            height,
        })
    }

    fn origin(&self, _node: &usize) -> Result<(f64, f64), String> {
        Ok((0.0, 0.0))
    }

    fn layout(&self, node: &usize, _window: &usize) -> Option<Rect> {
        self.frame(node)
    }

    fn scroll(&self, _node: &usize, _along: Orientation) -> Option<Scroll> {
        None
    }

    fn scroll_position(&self, _scroller: &usize) -> Option<f64> {
        None
    }

    fn labels(&self) -> Labels {
        Labels {
            preview: "Preview".to_owned(),
            peek: "Folder peek".to_owned(),
            search_results: "Search results".to_owned(),
        }
    }

    fn keep(&self, node: &usize) -> usize {
        *node
    }

    fn recall(&self, kept: &usize) -> Option<usize> {
        Some(*kept)
    }

    fn dump(&self, _window: &usize) -> String {
        String::new()
    }
}
