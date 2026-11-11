// SPDX-License-Identifier: MIT

use serde_json::{Value, json};

/// How a `note` cuts and orders the list of names it read.
pub struct Shape<'a> {
    pub from: Option<&'a str>,
    pub through: Option<&'a str>,
    pub middle: Option<usize>,
    pub plus: &'a [String],
    pub sorted: bool,
    pub pick: Option<i64>,
}

impl Shape<'_> {
    pub fn apply(&self, names: Vec<String>) -> Result<Value, String> {
        let index = |name: &str| {
            names
                .iter()
                .position(|candidate| candidate == name)
                .ok_or_else(|| format!("{name:?} is not in {names:?}"))
        };
        let start = self.from.map(index).transpose()?.unwrap_or(0);
        let end = self
            .through
            .map(index)
            .transpose()?
            .map_or(names.len(), |end| end + 1);
        let mut names: Vec<String> = names
            .get(start..end.max(start))
            .unwrap_or_default()
            .to_vec();
        if let Some(count) = self.middle {
            let start = names.len() / 2;
            names = names[start..(start + count).min(names.len())].to_vec();
        }
        if !self.plus.is_empty() {
            names.extend(self.plus.iter().cloned());
            names.sort();
            names.dedup();
        }
        if self.sorted {
            names.sort();
        }
        match self.pick {
            None => Ok(json!(names)),
            Some(pick) => {
                let length = i64::try_from(names.len()).map_err(|error| error.to_string())?;
                let index = if pick < 0 { length + pick } else { pick };
                usize::try_from(index)
                    .ok()
                    .and_then(|index| names.get(index))
                    .map(|name| json!(name))
                    .ok_or_else(|| format!("index {pick} is outside {names:?}"))
            }
        }
    }
}
