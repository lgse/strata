// SPDX-License-Identifier: MIT

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    At, Bound, Chord, DRAG_STEPS, EntryTarget, Expr, Modifier, Names, Predicate, StateName, Target,
    TextMatch, WHEEL_CLICKS, yes,
};

mod parse;

pub(crate) use parse::parse_command;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Command {
    Key {
        chord: Chord,
        repeat: u32,
    },
    Type(String),
    Click {
        target: Target,
        button: u8,
        count: u32,
        modifiers: Vec<Modifier>,
        at: Option<At>,
        left_bound: Option<String>,
        release_modifiers_first: bool,
    },
    ClickResultRows {
        states: RowStates,
        modifiers: Vec<Modifier>,
    },
    Select(EntryTarget),
    SelectWithKeyboard {
        name: String,
        message: Option<String>,
    },
    OpenMenu {
        target: Target,
        until: Predicate,
        message: Option<String>,
        open_states: Vec<StateName>,
    },
    Adopt {
        title: String,
        timeout: Duration,
        message: Option<String>,
    },
    Keep {
        rect: [i64; 4],
        name: String,
    },
    RevealControl {
        target: Target,
    },
    Reveal {
        target: Target,
    },
    Activate {
        target: Target,
        message: Option<String>,
    },
    Holds(Predicate),
    Remember {
        target: Target,
        name: String,
    },
    Note {
        value: Accessor,
        dir: Option<String>,
        wait: Option<Duration>,
        message: Option<String>,
        from: Option<String>,
        through: Option<String>,
        unless_first: Option<String>,
        of: Option<Target>,
        middle: Option<usize>,
        plus: Vec<String>,
        sorted: bool,
        pick: Option<i64>,
        expr: Option<Expr>,
        until: Vec<Bound>,
        until_names: Option<Names>,
    },
    Move {
        target: Target,
        at: Option<At>,
        left_bound: Option<String>,
    },
    MoveBy {
        x: i16,
        y: i16,
    },
    Wheel {
        target: Target,
        at: Option<At>,
        clicks: u32,
        down: bool,
        modifiers: Vec<Modifier>,
        until: Option<Repeat>,
    },
    KeysUntil {
        predicate: Predicate,
        keys: Vec<Chord>,
        times: u32,
        check_first: bool,
        await_focus_change: Option<(Duration, Option<String>)>,
        exhausted: Option<String>,
    },
    KeyDown(String),
    KeyUp(String),
    During {
        checks: Vec<(Predicate, Option<String>)>,
        duration: Duration,
        every: Duration,
    },
    Capture {
        target: Target,
        inset: u16,
        name: String,
    },
    MeasureText {
        kept: String,
        name: String,
    },
    DropSamples {
        kept: String,
        resting: String,
        dragging: String,
        escape: bool,
        name: String,
    },
    Drag {
        from: Target,
        to: Target,
        from_at: Option<At>,
        to_at: Option<At>,
        steps: u32,
        release: bool,
        modifiers: Vec<Modifier>,
        after_press: Vec<PressCheck>,
    },
    ButtonDown(u8),
    ButtonUp(u8),
    SetScrollbar {
        orientation: Orientation,
        value: f64,
    },
    Sleep(Duration),
    Resize {
        width: u16,
        height: Option<u16>,
    },
    Settle {
        target: Target,
        timeout: Duration,
        name: Option<String>,
    },
    Wait {
        predicate: Predicate,
        timeout: Duration,
        message: Option<String>,
        samples: usize,
        settle: Option<Target>,
        never: Option<Box<(Predicate, Option<String>)>>,
    },
    Expect {
        predicates: Vec<Predicate>,
        message: Option<String>,
        timeout: Option<Duration>,
        hold: Option<Duration>,
    },
    ClipboardSet(String),
    Clipboard {
        text: TextMatch,
        message: Option<String>,
    },
    Query(Query),
}

