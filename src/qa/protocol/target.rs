// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{At, Offset, yes};

pub(crate) const TARGET_KINDS: [&str; 27] = [
    "entry",
    "row",
    "result",
    "result_row",
    "focused_widget",
    "remembered",
    "search_result",
    "role",
    "named",
    "background",
    "pane",
    "viewport",
    "container",
    "field",
    "sidebar",
    "header",
    "menu_item",
    "dialog_button",
    "dialog",
    "window",
    "preview",
    "parent",
    "ancestor",
    "kept",
    "point",
    "toplevel",
    "other_window",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Entry(EntryTarget),
    Role {
        role: String,
        other_roles: Vec<String>,
        name: Option<String>,
        description: Option<String>,
        within: Option<Box<Target>>,
        name_ends_with: Vec<String>,
        nth: Option<i64>,
        states: Vec<StateName>,
        name_matches: Option<String>,
        has: Option<Box<Target>>,
        rendered: bool,
    },
    Row(String),
    Background {
        dir: Option<String>,
    },
    Pane {
        dir: Option<String>,
    },
    Viewport {
        dir: Option<String>,
    },
    Field(Option<String>),
    FieldWithin(Box<Target>),
    Toplevel(String),
    OtherWindow(String),
    Result(String),
    ResultRow {
        index: i64,
        containing: Option<String>,
        name: Option<String>,
    },
    FocusedWidget,
    Remembered(String),
    Sidebar(String),
    Header(String),
    MenuItem(String),
    DialogButton(String),
    Window {
        y_of: Option<Box<Target>>,
    },
    Preview,
    SearchResult(String),
    Named {
        name: String,
        within: Box<Target>,
    },
    Container {
        dir: Option<String>,
    },
    Dialog,
    Parent(Box<Target>),
    Ancestor {
        role: String,
        of: Box<Target>,
    },
    Kept(String),
    Point(Box<[Axis; 2]>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Axis {
    Of {
        target: Box<Target>,
        at: Option<Offset>,
        plus: i32,
    },
    Mid {
        pair: Box<[Axis; 2]>,
        plus: i32,
    },
    Max {
        pair: Box<[Axis; 2]>,
        plus: i32,
    },
    Extent {
        target: Box<Target>,
        plus: i32,
    },
}

impl Axis {
    fn from_value(value: &Value) -> Result<Self, String> {
        let Value::Object(object) = value else {
            return Err(format!("a point coordinate is an object, not {value}"));
        };
        let plus = match object.get("plus") {
            None => 0,
            Some(plus) => plus
                .as_i64()
                .and_then(|plus| i32::try_from(plus).ok())
                .ok_or_else(|| format!("plus is whole pixels, not {plus}"))?,
        };
        let pair = |value: &Value| -> Result<Box<[Axis; 2]>, String> {
            match value.as_array().map(Vec::as_slice) {
                Some([first, second]) => Ok(Box::new([
                    Self::from_value(first)?,
                    Self::from_value(second)?,
                ])),
                _ => Err(format!("mid and max take two coordinates, not {value}")),
            }
        };
        let known = ["of", "at", "plus", "mid", "max", "extent"];
        if let Some(key) = object.keys().find(|key| !known.contains(&key.as_str())) {
            return Err(format!("a point coordinate has no field {key:?}"));
        }
        if let Some(target) = object.get("extent") {
            if ["of", "at", "mid", "max"]
                .iter()
                .any(|key| object.contains_key(*key))
            {
                return Err("extent takes only a target and plus".to_owned());
            }
            return Ok(Self::Extent {
                target: Box::new(
                    serde_json::from_value(target.clone()).map_err(|error| error.to_string())?,
                ),
                plus,
            });
        }
        match (object.get("of"), object.get("mid"), object.get("max")) {
            (Some(target), None, None) => Ok(Self::Of {
                target: Box::new(
                    serde_json::from_value(target.clone()).map_err(|error| error.to_string())?,
                ),
                at: object.get("at").map(Offset::parse).transpose()?,
                plus,
            }),
            (None, Some(both), None) if !object.contains_key("at") => Ok(Self::Mid {
                pair: pair(both)?,
                plus,
            }),
            (None, None, Some(both)) if !object.contains_key("at") => Ok(Self::Max {
                pair: pair(both)?,
                plus,
            }),
            _ => Err(
                "a point coordinate is {\"of\":TARGET,\"at\":OFFSET}, {\"mid\":[A,B]}, or \
                 {\"max\":[A,B]}, each with an optional plus"
                    .to_owned(),
            ),
        }
    }

    fn to_json(&self) -> Value {
        let (mut object, plus) = match self {
            Self::Of { target, at, plus } => {
                let mut object = Map::new();
                object.insert("of".to_owned(), target.to_json());
                if let Some(at) = at {
                    object.insert("at".to_owned(), at.to_json());
                }
                (object, *plus)
            }
            Self::Mid { pair, plus } | Self::Max { pair, plus } => {
                let key = if matches!(self, Self::Mid { .. }) {
                    "mid"
                } else {
                    "max"
                };
                let mut object = Map::new();
                object.insert(
                    key.to_owned(),
                    json!([pair[0].to_json(), pair[1].to_json()]),
                );
                (object, *plus)
            }
            Self::Extent { target, plus } => {
                let mut object = Map::new();
                object.insert("extent".to_owned(), target.to_json());
                (object, *plus)
            }
        };
        if plus != 0 {
            object.insert("plus".to_owned(), json!(plus));
        }
        Value::Object(object)
    }
}

impl Target {
    pub(super) fn accepts(&self, at: Option<&At>) -> Result<(), String> {
        if at.is_some() && matches!(self, Self::Background { .. }) {
            return Err("at does not apply to a background target, which is a point".to_owned());
        }
        Ok(())
    }

    fn from_object(object: Map<String, Value>) -> Result<Self, String> {
        let kinds: Vec<&str> = TARGET_KINDS
            .into_iter()
            .filter(|kind| object.contains_key(*kind))
            .collect();
        let [kind] = kinds.as_slice() else {
            return Err(format!(
                "a target needs exactly one of {TARGET_KINDS:?}, found {kinds:?}"
            ));
        };
        let value = Value::Object(object);
        let invalid = |error: serde_json::Error| format!("invalid {kind} target: {error}");
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RoleFields {
            role: Value,
            name: Option<String>,
            description: Option<String>,
            within: Option<Target>,
            #[serde(default)]
            name_ends_with: Vec<String>,
            nth: Option<i64>,
            #[serde(default)]
            states: Vec<StateName>,
            name_matches: Option<String>,
            has: Option<Target>,
            #[serde(default = "yes")]
            rendered: bool,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct BackgroundFields {
            background: bool,
            dir: Option<String>,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct WindowFields {
            window: bool,
            y_of: Option<Target>,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct PreviewFields {
            preview: bool,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct DialogFields {
            dialog: bool,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct NamedFields {
            named: String,
            within: Target,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct ResultRowFields {
            result_row: i64,
            containing: Option<String>,
            name: Option<String>,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct FocusedWidgetFields {
            focused_widget: bool,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct AncestorFields {
            ancestor: String,
            of: Target,
        }
        let directory = |value: Value| -> Result<Option<String>, String> {
            let mut object = value.as_object().cloned().unwrap_or_default();
            if object.len() != 1 {
                return Err(format!("invalid {kind} target: it takes only {kind}"));
            }
            serde_json::from_value(object.remove(*kind).unwrap_or(Value::Null)).map_err(invalid)
        };
        let label = |value: Value| -> Result<String, String> {
            directory(value)?.ok_or_else(|| format!("invalid {kind} target: {kind} needs a label"))
        };
        match *kind {
            "entry" => serde_json::from_value(value)
                .map(Self::Entry)
                .map_err(invalid),
            "role" => {
                let fields: RoleFields = serde_json::from_value(value).map_err(invalid)?;
                let roles: Vec<String> = match &fields.role {
                    Value::String(role) => vec![role.clone()],
                    Value::Array(_) => {
                        serde_json::from_value(fields.role.clone()).map_err(invalid)?
                    }
                    _ => Vec::new(),
                };
                let Some((role, other_roles)) = roles.split_first() else {
                    return Err(
                        "invalid role target: role is a role name or a list of them".to_owned()
                    );
                };
                if let Some(pattern) = &fields.name_matches {
                    regex::Regex::new(pattern).map_err(|error| {
                        format!("invalid role target: name_matches {pattern:?}: {error}")
                    })?;
                }
                if let Some(has) = &fields.has
                    && !matches!(
                        has,
                        Self::Role {
                            within: None,
                            has: None,
                            rendered: true,
                            ..
                        }
                    )
                {
                    return Err(
                        "invalid role target: has takes a rendered role target without within \
                         or has"
                            .to_owned(),
                    );
                }
                if role == "*" && !other_roles.is_empty() {
                    return Err("invalid role target: \"*\" stands alone".to_owned());
                }
                Ok(Self::Role {
                    role: role.clone(),
                    other_roles: other_roles.to_vec(),
                    name: fields.name,
                    description: fields.description,
                    within: fields.within.map(Box::new),
                    name_ends_with: fields.name_ends_with,
                    nth: fields.nth,
                    states: fields.states,
                    name_matches: fields.name_matches,
                    has: fields.has.map(Box::new),
                    rendered: fields.rendered,
                })
            }
            "background" => {
                let fields: BackgroundFields = serde_json::from_value(value).map_err(invalid)?;
                if !fields.background {
                    return Err("invalid background target: background must be true".to_owned());
                }
                Ok(Self::Background { dir: fields.dir })
            }
            "field" => {
                let object = value.as_object().cloned().unwrap_or_default();
                match (object.get("field"), object.get("within"), object.len()) {
                    (Some(Value::Bool(true)), None, 1) => Ok(Self::Field(None)),
                    (Some(Value::Bool(true)), Some(within), 2) => {
                        serde_json::from_value(within.clone())
                            .map(|within| Self::FieldWithin(Box::new(within)))
                            .map_err(invalid)
                    }
                    (Some(Value::String(name)), None, 1) if !name.is_empty() => {
                        Ok(Self::Field(Some(name.clone())))
                    }
                    _ => Err(
                        "invalid field target: field is true, with an optional within, \
                         or the name a remember gave it"
                            .to_owned(),
                    ),
                }
            }
            "toplevel" => label(value).map(Self::Toplevel),
            "row" => label(value).map(Self::Row),
            "other_window" => label(value).map(Self::OtherWindow),
            "result" => label(value).map(Self::Result),
            "result_row" => {
                let fields: ResultRowFields = serde_json::from_value(value).map_err(invalid)?;
                if fields.containing.is_some() && fields.name.is_some() {
                    return Err(
                        "invalid result_row target: containing or name, not both".to_owned()
                    );
                }
                Ok(Self::ResultRow {
                    index: fields.result_row,
                    containing: fields.containing,
                    name: fields.name,
                })
            }
            "focused_widget" => {
                let fields: FocusedWidgetFields = serde_json::from_value(value).map_err(invalid)?;
                if !fields.focused_widget {
                    return Err(
                        "invalid focused_widget target: focused_widget must be true".to_owned()
                    );
                }
                Ok(Self::FocusedWidget)
            }
            "remembered" => label(value).map(Self::Remembered),
            "pane" => directory(value).map(|dir| Self::Pane { dir }),
            "viewport" => directory(value).map(|dir| Self::Viewport { dir }),
            "sidebar" => label(value).map(Self::Sidebar),
            "header" => label(value).map(Self::Header),
            "menu_item" => label(value).map(Self::MenuItem),
            "window" => {
                let fields: WindowFields = serde_json::from_value(value).map_err(invalid)?;
                if !fields.window {
                    return Err("invalid window target: window must be true".to_owned());
                }
                Ok(Self::Window {
                    y_of: fields.y_of.map(Box::new),
                })
            }
            "preview" => {
                let fields: PreviewFields = serde_json::from_value(value).map_err(invalid)?;
                if !fields.preview {
                    return Err("invalid preview target: preview must be true".to_owned());
                }
                Ok(Self::Preview)
            }
            "dialog" => {
                let fields: DialogFields = serde_json::from_value(value).map_err(invalid)?;
                if !fields.dialog {
                    return Err("invalid dialog target: dialog must be true".to_owned());
                }
                Ok(Self::Dialog)
            }
            "search_result" => label(value).map(Self::SearchResult),
            "named" => {
                let fields: NamedFields = serde_json::from_value(value).map_err(invalid)?;
                Ok(Self::Named {
                    name: fields.named,
                    within: Box::new(fields.within),
                })
            }
            "container" => directory(value).map(|dir| Self::Container { dir }),
            "parent" => {
                let object = value.as_object().cloned().unwrap_or_default();
                if object.len() != 1 {
                    return Err("invalid parent target: it takes only parent".to_owned());
                }
                serde_json::from_value(object["parent"].clone())
                    .map(|of| Self::Parent(Box::new(of)))
                    .map_err(invalid)
            }
            "ancestor" => {
                let fields: AncestorFields = serde_json::from_value(value).map_err(invalid)?;
                Ok(Self::Ancestor {
                    role: fields.ancestor,
                    of: Box::new(fields.of),
                })
            }
            "kept" => label(value).map(Self::Kept),
            "point" => {
                let object = value.as_object().cloned().unwrap_or_default();
                if object.len() != 1 {
                    return Err("invalid point target: it takes only point".to_owned());
                }
                match object["point"].as_array().map(Vec::as_slice) {
                    Some([x, y]) => Ok(Self::Point(Box::new([
                        Axis::from_value(x)
                            .map_err(|error| format!("invalid point target: x: {error}"))?,
                        Axis::from_value(y)
                            .map_err(|error| format!("invalid point target: y: {error}"))?,
                    ]))),
                    _ => Err("invalid point target: point is [x, y]".to_owned()),
                }
            }
            _ => label(value).map(Self::DialogButton),
        }
    }

    pub(crate) fn to_json(&self) -> Value {
        match self {
            Self::Entry(entry) => json!(entry),
            Self::Role {
                role,
                other_roles,
                name,
                description,
                within,
                name_ends_with,
                nth,
                states,
                name_matches,
                has,
                rendered,
            } => {
                let mut object = Map::new();
                if let Some(has) = has {
                    object.insert("has".to_owned(), has.to_json());
                }
                if !rendered {
                    object.insert("rendered".to_owned(), json!(false));
                }
                if other_roles.is_empty() {
                    object.insert("role".to_owned(), json!(role));
                } else {
                    let mut roles = vec![role.clone()];
                    roles.extend(other_roles.iter().cloned());
                    object.insert("role".to_owned(), json!(roles));
                }
                if !states.is_empty() {
                    object.insert("states".to_owned(), json!(states));
                }
                if let Some(pattern) = name_matches {
                    object.insert("name_matches".to_owned(), json!(pattern));
                }
                if let Some(name) = name {
                    object.insert("name".to_owned(), json!(name));
                }
                if let Some(description) = description {
                    object.insert("description".to_owned(), json!(description));
                }
                if let Some(within) = within {
                    object.insert("within".to_owned(), within.to_json());
                }
                if !name_ends_with.is_empty() {
                    object.insert("name_ends_with".to_owned(), json!(name_ends_with));
                }
                if let Some(nth) = nth {
                    object.insert("nth".to_owned(), json!(nth));
                }
                Value::Object(object)
            }
            Self::Background { dir: None } => json!({ "background": true }),
            Self::Background { dir } => json!({ "background": true, "dir": dir }),
            Self::Pane { dir } => json!({ "pane": dir }),
            Self::Viewport { dir } => json!({ "viewport": dir }),
            Self::Field(None) => json!({ "field": true }),
            Self::Field(Some(name)) => json!({ "field": name }),
            Self::FieldWithin(within) => json!({ "field": true, "within": within.to_json() }),
            Self::Toplevel(title) => json!({ "toplevel": title }),
            Self::Row(name) => json!({ "row": name }),
            Self::OtherWindow(title) => json!({ "other_window": title }),
            Self::Result(path) => json!({ "result": path }),
            Self::ResultRow {
                index,
                containing,
                name,
            } => {
                let mut object = Map::new();
                object.insert("result_row".to_owned(), json!(index));
                if let Some(containing) = containing {
                    object.insert("containing".to_owned(), json!(containing));
                }
                if let Some(name) = name {
                    object.insert("name".to_owned(), json!(name));
                }
                Value::Object(object)
            }
            Self::FocusedWidget => json!({ "focused_widget": true }),
            Self::Remembered(name) => json!({ "remembered": name }),
            Self::Sidebar(name) => json!({ "sidebar": name }),
            Self::Header(name) => json!({ "header": name }),
            Self::MenuItem(name) => json!({ "menu_item": name }),
            Self::DialogButton(name) => json!({ "dialog_button": name }),
            Self::Window { y_of: None } => json!({ "window": true }),
            Self::Window { y_of: Some(target) } => {
                json!({ "window": true, "y_of": target.to_json() })
            }
            Self::Preview => json!({ "preview": true }),
            Self::SearchResult(name) => json!({ "search_result": name }),
            Self::Named { name, within } => json!({ "named": name, "within": within.to_json() }),
            Self::Container { dir } => json!({ "container": dir }),
            Self::Dialog => json!({ "dialog": true }),
            Self::Parent(of) => json!({ "parent": of.to_json() }),
            Self::Ancestor { role, of } => json!({ "ancestor": role, "of": of.to_json() }),
            Self::Kept(name) => json!({ "kept": name }),
            Self::Point(axes) => json!({ "point": [axes[0].to_json(), axes[1].to_json()] }),
        }
    }
}

impl<'de> Deserialize<'de> for Target {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Value::deserialize(deserializer)? {
            Value::Object(object) => Self::from_object(object),
            other => Err(format!(
                "a target is an object such as {{\"entry\":\"todo.txt\"}}, not {other}"
            )),
        }
        .map_err(serde::de::Error::custom)
    }
}

impl Serialize for Target {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_json().serialize(serializer)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StateName {
    Selected,
    Focused,
    Checked,
    Pressed,
    Sensitive,
    Selectable,
    Editable,
    Busy,
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EntryTarget {
    pub(crate) entry: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) part: Option<EntryPart>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EntryPart {
    Label,
    Icon,
    Metadata,
}
