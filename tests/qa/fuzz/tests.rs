// SPDX-License-Identifier: MIT

//! The fuzz driver's own tests. None starts the application: the generator, the projection,
//! the rest rule, the comparator and the shrinker are held to what they promise on values
//! made in the tests, and the tree's alphabets, map, mask and corpus are loaded as a run
//! loads them.

use super::{
    compare_tests::{
        a_divergence_that_does_not_repeat_is_not_confirmed,
        a_perturbation_waits_for_its_step_and_its_state,
        a_step_not_at_rest_is_compared_by_its_outcome_alone,
        a_warning_or_a_lost_target_diverges_even_when_both_replays_show_it,
        the_comparator_reports_the_first_field_outside_the_map_and_the_mask,
        the_map_and_the_mask_take_one_sorted_entry_to_a_line,
        the_shrinker_keeps_the_fewest_steps_that_still_diverge_and_confirms_them,
    },
    generate_tests::{
        a_fuzz_command_names_what_it_needs_and_refuses_the_rest,
        a_seed_writes_the_same_sequence_on_every_machine,
        a_sequence_is_scenario_text_and_takes_only_steps_that_act,
        an_alphabet_is_refused_for_what_a_replay_could_not_run,
        the_tree_s_alphabets_map_mask_and_corpus_load_as_a_run_loads_them,
    },
    observe_tests::{
        a_field_is_named_by_its_path_and_an_entry_covers_what_is_below_it,
        a_replay_ends_where_the_application_leaves_the_fixture,
        a_stream_keeps_its_observations_through_its_file,
        rest_is_equal_samples_in_a_row_and_the_budget_never_fails_a_step,
        the_projection_keeps_the_ten_keys_and_names_the_places_of_a_run,
    },
};

/// The runner has its own harness, so a test runs only when it is listed here;
/// `scenario::planned_tests` adds these to the runner's.
macro_rules! unit_tests {
    ($($name:ident),* $(,)?) => {
        pub(crate) const UNIT_TESTS: &[(&str, fn())] = &[$((stringify!($name), $name)),*];
    };
}

unit_tests! {
    a_seed_writes_the_same_sequence_on_every_machine,
    an_alphabet_is_refused_for_what_a_replay_could_not_run,
    a_sequence_is_scenario_text_and_takes_only_steps_that_act,
    a_fuzz_command_names_what_it_needs_and_refuses_the_rest,
    the_tree_s_alphabets_map_mask_and_corpus_load_as_a_run_loads_them,
    the_projection_keeps_the_ten_keys_and_names_the_places_of_a_run,
    a_field_is_named_by_its_path_and_an_entry_covers_what_is_below_it,
    rest_is_equal_samples_in_a_row_and_the_budget_never_fails_a_step,
    a_replay_ends_where_the_application_leaves_the_fixture,
    a_stream_keeps_its_observations_through_its_file,
    the_comparator_reports_the_first_field_outside_the_map_and_the_mask,
    a_step_not_at_rest_is_compared_by_its_outcome_alone,
    a_warning_or_a_lost_target_diverges_even_when_both_replays_show_it,
    the_shrinker_keeps_the_fewest_steps_that_still_diverge_and_confirms_them,
    a_divergence_that_does_not_repeat_is_not_confirmed,
    a_perturbation_waits_for_its_step_and_its_state,
    the_map_and_the_mask_take_one_sorted_entry_to_a_line,
}
