// SPDX-License-Identifier: MIT

//! What the QA mode asks of the toolkit an application is drawn with.

use std::{path::PathBuf, time::Duration};

use crate::{Chord, Command, EntryPart, FieldState, Modifier, Orientation, Outcome, Rect, Scope};

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

#[expect(
    async_fn_in_trait,
    reason = "the engine runs on the toolkit's own thread, so no future has to be Send"
)]
pub trait Backend {
    type Node: Clone + PartialEq;
    /// A handle that does not keep its node alive: `remember` holds these.
    type Kept: Clone;

    /// Waits until the application can take steps: its window is up and input can reach it.
    async fn ready(&self) -> Result<(), String>;
    fn quit(&self);
    fn windowless(&self) -> bool {
        false
    }
    fn nests_dialogs(&self) -> bool {
        false
    }
    /// The warnings and criticals the toolkit logged since the last call.
    fn criticals(&self) -> Vec<String>;

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
    fn orientation(&self, node: &Self::Node) -> Option<Orientation>;
    /// The text a label shows or a field holds.
    fn text(&self, node: &Self::Node) -> Option<String>;
    /// The content, caret and selection of a text field.
    fn field(&self, node: &Self::Node) -> Option<FieldState>;
    /// The number a slider or another ranged control stands at.
    fn value(&self, node: &Self::Node) -> Option<f64>;

    fn frame(&self, node: &Self::Node) -> Option<Rect>;
    /// The box of a visual sub-part (`EntryPart::Label` or `EntryPart::Icon`) inside an entry
    /// row, in window coordinates, for pointer targeting without synthesizing accessibility nodes.
    fn entry_part_frame(&self, _node: &Self::Node, _part: EntryPart) -> Option<Rect> {
        None
    }
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

    fn entries(&self, container: &Self::Node) -> Vec<EntryRow<Self::Node>> {
        crate::presented(self, container)
            .iter()
            .filter(|node| self.frame(node).is_some())
            .filter_map(|node| self.entry(node))
            .collect()
    }

    fn labels(&self) -> Labels;
    /// The file the application saves its preferences to.
    fn settings_file(&self) -> PathBuf;

    fn keep(&self, node: &Self::Node) -> Self::Kept;
    fn recall(&self, kept: &Self::Kept) -> Option<Self::Node>;

    /// The tree a failed wait reports and the `tree` query returns.
    fn dump(&self, window: &Self::Node) -> String;

    /// Returns once the window has painted a frame, or at once if it has nothing to paint.
    async fn settle(&self, window: &Self::Node);
    /// Lets the application run for this long.
    async fn sleep(&self, time: Duration);

    /// Called before each step that needs a window; an error ends the step.
    fn begin(&self, _window: &Self::Node) -> Result<(), String> {
        Ok(())
    }
    /// Makes `window` the one later steps act on.
    fn adopt(&self, window: &Self::Node);
    async fn activate_window(
        &self,
        _window: &Self::Node,
        _deadline: std::time::Instant,
    ) -> Result<bool, String> {
        Err("activate_window is not supported on this target".to_owned())
    }
    /// Sets the window's size; without a height, it keeps the one it has.
    async fn resize(
        &self,
        window: &Self::Node,
        width: u16,
        height: Option<u16>,
    ) -> Result<(), String>;

    async fn chord(&self, window: &Self::Node, chord: &Chord) -> Result<(), String>;
    /// Types one character, whether or not the keyboard layout has a key for it.
    async fn text_input(&self, window: &Self::Node, character: char) -> Result<(), String>;
    /// Presses the modifiers in order and keeps them down. On an error none stays down.
    async fn hold(&self, window: &Self::Node, modifiers: &[Modifier]) -> Result<(), String>;
    /// Releases the modifiers `hold` pressed, last first.
    async fn release(&self, window: &Self::Node, modifiers: &[Modifier]);
    /// Sends one key event by key or modifier name and waits for nothing.
    async fn key(&self, name: &str, pressed: bool) -> Result<(), String>;

    /// Sends one pointer motion to a point on the screen and waits for nothing.
    async fn pointer_move(&self, x: i16, y: i16) -> Result<(), String>;
    fn pointer_position(&self) -> Result<(i16, i16), String>;
    async fn button(&self, number: u8, pressed: bool) -> Result<(), String>;
    /// Runs the node's default action; false when it has none.
    fn activate(&self, node: &Self::Node) -> bool;

    async fn command(&self, _scope: &Scope<'_, Self>, _command: &Command) -> Option<Outcome>
    where
        Self: Sized,
    {
        None
    }
}
