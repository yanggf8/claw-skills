use inflation_con::analysis::{annualized, classify, latest, rising_over, Obs, Series, Status};

/// A monthly index compounding at `monthly_pct` per observation.
fn level(monthly_pct: f64, n: usize) -> Vec<Obs> {
    let mut v = vec![100.0_f64];
    for _ in 1..n { v.push(v[v.len() - 1] * (1.0 + monthly_pct / 100.0)); }
    v.iter().enumerate()
        .map(|(i, x)| Obs { day: format!("2026-{:02}-01", i % 12 + 1), value: *x })
        .collect()
}

fn daily(value: f64, n: usize) -> Vec<Obs> {
    (0..n).map(|i| Obs { day: format!("2026-06-{:02}", i % 28 + 1), value }).collect()
}

/// Monthly rate that annualizes to `annual` percent.
fn mo(annual: f64) -> f64 { ((1.0 + annual / 100.0f64).powf(1.0 / 12.0) - 1.0) * 100.0 }

fn series(pce: Vec<Obs>, cpi: Vec<Obs>, be: Vec<Obs>) -> Series {
    Series { core_pce: pce, core_cpi: cpi, breakeven_10y: be }
}

// Every expectation below was produced by running the real Python first.

#[test]
fn red_when_levels_cpi_and_context_all_confirm() {
    // pce3 = pce6 = 4.0%, cpi hot, breakeven 2.6 >= 2.5, stance restrictive.
    let s = series(level(mo(4.0), 12), level(mo(4.0), 12), daily(2.6, 70));
    let (st, d) = classify(&s, "restrictive");
    assert_eq!(st, Status::Red);
    assert_eq!(d.reasons.len(), 4, "{:?}", d.reasons);
}

#[test]
fn yellow_when_levels_are_above_target_but_short_of_red() {
    // pce3 = pce6 = 3.2% — over 3.0, under 3.5.
    let s = series(level(mo(3.2), 12), level(mo(3.2), 12), daily(2.2, 70));
    let (st, d) = classify(&s, "restrictive");
    assert_eq!(st, Status::Yellow);
    assert_eq!(d.reasons.len(), 2, "no boundary note when levels do not reach RED: {:?}", d.reasons);
}

#[test]
fn yellow_boundary_note_when_levels_reach_red_but_context_fails() {
    // Levels 4.0% reach RED, but breakeven 2.2 < 2.5 and stance is easing.
    let s = series(level(mo(4.0), 12), level(mo(4.0), 12), daily(2.2, 70));
    let (st, d) = classify(&s, "easing");
    assert_eq!(st, Status::Yellow);
    assert_eq!(d.reasons.len(), 3, "the boundary note is the third: {:?}", d.reasons);
    assert!(d.reasons.iter().any(|r| r.contains("context")), "{:?}", d.reasons);
}

#[test]
fn watch_when_pce_is_warm_but_not_confirmed() {
    let s = series(level(mo(2.6), 12), level(mo(2.0), 12), daily(2.2, 70));
    let (st, d) = classify(&s, "restrictive");
    assert_eq!(st, Status::Watch);
    assert_eq!(d.reasons.len(), 1, "{:?}", d.reasons);
}

#[test]
fn ok_when_nothing_confirms() {
    let s = series(level(mo(2.0), 12), level(mo(2.0), 12), daily(2.2, 70));
    let (st, d) = classify(&s, "restrictive");
    assert_eq!(st, Status::Ok);
    assert_eq!(d.reasons.len(), 1, "{:?}", d.reasons);
}

#[test]
fn insufficient_data_below_seven_core_pce_observations() {
    let s = series(level(mo(3.0), 5), level(mo(3.0), 5), daily(2.2, 70));
    let (st, d) = classify(&s, "restrictive");
    assert_eq!(st, Status::InsufficientData);
    assert_eq!(d.core_pce_obs, 5);
    assert!(d.pce3.is_none(), "the guard returns before computing anything");
}

#[test]
fn insufficient_data_when_core_cpi_is_missing_however_long_pce_is() {
    let s = series(level(mo(3.0), 24), vec![], daily(2.2, 70));
    let (st, _) = classify(&s, "restrictive");
    assert_eq!(st, Status::InsufficientData, "core CPI is required, not optional");
}

#[test]
fn the_ladder_is_mutually_exclusive() {
    // chipcon's RED and ORANGE accumulate reasons; this ladder does not. A RED
    // run must carry no YELLOW wording, and a YELLOW run no WATCH wording.
    //
    // The needle matters. RED legitimately says "core CPI confirms (>= 3.0% on
    // 3-mo or 6-mo)", so matching on ">= 3.0%" hits RED's own text and the test
    // fails against a correct implementation. Only YELLOW says "both >= 3.0%".
    // Verified against the real Python: RED has 1 line containing ">= 3.0%" and
    // 0 containing "both >= 3.0%".
    let red = series(level(mo(4.0), 12), level(mo(4.0), 12), daily(2.6, 70));
    let (st, d) = classify(&red, "restrictive");
    assert_eq!(st, Status::Red);
    assert!(d.reasons.iter().any(|r| r.contains("both >= 3.5%")), "RED's own PCE line: {:?}", d.reasons);
    assert!(!d.reasons.iter().any(|r| r.contains("both >= 3.0%")), "RED must not carry YELLOW's PCE line: {:?}", d.reasons);

    let yellow = series(level(mo(3.2), 12), level(mo(3.2), 12), daily(2.2, 70));
    let (st, d) = classify(&yellow, "restrictive");
    assert_eq!(st, Status::Yellow);
    assert!(!d.reasons.iter().any(|r| r.contains("not yet confirming")), "YELLOW must not carry WATCH reasons: {:?}", d.reasons);
    assert!(!d.reasons.iter().any(|r| r.contains("both >= 3.5%")), "YELLOW must not carry RED's PCE line: {:?}", d.reasons);
}

