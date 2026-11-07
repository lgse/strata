// SPDX-License-Identifier: MIT

use super::conflict_dialog::{ConflictActions, ConflictChoice, ConflictPrompt};
use super::{ConflictFocus, PendingSendToCompletion, SendToTransferContext};
use crate::adapters::TransferTargetCollision;
use crate::model::Location;
use crate::services::{PasteItem, ShownName, TransferConflict};
use crate::ui::browser::ViewState;
use crate::ui::browser::paths::compact_display_path;
use crate::ui::browser::transfer_report::shown_name;
use std::rc::Rc;

#[derive(Clone)]
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
    pub(super) send_to: Option<SendToTransferContext>,
    pub(super) focus: ConflictFocus,
}

fn merge_explanation(collision: &TransferCollision, name: &str, folder: &str) -> String {
    let merging = rust_i18n::t!(
        "A folder named “%{name}” already exists in %{folder}. Merging combines the contents of both folders; incoming items overwrite items with the same name.",
        name = name,
        folder = folder
    );
    if collision.fixes_inside {
        crate::i18n::join_sentences(
            &merging,
            &crate::i18n::tr(
                "Items whose names change to suit this location overwrite items with the new name.",
            ),
        )
    } else {
        merging.into_owned()
    }
}

/// Splits the collisions an Apply to All answer covers from those that still need their own
/// prompt, such as a file meeting a folder after Replace.
pub(super) fn covered_by_answer<T>(
    remaining: Vec<T>,
    conflict: TransferConflict,
    kinds: impl Fn(&T) -> TransferTargetCollision,
) -> (Vec<T>, Vec<T>) {
    remaining
        .into_iter()
        .partition(|collision| kinds(collision).allows(conflict))
}

pub(super) fn unreplaceable_explanation(name: &str, folder: &str) -> String {
    rust_i18n::t!(
        "An item named “%{name}” already exists in %{folder}. A file and a folder can’t replace one another.",
        name = name,
        folder = folder
    )
    .into_owned()
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
        Some(own_name) => crate::i18n::join_sentences(
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
            .and_then(crate::adapters::trash_support::unsupported_reason_for)
            .is_some()
}

fn transfer_conflict_text(
    collision: &TransferCollision,
    plan: &TransferPlan,
    merge: bool,
) -> String {
    let mut explanation = conflict_explanation(collision, &plan.destination, merge);
    // Undo cannot put back a source the merge deleted, so say so before the choice.
    if merge && plan.move_sources {
        explanation = crate::i18n::join_sentences(
            &explanation,
            &crate::i18n::tr("Merging removes the source folder and can’t be undone."),
        );
    }
    if collision.kinds.replaceable && replaces_without_trash(&plan.destination) {
        format!(
            "{explanation}\n\n{}",
            crate::i18n::tr("Replaced items can't be moved to Trash here and will be deleted.")
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

impl ViewState {
    pub(super) fn resolve_transfer_collisions(
        self: &Rc<Self>,
        plan: TransferPlan,
        mut collisions: Vec<TransferCollision>,
        accepted: Vec<PasteItem>,
    ) {
        if collisions.is_empty() {
            self.dispatch_transfer(plan, accepted);
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
        let state = self.clone();
        self.confirm_replace_conflict(
            ConflictPrompt {
                name: &name,
                explanation: &explanation,
                apply_to_all_visible,
                skip_visible,
                actions,
            },
            Rc::new(move |choice, apply_to_all| {
                let (accepted, remaining) = apply_transfer_choice(
                    &collision,
                    (choice, apply_to_all),
                    accepted.clone(),
                    collisions.clone(),
                );
                state.resolve_transfer_collisions(plan.clone(), remaining, accepted);
            }),
        );
    }

    fn dispatch_transfer(&self, plan: TransferPlan, accepted: Vec<PasteItem>) {
        let source_depth = self
            .drag_source_depth
            .replace(None)
            .or_else(|| self.browser.active_depth());
        self.drop_active_depths.set(None);
        if !accepted.is_empty() {
            self.prepare_transfer_dispatch(&plan, source_depth, accepted.len());
        }
        let TransferPlan {
            destination,
            move_sources,
            reveal,
            ..
        } = plan;
        if self.transfer_replaces_cursor.get() {
            self.browser
                .transfer_replacing_cursor(destination, accepted, move_sources, reveal);
        } else {
            self.browser
                .transfer(destination, accepted, move_sources, reveal);
        }
    }

    fn prepare_transfer_dispatch(
        &self,
        plan: &TransferPlan,
        source_depth: Option<usize>,
        item_count: usize,
    ) {
        self.suppress_scroll_after_drop.set(!plan.reveal);
        if !plan.reveal {
            let destination_depth = (0..)
                .map_while(|depth| self.browser.location_at(depth))
                .position(|location| location == plan.destination);
            self.drop_active_depths
                .set(source_depth.zip(destination_depth));
        }
        // Parked jobs own their completion state independently; clear only
        // stale foreground metadata before this dispatch.
        self.pending_send_to_completion.take();
        self.finished_send_to_completion.take();
        if let Some(send_to) = &plan.send_to {
            self.pending_send_to_completion
                .replace(Some(PendingSendToCompletion {
                    device_name: send_to.device_name.clone(),
                    item_count,
                }));
        }
    }
}
