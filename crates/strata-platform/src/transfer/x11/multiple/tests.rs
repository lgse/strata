use x11rb::NONE;

use super::{answered_pairs, parse_pairs};

#[test]
fn an_atom_pair_list_reads_as_pairs() {
    assert_eq!(parse_pairs(&[1, 2, 3, 4]), Some(vec![(1, 2), (3, 4)]));
    assert_eq!(parse_pairs(&[]), Some(vec![]));
}

#[test]
fn an_odd_count_is_no_atom_pair_list() {
    assert_eq!(parse_pairs(&[1, 2, 3]), None);
}

#[test]
fn a_refused_pair_gets_none_for_its_property_and_the_rest_stay_as_asked() {
    let asked = [(10, 20), (11, 21), (12, 22)];
    assert_eq!(
        answered_pairs(&asked, &[false, true, false]),
        vec![10, 20, 11, NONE, 12, 22]
    );
}

#[test]
fn each_target_is_planned_once_in_the_order_first_asked() {
    use super::distinct;
    assert_eq!(
        distinct(&[(7, 1), (8, 2), (7, 3), (7, 4), (9, 5)]),
        vec![7, 8, 9]
    );
}
