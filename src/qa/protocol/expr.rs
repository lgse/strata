// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{StateName, Target, yes};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Bound {
    pub(crate) op: CompareOp,
    pub(crate) than: Expr,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Comparison {
    pub(crate) left: Expr,
    pub(crate) op: CompareOp,
    pub(crate) right: Expr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) enum CompareOp {
    #[serde(rename = "<")]
    Less,
    #[serde(rename = "<=")]
    AtMost,
    #[serde(rename = "==")]
    Equal,
    #[serde(rename = "!=")]
    Unequal,
    #[serde(rename = ">=")]
    AtLeast,
    #[serde(rename = ">")]
    Greater,
}

impl CompareOp {
    pub(crate) fn holds(self, left: f64, right: f64) -> bool {
        match self {
            Self::Less => left < right,
            Self::AtMost => left <= right,
            #[expect(clippy::float_cmp, reason = "Python compares the numbers exactly")]
            Self::Equal => left == right,
            #[expect(clippy::float_cmp, reason = "Python compares the numbers exactly")]
            Self::Unequal => left != right,
            Self::AtLeast => left >= right,
            Self::Greater => left > right,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Expr {
    Number(serde_json::Number),
    Of {
        target: Box<Target>,
        get: Measure,
    },
    Pref {
        key: String,
        default: Option<serde_json::Number>,
    },
    Count(Box<CountSpec>),
    Sum(Vec<Expr>),
    Minus(Box<[Expr; 2]>),
    Times(Box<[Expr; 2]>),
    Divide(Box<[Expr; 2]>),
    FloorDivide(Box<[Expr; 2]>),
    Ceil(Box<Expr>),
    Abs(Box<Expr>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Measure {
    X,
    Y,
    Width,
    Height,
    Right,
    Bottom,
    CenterX,
    CenterY,
    Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CountSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) within: Option<Target>,
    #[serde(default = "yes")]
    pub(crate) rendered: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) states: Vec<StateName>,
}

impl Expr {
    fn from_value(value: &Value) -> Result<Self, String> {
        if let Value::Number(number) = value {
            return Ok(Self::Number(number.clone()));
        }
        let Value::Object(object) = value else {
            return Err(format!("a number is a literal or an object, not {value}"));
        };
        let pair = |value: &Value| -> Result<Box<[Expr; 2]>, String> {
            match value.as_array().map(Vec::as_slice) {
                Some([first, second]) => Ok(Box::new([
                    Self::from_value(first)?,
                    Self::from_value(second)?,
                ])),
                _ => Err(format!("an operation takes two numbers, not {value}")),
            }
        };
        let only = |keys: &[&str]| -> Result<(), String> {
            match object.keys().find(|key| !keys.contains(&key.as_str())) {
                Some(key) => Err(format!("a number has no field {key:?} here")),
                None => Ok(()),
            }
        };
        let target = |value: &Value| -> Result<Box<Target>, String> {
            serde_json::from_value(value.clone())
                .map(Box::new)
                .map_err(|error| error.to_string())
        };
        if let Some(of) = object.get("of") {
            only(&["of", "get"])?;
            let get = object.get("get").ok_or("of needs get, such as \"width\"")?;
            return Ok(Self::Of {
                target: target(of)?,
                get: serde_json::from_value(get.clone()).map_err(|error| error.to_string())?,
            });
        }
        if let Some(key) = object.get("pref") {
            only(&["pref", "default"])?;
            return Ok(Self::Pref {
                key: key.as_str().ok_or("pref names a key")?.to_owned(),
                default: match object.get("default") {
                    None => None,
                    Some(Value::Number(number)) => Some(number.clone()),
                    Some(other) => return Err(format!("default is a number, not {other}")),
                },
            });
        }
        let [(key, inner)] = object.iter().collect::<Vec<_>>()[..] else {
            return Err(format!("a number has one operation, not {value}"));
        };
        Ok(match key.as_str() {
            "count" => Self::Count(Box::new(
                serde_json::from_value(inner.clone()).map_err(|error| error.to_string())?,
            )),
            "sum" => Self::Sum(
                inner
                    .as_array()
                    .ok_or("sum takes a list")?
                    .iter()
                    .map(Self::from_value)
                    .collect::<Result<_, _>>()?,
            ),
            "minus" => Self::Minus(pair(inner)?),
            "times" => Self::Times(pair(inner)?),
            "divide" => Self::Divide(pair(inner)?),
            "floor_divide" => Self::FloorDivide(pair(inner)?),
            "ceil" => Self::Ceil(Box::new(Self::from_value(inner)?)),
            "abs" => Self::Abs(Box::new(Self::from_value(inner)?)),
            other => return Err(format!("no number operation {other:?}")),
        })
    }

    pub(crate) fn to_json(&self) -> Value {
        let pair = |items: &[Expr; 2]| json!([items[0].to_json(), items[1].to_json()]);
        match self {
            Self::Number(number) => Value::Number(number.clone()),
            Self::Of { target, get } => json!({ "of": target.to_json(), "get": get }),
            Self::Pref { key, default: None } => json!({ "pref": key }),
            Self::Pref {
                key,
                default: Some(default),
            } => json!({ "pref": key, "default": default }),
            Self::Count(spec) => json!({ "count": spec }),
            Self::Sum(items) => {
                json!({ "sum": items.iter().map(Self::to_json).collect::<Vec<_>>() })
            }
            Self::Minus(items) => json!({ "minus": pair(items) }),
            Self::Times(items) => json!({ "times": pair(items) }),
            Self::Divide(items) => json!({ "divide": pair(items) }),
            Self::FloorDivide(items) => json!({ "floor_divide": pair(items) }),
            Self::Ceil(inner) => json!({ "ceil": inner.to_json() }),
            Self::Abs(inner) => json!({ "abs": inner.to_json() }),
        }
    }
}

impl<'de> Deserialize<'de> for Expr {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::from_value(&Value::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl Serialize for Expr {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_json().serialize(serializer)
    }
}
