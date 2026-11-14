// SPDX-License-Identifier: MIT

//! The fuzz driver's own tests. None starts the application: the generator, the projection,
//! the rest rule, the comparator and the shrinker are held to what they promise on values
//! made in the tests, and the tree's alphabets, map, mask and corpus are loaded as a run
//! loads them.

use super::{
    command_tests::{
        a_command_fails_on_what_did_not_agree_and_on_nothing_replayed,
        a_fuzz_command_names_what_it_needs_and_refuses_the_rest,
        a_parked_line_names_a_version_and_a_seed_and_is_sorted_by_both,
        a_perturbed_run_and_a_survey_of_parked_sequences_say_what_they_are,
        a_run_skips_a_seed_only_where_it_would_write_what_was_parked,
        the_summary_says_how_long_the_target_went_on_changing,
        the_tree_s_alphabets_map_mask_and_corpus_load_as_a_run_loads_them,
    },
    compare_tests::{
        a_divergence_that_does_not_repeat_is_not_confirmed,
        a_perturbation_waits_for_its_step_and_its_state,
        a_step_not_at_rest_is_compared_by_its_outcome_alone,
        a_warning_a_broken_invariant_or_a_lost_target_diverges_even_when_both_replays_show_it,
        an_entry_is_refused_for_its_form_its_code_or_blinding_a_whole_root,
        the_comparator_reports_the_first_field_outside_the_map_and_the_mask,
        the_map_and_the_mask_take_one_sorted_entry_to_a_line,
        the_shrinker_keeps_the_fewest_steps_that_still_diverge_and_confirms_them,
    },
    generate_tests::{
        a_kept_file_of_the_alphabet_s_version_is_made_of_steps_its_seed_writes,
        a_seed_writes_the_same_sequence_on_every_machine,
        a_sequence_is_scenario_text_and_takes_only_steps_that_act,
        a_sequence_s_name_says_the_alphabet_version_and_the_seed_it_came_from,
        an_alphabet_is_refused_for_what_a_replay_could_not_run,
        an_alphabet_s_version_is_pinned_to_what_its_seeds_write,
    },
    observe_tests::{
        a_field_is_named_by_its_path_and_an_entry_covers_what_is_below_it,
        a_replay_ends_where_the_application_leaves_the_fixture,
        a_step_s_outcome_is_what_the_target_answered_or_how_it_went_away,
        a_stream_keeps_its_observations_through_its_file,
        an_entry_covers_its_field_and_what_is_below_it_at_a_boundary,
        rest_is_equal_samples_in_a_row_and_the_budget_never_fails_a_step,
        the_projection_keeps_the_ten_keys_and_names_the_places_of_a_run,
        the_projection_names_every_key_the_snapshot_has,
        two_streams_are_compared_only_when_they_answer_one_another,
    },
    targets_tests::{
        a_command_takes_one_target_or_gtk_and_rune_and_the_comparing_commands_take_both,
        a_run_on_two_targets_names_the_replays_that_differ_and_keeps_its_own_folder,
        each_target_is_held_to_itself_and_rune_to_the_first_gtk_replay_under_the_map,
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
    a_sequence_s_name_says_the_alphabet_version_and_the_seed_it_came_from,
    an_alphabet_is_refused_for_what_a_replay_could_not_run,
    an_alphabet_s_version_is_pinned_to_what_its_seeds_write,
    a_sequence_is_scenario_text_and_takes_only_steps_that_act,
    a_kept_file_of_the_alphabet_s_version_is_made_of_steps_its_seed_writes,
    the_projection_keeps_the_ten_keys_and_names_the_places_of_a_run,
    the_projection_names_every_key_the_snapshot_has,
    a_field_is_named_by_its_path_and_an_entry_covers_what_is_below_it,
    an_entry_covers_its_field_and_what_is_below_it_at_a_boundary,
    rest_is_equal_samples_in_a_row_and_the_budget_never_fails_a_step,
    a_replay_ends_where_the_application_leaves_the_fixture,
    a_step_s_outcome_is_what_the_target_answered_or_how_it_went_away,
    a_stream_keeps_its_observations_through_its_file,
    two_streams_are_compared_only_when_they_answer_one_another,
    the_comparator_reports_the_first_field_outside_the_map_and_the_mask,
    a_step_not_at_rest_is_compared_by_its_outcome_alone,
    a_warning_a_broken_invariant_or_a_lost_target_diverges_even_when_both_replays_show_it,
    the_shrinker_keeps_the_fewest_steps_that_still_diverge_and_confirms_them,
    a_divergence_that_does_not_repeat_is_not_confirmed,
    a_perturbation_waits_for_its_step_and_its_state,
    the_map_and_the_mask_take_one_sorted_entry_to_a_line,
    an_entry_is_refused_for_its_form_its_code_or_blinding_a_whole_root,
    a_parked_line_names_a_version_and_a_seed_and_is_sorted_by_both,
    a_run_skips_a_seed_only_where_it_would_write_what_was_parked,
    a_fuzz_command_names_what_it_needs_and_refuses_the_rest,
    a_command_fails_on_what_did_not_agree_and_on_nothing_replayed,
    a_perturbed_run_and_a_survey_of_parked_sequences_say_what_they_are,
    the_summary_says_how_long_the_target_went_on_changing,
    the_tree_s_alphabets_map_mask_and_corpus_load_as_a_run_loads_them,
    a_command_takes_one_target_or_gtk_and_rune_and_the_comparing_commands_take_both,
    each_target_is_held_to_itself_and_rune_to_the_first_gtk_replay_under_the_map,
    a_run_on_two_targets_names_the_replays_that_differ_and_keeps_its_own_folder,
}
