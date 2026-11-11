// SPDX-License-Identifier: MIT

//! What the QA mode asks of the toolkit an application is drawn with.

use std::time::Duration;

use crate::{FieldState, Orientation, Rect};

/// What the application says a node is for, where its role and name do not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// A browser pane, with the name of the view it shows: `Columns`, `Icons` or `List`.
    Pane(&'static str),
    EntryContainer,
    PaneSurface,
    Entry,
    Dialog,
    MenuItem,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct States {
    pub selected: bool,
    pub focused: bool,
    pub checked: bool,
    pub pressed: bool,
    pub sensitive: bool,
    pub selectable: bool,
    pub editable: bool,
    pub focusable: bool,
    pub busy: bool,
    /// Outlined as the result Enter in a filter field opens.
    pub marked: bool,
}

/// One listed file or folder, as the snapshot reports it.
#[derive(Clone, Debug, PartialEq)]
pub struct EntryRow<Node> {
    pub node: Node,
    pub name: String,
    pub selected: bool,
    pub focused: bool,
}

/// The application's own names for the parts the snapshot looks for, in the language it runs in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Labels {
    pub preview: String,
    pub peek: String,
    pub search_results: String,
}

/// How far a scroll pane has scrolled along one direction, and whether it can.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scroll {
    pub value: f64,
    pub page: f64,
    pub scrollable: bool,
}

/// One toolkit's side of the QA mode. A `Node` is a live element of the interface: a
/// `gtk::Widget`, or a Rune accessibility node.
///
/// The methods answer for one node at a time and decide nothing: which node a target means,
/// what a predicate holds and how a step waits are the engine's, so two backends cannot
/// come to mean different things by `selected` or "the entry named X".
#[expect(
    async_fn_in_trait,
    reason = "the engine runs on the toolkit's own thread, so no future has to be Send"
)]
pub trait Backend {
    type Node: Clone + PartialEq;
    /// A handle that does not keep its node alive: `remember` holds these.
    type Kept: Clone;

    /// The window the steps act on.
    fn window(&self) -> Option<Self::Node>;
    /// Every window the toolkit has open, whether or not it is drawn.
    fn toplevels(&self) -> Vec<Self::Node>;
    fn title(&self, window: &Self::Node) -> Option<String>;
    /// The width of a window's content, in points.
    fn window_width(&self, window: &Self::Node) -> i32;
    /// The node in `window` that takes key presses, while it is shown.
    fn focus(&self, window: &Self::Node) -> Option<Self::Node>;

    fn parent(&self, node: &Self::Node) -> Option<Self::Node>;
    /// Every child, hidden ones included, in the order the toolkit keeps them.
    fn children(&self, node: &Self::Node) -> Vec<Self::Node>;
    /// False for a node the application has hidden. Its descendants are then not presented.
    fn visible(&self, node: &Self::Node) -> bool;
    /// Whether the node still belongs to a window.
    fn attached(&self, node: &Self::Node) -> bool;

    /// The AT-SPI role name, lowercase with spaces.
    fn role(&self, node: &Self::Node) -> &'static str;
    fn name(&self, node: &Self::Node) -> String;
    fn description(&self, node: &Self::Node) -> String;
    fn mark(&self, node: &Self::Node) -> Option<Mark>;
    fn states(&self, node: &Self::Node) -> States;
    /// The direction a box, a separator or a scroll bar runs in. Asked only of a node a step
    /// tests for it: GTK logs a critical when some widgets are asked.
    fn orientation(&self, node: &Self::Node) -> Option<Orientation>;
    /// The text a label shows or a field holds.
    fn text(&self, node: &Self::Node) -> Option<String>;
    /// The content, caret and selection of a text field.
    fn field(&self, node: &Self::Node) -> Option<FieldState>;
    /// The number a slider or another ranged control stands at.
    fn value(&self, node: &Self::Node) -> Option<f64>;

    /// The node's box inside the surface it is drawn on; `None` while it is not drawn. This
    /// must be cheap: the snapshot asks it of every node.
    fn frame(&self, node: &Self::Node) -> Option<Rect>;
    /// Where the corner of that surface is on the screen. A popup is a surface of its own.
    fn origin(&self, node: &Self::Node) -> Result<(f64, f64), String>;
    /// The node's box in the coordinates of `window`, whether or not it is drawn.
    fn layout(&self, node: &Self::Node, window: &Self::Node) -> Option<Rect>;
    /// `None` for a node that is not a scroll pane.
    fn scroll(&self, node: &Self::Node, along: Orientation) -> Option<Scroll>;
    /// The position of the scroll bar a scroll pane shows, when it shows one.
    fn scroll_position(&self, scroller: &Self::Node) -> Option<f64>;

    /// The entry a node is, when it is one.
    fn entry(&self, node: &Self::Node) -> Option<EntryRow<Self::Node>> {
        (self.mark(node) == Some(Mark::Entry)).then(|| {
            let states = self.states(node);
            EntryRow {
                node: node.clone(),
                name: self.name(node),
                selected: states.selected,
                focused: states.focused,
            }
        })
    }

    /// The entries an entry container lists, in any order. A toolkit that recycles rows
    /// answers this itself, to leave out a row that still shows its previous entry.
    fn entries(&self, container: &Self::Node) -> Vec<EntryRow<Self::Node>> {
        crate::presented(self, container)
            .iter()
            .filter(|node| self.frame(node).is_some())
            .filter_map(|node| self.entry(node))
            .collect()
    }

    fn labels(&self) -> Labels;

    fn keep(&self, node: &Self::Node) -> Self::Kept;
    fn recall(&self, kept: &Self::Kept) -> Option<Self::Node>;

    /// The tree a failed wait reports and the `tree` query returns.
    fn dump(&self, window: &Self::Node) -> String;

    /// Returns once the window has painted a frame, or at once if it has nothing to paint.
    async fn settle(&self, window: &Self::Node);
    /// Lets the application run for this long.
    async fn sleep(&self, time: Duration);
}
