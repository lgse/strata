// SPDX-License-Identifier: MIT

mod state;
mod view;

#[cfg(test)]
mod tests;

pub use state::{FormPage, Mounting, TabbedFormState};
pub use view::{
    FormLabels, LABEL_REST_OPACITY, LABEL_WEIGHT, MARK, RING_ALPHA, RULE_ALPHA, STRIP_PAD_X,
    TAB_GAP, TAB_HEIGHT, TAB_PAD_X, TabbedForm, UNDERLINE,
};
