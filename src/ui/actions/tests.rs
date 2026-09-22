// SPDX-License-Identifier: MIT

use std::{ffi::OsString, path::PathBuf, rc::Rc};

use super::{ActionHandle, chord_slots};
use crate::model::{ActionDefinition, MenuPlacement};
use crate::services::{ActionAvailability, ActionProgram, MatchedAction};

fn matched(id: &str, name: &str) -> MatchedAction {
    let definition = ActionDefinition::parse(&format!(
        "schema_version = 1\nid = \"{id}\"\nname = \"{name}\"\n\n[when]\n\n[run]\nruntime = \"command\"\nprogram = \"true\"\n"
    ))
    .expect("valid test definition");
    MatchedAction {
        action: Rc::new(ActionHandle {
            definition,
            directory: PathBuf::from("/tmp/actions").join(id),
            availability: ActionAvailability::Available(ActionProgram::Command {
                program: OsString::from("true"),
                arguments: Vec::new(),
            }),
        }),
        placement: MenuPlacement::Submenu,
    }
}

#[test]
fn chord_slots_number_the_first_ten_matches_1_through_9_then_0() {
    let matched: Vec<_> = (1..=11)
        .map(|index| matched(&format!("id-{index:02}"), &format!("Name {index:02}")))
        .collect();
    let slots = chord_slots(&matched);
    assert_eq!(
        slots
            .iter()
            .map(|(key, action)| (*key, action.name().to_owned()))
            .collect::<Vec<_>>(),
        [
            ('1', "Name 01".to_owned()),
            ('2', "Name 02".to_owned()),
            ('3', "Name 03".to_owned()),
            ('4', "Name 04".to_owned()),
            ('5', "Name 05".to_owned()),
            ('6', "Name 06".to_owned()),
            ('7', "Name 07".to_owned()),
            ('8', "Name 08".to_owned()),
            ('9', "Name 09".to_owned()),
            ('0', "Name 10".to_owned()),
        ]
    );
}

#[test]
fn chord_slots_omit_vacant_digits_when_fewer_than_ten_match() {
    let slots = chord_slots(&[matched("first", "First"), matched("second", "Second")]);
    assert_eq!(
        slots
            .iter()
            .map(|(key, action)| (*key, action.name().to_owned()))
            .collect::<Vec<_>>(),
        [('1', "First".to_owned()), ('2', "Second".to_owned())]
    );
}
