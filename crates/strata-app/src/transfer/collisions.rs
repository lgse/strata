// SPDX-License-Identifier: MIT

//! Destination transfer collision sorting, explanations, and step-by-step resolution through
//! [`crate::conflict_dialog`], matching `src/ui/browser/transfer/collisions.rs`.

use strata_core::{
    i18n::{join_sentences, tr},
    model::Location,
};
use strata_fs::{
    adapters::{TransferTargetCollision, trash_support},
    services::{PasteItem, ShownName, TransferConflict},
};

use crate::{
    App,
    conflict_dialog::{
        self, ConflictActions, ConflictChoice, ConflictFocus, ConflictPrompt, compact_display_path,
        covered_by_answer, shown_name, unreplaceable_explanation,
    },
};

pub(super) type TargetFacts = (Option<ShownName>, Option<TransferTargetCollision>, bool);

#[derive(Clone, Debug)]
pub(super) struct TransferCollision {
    pub(super) source: Location,
    /// The name the paste will use, already held by the colliding item.
    pub(super) target_name: Option<ShownName>,
    pub(super) kinds: TransferTargetCollision,
    pub(super) fixes_inside: bool,
}

impl TransferCollision {
    fn paste_item(self, conflict: TransferConflict) -> PasteItem {
        PasteItem {
            source: self.source,
            conflict,
            target_name: self.target_name,
        }
    }
}

/// The parts of a transfer that stay fixed while its collisions are resolved one by one.
#[derive(Clone)]
pub(super) struct TransferPlan {
    pub(super) destination: Location,
    pub(super) move_sources: bool,
    pub(super) reveal: bool,
    pub(super) focus: ConflictFocus,
}

/// Splits prepared targets into items that go straight through and those that collide.
pub(super) fn sort_transfer_targets(
    sources: Vec<Location>,
    targets: impl IntoIterator<Item = TargetFacts>,
) -> (Vec<PasteItem>, Vec<TransferCollision>) {
    let mut accepted = Vec::new();
    let mut collisions = Vec::new();
    for (source, (target_name, collision, fixes_inside)) in sources.into_iter().zip(targets) {
        match collision {
            Some(collision) => collisions.push(TransferCollision {
                source,
                target_name,
                kinds: collision,
                fixes_inside,
            }),
            None => accepted.push(PasteItem {
                source,
                conflict: TransferConflict::FailIfExists,
                target_name,
            }),
        }
    }
    (accepted, collisions)
}

fn merge_explanation(collision: &TransferCollision, name: &str, folder: &str) -> String {
    let merging = rust_i18n::t!(
        "A folder named “%{name}” already exists in %{folder}. Merging combines the contents of both folders; incoming items overwrite items with the same name.",
        name = name,
        folder = folder
    );
    if collision.fixes_inside {
        join_sentences(
            &merging,
            &tr(
                "Items whose names change to suit this location overwrite items with the new name.",
            ),
        )
    } else {
        merging.into_owned()
    }
}

pub(super) fn conflict_explanation(
    collision: &TransferCollision,
    destination: &Location,
    merge: bool,
) -> String {
    let own_name = collision.source.file_name();
    let target_name = collision
        .target_name
        .as_ref()
        .map(|shown| shown.name.clone())
        .or_else(|| own_name.clone())
        .unwrap_or_default();
    let name = shown_name(&target_name);
    let folder = compact_display_path(destination);
    let choice = if merge {
        merge_explanation(collision, &name, &folder)
    } else if !collision.kinds.replaceable {
        unreplaceable_explanation(&name, &folder)
    } else {
        rust_i18n::t!(
            "An item named “%{name}” already exists in %{folder}. Replacing it will overwrite its contents.",
            name = name,
            folder = folder
        )
        .into_owned()
    };
    match own_name.filter(|own_name| *own_name != target_name) {
        Some(own_name) => join_sentences(
            &rust_i18n::t!(
                "“%{name}” will be saved as “%{new_name}” in this location.",
                name = shown_name(&own_name),
                new_name = name
            ),
            &choice,
        ),
        None => choice,
    }
}

fn replaces_without_trash(destination: &Location) -> bool {
    destination.is_administrator()
        || destination
            .native_path()
            .and_then(trash_support::unsupported_reason_for)
            .is_some()
}

pub(super) fn transfer_conflict_text(
    collision: &TransferCollision,
    plan: &TransferPlan,
    merge: bool,
) -> String {
    let mut explanation = conflict_explanation(collision, &plan.destination, merge);
    // Undo cannot put back a source the merge deleted, so say so before the choice.
    if merge && plan.move_sources {
        explanation = join_sentences(
            &explanation,
            &tr("Merging removes the source folder and can’t be undone."),
        );
    }
    if collision.kinds.replaceable && replaces_without_trash(&plan.destination) {
        format!(
            "{explanation}\n\n{}",
            tr("Replaced items can't be moved to Trash here and will be deleted.")
        )
    } else {
        explanation
    }
}

/// Applies one answer to the current collision and, with Apply to All, to the rest it covers.
fn apply_transfer_choice(
    current: &TransferCollision,
    (choice, apply_to_all): (ConflictChoice, bool),
    mut accepted: Vec<PasteItem>,
    mut remaining: Vec<TransferCollision>,
) -> (Vec<PasteItem>, Vec<TransferCollision>) {
    let conflict = match choice {
        ConflictChoice::Skip => {
            if apply_to_all {
                remaining.clear();
            }
            return (accepted, remaining);
        }
        ConflictChoice::Replace => TransferConflict::ReplaceExisting,
        ConflictChoice::Merge => TransferConflict::Merge,
        ConflictChoice::KeepBoth => TransferConflict::KeepBoth,
    };
    accepted.push(current.clone().paste_item(conflict));
    if !apply_to_all {
        return (accepted, remaining);
    }
    let (covered, rest) = covered_by_answer(remaining, conflict, |collision| collision.kinds);
    accepted.extend(
        covered
            .into_iter()
            .map(|collision| collision.paste_item(conflict)),
    );
    (accepted, rest)
}

pub(super) fn resolve(
    app: &mut App,
    plan: TransferPlan,
    mut collisions: Vec<TransferCollision>,
    accepted: Vec<PasteItem>,
) {
    if collisions.is_empty() {
        if !accepted.is_empty() {
            app.browser
                .transfer(plan.destination, accepted, plan.move_sources, plan.reveal);
        }
        return;
    }
    let collision = collisions.remove(0);
    let name = collision.source.display_name();
    // Merge stages what it overwrites in Trash, which `admin://` lacks.
    let allow_merge = collision.kinds.mergeable && !plan.destination.is_administrator();
    let explanation = transfer_conflict_text(&collision, &plan, allow_merge);
    let (apply_to_all_visible, skip_visible) =
        ConflictPrompt::visibility(collisions.len(), accepted.len());
    let actions = ConflictActions {
        replace: collision.kinds.replaceable,
        keep_both: true,
        merge: allow_merge,
        focus: plan.focus,
    };
    conflict_dialog::prompt(
        app,
        ConflictPrompt {
            name,
            explanation,
            apply_to_all_visible,
            skip_visible,
            actions,
        },
        move |app, choice, apply_to_all| {
            let (accepted, remaining) = apply_transfer_choice(
                &collision,
                (choice, apply_to_all),
                accepted.clone(),
                collisions.clone(),
            );
            resolve(app, plan.clone(), remaining, accepted);
        },
    );
}