pub(crate) const COMMANDS: [&str; 37] = [
    "key",
    "adopt",
    "keep",
    "reveal_control",
    "reveal",
    "clipboard_set",
    "clipboard",
    "click_result_rows",
    "keys_until",
    "key_down",
    "key_up",
    "during",
    "capture",
    "measure_text",
    "drop_samples",
    "type",
    "click",
    "select",
    "select_with_keyboard",
    "open_menu",
    "activate",
    "holds",
    "remember",
    "note",
    "move",
    "move_by",
    "wheel",
    "drag",
    "button_down",
    "button_up",
    "set_scrollbar",
    "sleep",
    "resize",
    "settle",
    "wait",
    "expect",
    "query",
];

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Repeat {
    pub(crate) predicate: Predicate,
    pub(crate) tries: Option<u32>,
    pub(crate) timeout: Duration,
    pub(crate) message: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PressCheck {
    pub(crate) predicate: Predicate,
    pub(crate) wait: Option<Duration>,
    pub(crate) message: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PressStepFields {
    wait: Option<Predicate>,
    expect: Option<Predicate>,
    timeout_ms: Option<u64>,
    message: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeysUntilFields {
    keys_until: Predicate,
    keys: Vec<String>,
    times: Value,
    #[serde(default)]
    check_first: bool,
    await_focus_change: Option<AwaitChange>,
    exhausted: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AwaitChange {
    timeout_ms: u64,
    message: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdoptFields {
    adopt: String,
    timeout_ms: Option<u64>,
    message: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeepFields {
    keep: [i64; 4],
    #[serde(rename = "as")]
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevealControlFields {
    reveal_control: Target,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevealFields {
    reveal: Target,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyDownFields {
    key_down: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyUpFields {
    key_up: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DuringFields {
    during: Value,
    ms: u64,
    #[serde(default = "poll_ms")]
    every_ms: u64,
    message: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureFields {
    capture: Target,
    #[serde(default)]
    inset: u16,
    #[serde(rename = "as")]
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MeasureTextFields {
    measure_text: String,
    #[serde(rename = "as")]
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DropSamplesFields {
    drop_samples: String,
    resting: String,
    dragging: String,
    #[serde(default)]
    escape: bool,
    #[serde(rename = "as")]
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyFields {
    key: String,
    #[serde(default = "one")]
    repeat: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TypeFields {
    #[serde(rename = "type")]
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectWithKeyboardFields {
    select_with_keyboard: String,
    message: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenMenuFields {
    open_menu: Target,
    until: Predicate,
    message: Option<String>,
    open_states: Option<Vec<StateName>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ActivateFields {
    activate: Target,
    message: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HoldsFields {
    holds: Predicate,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RememberFields {
    remember: Target,
    #[serde(rename = "as")]
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoteFields {
    note: Accessor,
    /// The runner keeps the result under this name; the process ignores it.
    #[serde(rename = "as")]
    name: String,
    dir: Option<String>,
    #[serde(default)]
    until_found: bool,
    timeout_ms: Option<u64>,
    message: Option<String>,
    from: Option<String>,
    through: Option<String>,
    unless_first: Option<String>,
    of: Option<Target>,
    middle: Option<usize>,
    #[serde(default)]
    plus: Vec<String>,
    #[serde(default)]
    sorted: bool,
    pick: Option<i64>,
    expr: Option<Expr>,
    #[serde(default)]
    until: Vec<Bound>,
    until_names: Option<Names>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClickFields {
    click: Target,
    #[serde(default = "first_button")]
    button: u8,
    #[serde(default = "one")]
    count: u32,
    #[serde(default)]
    mods: Vec<String>,
    at: Option<At>,
    left_bound: Option<String>,
    #[serde(default)]
    release_mods_first: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClickResultRowsFields {
    click_result_rows: RowStates,
    #[serde(default)]
    mods: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RowStates {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) selected: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) focused: Option<bool>,
}

impl RowStates {
    pub(crate) fn matches(&self, selected: bool, focused: bool) -> bool {
        self.selected.is_none_or(|wanted| wanted == selected)
            && self.focused.is_none_or(|wanted| wanted == focused)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectFields {
    select: EntryTarget,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveFields {
    #[serde(rename = "move")]
    target: Target,
    at: Option<At>,
    left_bound: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveByFields {
    move_by: [i16; 2],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ButtonDownFields {
    button_down: u8,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetScrollbarFields {
    set_scrollbar: Orientation,
    value: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SleepFields {
    sleep: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Orientation {
    Horizontal,
    Vertical,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WheelFields {
    wheel: Target,
    at: Option<At>,
    #[serde(default = "wheel_clicks")]
    clicks: u32,
    #[serde(default)]
    direction: WheelDirection,
    #[serde(default)]
    mods: Vec<String>,
    until: Option<Predicate>,
    tries: Option<u32>,
    timeout_ms: Option<u64>,
    message: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WheelDirection {
    #[default]
    Down,
    Up,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DragFields {
    drag: DragEnds,
    #[serde(default = "drag_steps")]
    steps: u32,
    #[serde(default = "yes")]
    release: bool,
    #[serde(default)]
    mods: Vec<String>,
    after_press: Option<Value>,
    message: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DragEnds {
    from: Target,
    to: Target,
    from_at: Option<At>,
    to_at: Option<At>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ButtonUpFields {
    button_up: u8,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResizeFields {
    resize: Size,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Size {
    width: u16,
    height: Option<u16>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SettleFields {
    settle: Target,
    timeout_ms: Option<u64>,
    #[serde(rename = "as")]
    name: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitFields {
    wait: Predicate,
    timeout_ms: Option<u64>,
    message: Option<String>,
    #[serde(default = "one_sample")]
    samples: usize,
    settling: Option<Target>,
    never: Option<Predicate>,
    never_message: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DuringCheck {
    check: Predicate,
    message: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectFields {
    expect: Value,
    message: Option<String>,
    timeout_ms: Option<u64>,
    hold_ms: Option<u64>,
    /// The runner keeps this flag for a later `if` or `unless`; the process ignores it.
    #[serde(rename = "as")]
    _name: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClipboardSetFields {
    clipboard_set: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClipboardFields {
    clipboard: TextMatch,
    message: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryFields {
    query: Query,
}

fn one() -> u32 {
    1
}

fn one_sample() -> usize {
    1
}

/// How often a wait polls, in milliseconds.
fn poll_ms() -> u64 {
    50
}

fn first_button() -> u8 {
    1
}

fn wheel_clicks() -> u32 {
    WHEEL_CLICKS
}

fn drag_steps() -> u32 {
    DRAG_STEPS
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Query {
    State,
    Tree,
    Prefs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Accessor {
    Directory,
    Pane,
    Panes,
    Entries,
    Selected,
    AllSelected,
    Matches,
    Focused,
    FocusedPane,
    MiddleVisible,
    Visible,
    ScrollTarget,
    Bounds,
    EditableFields,
    MenuItems,
    Number,
    VisibleTop,
}

impl Accessor {
    pub(crate) fn name(self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default()
    }

    pub(crate) fn takes_directory(self) -> bool {
        matches!(
            self,
            Self::Entries | Self::Selected | Self::Matches | Self::Visible | Self::ScrollTarget
        )
    }

    pub(crate) fn is_list(self) -> bool {
        matches!(
            self,
            Self::Panes
                | Self::Entries
                | Self::Selected
                | Self::AllSelected
                | Self::Matches
                | Self::Visible
                | Self::MenuItems
                | Self::VisibleTop
        )
    }
}
