// SPDX-License-Identifier: MIT

use super::*;

mod contents;
mod names;
mod races;
mod read_only;
mod rejected;

type MakeCopy = fn() -> CopyStep;

fn engine_copy(names: NamePolicy) -> CopyStep {
    super::super::super::merge_copy(None, names)
}

fn gio_merge_copy(names: NamePolicy) -> CopyStep {
    Rc::new(
        move |source, target, _: super::super::super::MergeCopy, cancellable| {
            let names = super::super::super::GioNames::new(names.clone(), Default::default());
            Box::pin(async move {
                super::super::super::copy_recursively_with_progress(
                    source,
                    target,
                    super::super::super::CopyInto::Existing,
                    cancellable,
                    super::super::super::CopyRecord::Nothing,
                    None,
                    names,
                )
                .await
                .map(|()| MergeCopied::default())
            })
        },
    )
}

fn no_plan(_: MergePlan) {}

fn visible_names(folder: &Path) -> io::Result<Vec<String>> {
    Ok(names(folder)?
        .into_iter()
        .filter(|name| !name.starts_with(".strata-"))
        .collect())
}
