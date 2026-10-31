// SPDX-License-Identifier: MIT

use crate::{
    scenario_tests::{
        a_cost_gives_each_run_label_a_positive_number,
        a_known_bug_must_name_a_step_line_that_can_fail,
        a_quality_rates_four_axes_and_its_score_follows_the_rubric,
        a_scenario_lives_in_a_feature_folder_and_names_its_runs,
        a_skipped_scenario_names_a_reason_and_no_mutation_or_known_bug,
        every_scenario_file_is_planned_and_well_formed, no_test_attribute_hides_a_runner_test,
        the_harness_starts_costly_runs_first_and_names_select_whole_scenarios,
    },
    step_tests::{
        a_flag_keeps_whether_a_predicate_held_and_always_runs,
        a_numbered_tree_expands_like_the_python_comprehension,
        a_run_keeps_its_own_mode_steps_and_rejects_a_malformed_one,
        a_slice_keeps_the_middle_band_or_one_name_of_a_noted_list,
        a_variable_replaces_only_its_whole_token,
        an_empty_check_counts_a_missing_directory_as_empty,
        baseline_comparison_thresholds_pixels_and_budget,
        baseline_mismatches_fail_and_only_an_update_writes_the_baseline,
        file_parts_join_repeats_and_fs_actions_write_archives_and_sparse_files,
        fixture_roots_and_baseline_steps_check_their_shape_before_the_run,
        fs_actions_make_numbered_directories_and_pin_a_populated_tree,
        fs_actions_write_archives_thumbnails_and_records,
        glob_and_zipcrypto_helpers_match_names_and_write_archives,
        log_proc_and_each_steps_check_their_shape_before_the_run,
        notes_stand_in_for_what_a_run_reads_and_unknown_variables_are_rejected,
        only_a_controller_panic_is_filed_as_a_harness_error,
        peer_restart_and_guard_steps_check_their_shape_before_the_run,
        prefs_write_nested_tables_into_settings_toml,
        remove_and_rename_act_only_inside_the_run_directories,
        waiting_for_exit_wakes_on_the_exit_and_gives_up_at_the_timeout,
    },
};

/// This target has its own harness, so `#[test]` and `#[cfg(test)]` code never runs;
/// a test runs only when listed here.
macro_rules! unit_tests {
    ($($name:ident),* $(,)?) => {
        pub(crate) const UNIT_TESTS: &[(&str, fn())] = &[$((stringify!($name), $name)),*];
    };
}

unit_tests! {
    every_scenario_file_is_planned_and_well_formed,
    a_numbered_tree_expands_like_the_python_comprehension,
    a_run_keeps_its_own_mode_steps_and_rejects_a_malformed_one,
    notes_stand_in_for_what_a_run_reads_and_unknown_variables_are_rejected,
    fs_actions_make_numbered_directories_and_pin_a_populated_tree,
    a_variable_replaces_only_its_whole_token,
    a_known_bug_must_name_a_step_line_that_can_fail,
    a_slice_keeps_the_middle_band_or_one_name_of_a_noted_list,
    an_empty_check_counts_a_missing_directory_as_empty,
    only_a_controller_panic_is_filed_as_a_harness_error,
    remove_and_rename_act_only_inside_the_run_directories,
    file_parts_join_repeats_and_fs_actions_write_archives_and_sparse_files,
    a_flag_keeps_whether_a_predicate_held_and_always_runs,
    fs_actions_write_archives_thumbnails_and_records,
    log_proc_and_each_steps_check_their_shape_before_the_run,
    peer_restart_and_guard_steps_check_their_shape_before_the_run,
    glob_and_zipcrypto_helpers_match_names_and_write_archives,
    baseline_comparison_thresholds_pixels_and_budget,
    baseline_mismatches_fail_and_only_an_update_writes_the_baseline,
    fixture_roots_and_baseline_steps_check_their_shape_before_the_run,
    a_skipped_scenario_names_a_reason_and_no_mutation_or_known_bug,
    a_cost_gives_each_run_label_a_positive_number,
    a_scenario_lives_in_a_feature_folder_and_names_its_runs,
    a_quality_rates_four_axes_and_its_score_follows_the_rubric,
    the_harness_starts_costly_runs_first_and_names_select_whole_scenarios,
    waiting_for_exit_wakes_on_the_exit_and_gives_up_at_the_timeout,
    prefs_write_nested_tables_into_settings_toml,
    no_test_attribute_hides_a_runner_test,
}
