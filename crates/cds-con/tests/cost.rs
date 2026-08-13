//! Attribute 2: the Baa yield's as-of level, and whether it reads 高.
//!
//! `finance-cli`'s `cost_cmd::level_at` is the authority. These pin the parts
//! of the rule that decide a label — the as-of window, strict "below", the
//! integer truncation, and the median cut — so a drift away from that
//! implementation fails here rather than in a delivered message.

use cds_con::cost::{label, latest_level, level_at, Level, HIGH_PCT};
use credit_store::Observation;

fn obs(pairs: &[(&str, f64)]) -> Vec<Observation> {
    pairs
        .iter()
        .map(|(d, v)| Observation {
            date: d.to_string(),
            value: *v,
        })
        .collect()
}

/// Five ascending months. The reading for each is its own rank so far.
fn ascending() -> Vec<Observation> {
    obs(&[
        ("1919-01-01", 1.0),
        ("1919-02-01", 2.0),
        ("1919-03-01", 3.0),
        ("1919-04-01", 4.0),
        ("1919-05-01", 5.0),
    ])
}

#[test]
fn the_window_is_as_of_and_a_later_month_never_leaks_into_an_earlier_reading() {
    // The whole point of an as-of basis: on 1919-02 only two observations
    // existed, so the reading must rest on two — not on all five, which is
    // knowledge that month did not have.
    let rows = ascending();
    let l = level_at(&rows, "1919-02-01").expect("month present");
    assert_eq!(l.n, 2);
    assert_eq!(l.pct, 50); // one of two below
    assert_eq!(l.value, 2.0);

    // The last month sees everything, and nothing more.
    let last = level_at(&rows, "1919-05-01").expect("month present");
    assert_eq!(last.n, 5);
    assert_eq!(last.pct, 80); // four of five below
}

#[test]
fn the_first_observation_rests_on_itself_alone() {
    // n=1, nothing below it: a real reading on the thinnest possible base.
    // Reported rather than suppressed — the `n` is what tells the reader how
    // much the percentile is worth.
    let l = level_at(&ascending(), "1919-01-01").expect("month present");
    assert_eq!((l.pct, l.n), (0, 1));
}

#[test]
fn below_is_strict_so_ties_do_not_count_against_themselves() {
    // Three identical values: none is below any other, so every reading is 0.
    // A non-strict comparison would report the last one at 66, inventing a
    // rise out of a flat series.
    let rows = obs(&[("2020-01-01", 4.0), ("2020-02-01", 4.0), ("2020-03-01", 4.0)]);
    assert_eq!(level_at(&rows, "2020-03-01").expect("present").pct, 0);
}

#[test]
fn the_percentile_truncates_rather_than_rounds() {
    // Two of three below is 66.67%. `cost_cmd::level_at` divides in integers,
    // so it prints 66. Rounding would print 67 and the two implementations
    // would disagree on a number the owner reads off both.
    let rows = obs(&[("2020-01-01", 1.0), ("2020-02-01", 2.0), ("2020-03-01", 3.0)]);
    assert_eq!(level_at(&rows, "2020-03-01").expect("present").pct, 66);
}

#[test]
fn a_date_the_series_does_not_carry_is_absent_not_zero() {
    // 無資料 and "0th percentile" are opposite findings; collapsing them would
    // report the cheapest possible borrowing for a month with no data at all.
    assert_eq!(level_at(&ascending(), "1901-06-01"), None);
    assert_eq!(level_at(&[], "1919-01-01"), None);
}

#[test]
fn the_cut_is_the_median_and_the_boundary_reads_high() {
    assert_eq!(HIGH_PCT, 50);
    assert_eq!(label(49), "不高");
    assert_eq!(label(50), "高");
    assert_eq!(label(51), "高");
    assert_eq!(label(0), "不高");
    assert_eq!(label(100), "高");
}

#[test]
fn the_latest_reading_is_the_newest_row_read_as_of_itself() {
    let rows = ascending();
    assert_eq!(latest_level(&rows), level_at(&rows, "1919-05-01"));
    assert_eq!(
        latest_level(&rows),
        Some(Level {
            date: "1919-05-01".into(),
            value: 5.0,
            pct: 80,
            n: 5
        })
    );
    assert_eq!(latest_level(&[]), None);
}

#[test]
fn a_fall_from_the_top_is_visible_in_the_reading() {
    // The direction measure this replaced could not separate these: the last
    // month is a sharp drop, and what attribute 2 must report is that
    // borrowing is now cheap, regardless of which way the series moved.
    let rows = obs(&[
        ("2020-01-01", 8.0),
        ("2020-02-01", 9.0),
        ("2020-03-01", 10.0),
        ("2020-04-01", 1.0),
    ]);
    let l = latest_level(&rows).expect("present");
    assert_eq!((l.pct, l.n), (0, 4));
    assert_eq!(label(l.pct), "不高");
}
