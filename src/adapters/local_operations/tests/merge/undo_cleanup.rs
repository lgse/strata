// SPDX-License-Identifier: MIT

use super::*;

mod aside;
mod compares;
mod no_trash_undo;
mod recording;
mod view;

fn aside_folder(target: &Path) -> Result<Option<String>, io::Error> {
    Ok(names(target)?
        .into_iter()
        .find(|name| name.starts_with(".strata-merge-")))
}

fn merge_through_trash(
    source: &Path,
    target: &Path,
    move_source: bool,
    copy_into_target: CopyStep,
) -> (Result<(), glib::Error>, Vec<MergePlan>) {
    let (plans, on_merged) = recorded_plans();
    let result = glib::MainContext::default().block_on(merge_local_with(
        gio::File::for_path(source),
        gio::File::for_path(target),
        move_source,
        gio::Cancellable::new(),
        None,
        MergeHooks {
            names: NamePolicy::default(),
            copy_into_target,
            stage_overwrite: Rc::new(trash_stage_overwrite),
            on_merged: &on_merged,
            progress: None,
        },
    ));
    (result, plans.take())
}
