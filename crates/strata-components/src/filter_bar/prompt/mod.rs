//! The 10xer footer prompts: their specs, state and element.

mod spec;
mod state;
mod view;

pub use spec::{Arrows, Legend, PromptSpec};
pub use state::{By, Candidate, Candidates, CompletionQuery, Polled, PromptIntent, PromptState};
pub use view::{FooterPrompt, PromptLabels};