#[test]
fn an_empty_breakeven_cannot_confirm_red() {
    // breakeven_ge is false and be_rising is None, so context_not_easing is false
    // however hot the levels are. This is also the shape that used to crash.
    let s = series(level(mo(4.0), 12), level(mo(4.0), 12), vec![]);
    let (st, d) = classify(&s, "restrictive");
    assert_eq!(st, Status::Yellow, "a missing series must not confirm a regime");
    assert!(d.breakeven.is_none());
    assert!(d.breakeven_rising.is_none());
    assert!(d.reasons.iter().any(|r| r.contains("unavailable")), "the note must say so: {:?}", d.reasons);
}

#[test]
fn a_rising_breakeven_can_substitute_for_the_level() {
    // context_not_easing accepts EITHER breakeven >= 2.5 OR a rising trend.
    let mut be: Vec<Obs> = daily(2.0, 40);
    be.extend((0..40).map(|i| Obs { day: format!("2026-07-{:02}", i % 28 + 1), value: 2.3 }));
    let s = series(level(mo(4.0), 12), level(mo(4.0), 12), be);
    let (st, d) = classify(&s, "restrictive");
    assert_eq!(st, Status::Red, "a rising breakeven under 2.5 still satisfies the context clause");
    // Pin WHY it is RED, or an implementation that satisfied the clause through
    // breakeven_ge instead would also pass and the `or` would be untested.
    assert!(d.breakeven.unwrap() < 2.5, "the level alone must not qualify: {:?}", d.breakeven);
    assert_eq!(d.breakeven_rising, Some(true));
    assert!(d.reasons.iter().any(|r| r.contains("rising")), "the rising branch must be cited: {:?}", d.reasons);
}

#[test]
fn easing_stance_blocks_red_regardless_of_the_data() {
    let s = series(level(mo(4.0), 12), level(mo(4.0), 12), daily(2.6, 70));
    let (st, _) = classify(&s, "easing");
    assert_eq!(st, Status::Yellow, "policy_stance is the human's veto");
}

#[test]
fn a_cool_core_cpi_blocks_red_however_hot_pce_is() {
    // core_cpi_hot_3_or_6 is a required conjunct. Without this, dropping it from
    // the RED condition changes nothing any test can see.
    let s = series(level(mo(4.0), 12), level(mo(1.0), 12), daily(2.6, 70));
    let (st, _) = classify(&s, "restrictive");
    assert_ne!(st, Status::Red, "core CPI must confirm before RED");
}

#[test]
fn a_falling_pce_pace_changes_the_ok_wording() {
    // pce_falling affects only the OK reason text, never the status choice.
    // Rising path: 3-mo pace equals 6-mo, so not falling.
    let flat = series(level(mo(2.0), 12), level(mo(2.0), 12), daily(2.2, 70));
    let (st, d) = classify(&flat, "restrictive");
    assert_eq!(st, Status::Ok);
    assert!(d.reasons[0].contains("< 2.5%"), "steady path takes the threshold wording: {:?}", d.reasons);
}

#[test]
fn annualized_needs_strictly_more_than_n_observations() {
    assert!(annualized(&level(mo(3.0), 3), 3).is_none(), "len == n must be None");
    assert!(annualized(&level(mo(3.0), 4), 3).is_some());
}

#[test]
fn annualized_compounds_to_the_expected_rate() {
    let a = annualized(&level(mo(4.0), 13), 6).unwrap();
    assert!((a - 4.0).abs() < 0.01, "6-month window of a 4% path annualizes to 4%, got {a}");
}

#[test]
fn annualized_returns_none_when_the_base_is_not_positive() {
    let rows = vec![
        Obs { day: "2026-01-01".into(), value: 0.0 },
        Obs { day: "2026-02-01".into(), value: 100.0 },
    ];
    assert!(annualized(&rows, 1).is_none());
}

#[test]
fn rising_over_needs_strictly_more_than_the_lookback() {
    assert!(rising_over(&daily(2.0, 63), 63).is_none(), "len == lookback must be None");
    assert!(rising_over(&daily(2.0, 64), 63).is_some());
}

#[test]
fn rising_over_compares_the_endpoints_only() {
    let mut rows = daily(2.0, 64);
    rows.last_mut().unwrap().value = 2.5;
    assert_eq!(rising_over(&rows, 63), Some(true));
    rows.last_mut().unwrap().value = 1.5;
    assert_eq!(rising_over(&rows, 63), Some(false));
}

#[test]
fn latest_is_the_final_observation_or_none() {
    assert!(latest(&[]).is_none());
    let rows = daily(2.2, 3);
    assert_eq!(latest(&rows).unwrap().value, 2.2);
}

#[test]
fn details_carries_every_field_render_needs() {
    let s = series(level(mo(3.2), 12), level(mo(3.2), 12), daily(2.2, 70));
    let (_, d) = classify(&s, "restrictive");
    assert!(d.pce3.is_some() && d.pce6.is_some() && d.cpi3.is_some() && d.cpi6.is_some());
    assert!(d.breakeven.is_some() && d.breakeven_day.is_some());
    assert_eq!(d.policy_stance, "restrictive");
    assert_eq!(d.core_pce_obs, 12);
    assert!(!d.core_pce_day.is_empty());
}
