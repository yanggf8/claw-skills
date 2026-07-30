use oilcon::analysis::{
    classify_oil_trend, compute_change_pct, compute_extremes, ma_rising, moving_average,
    pct_below_60d_high, Row,
};

fn series(n: usize, base: f64, step: f64) -> Vec<Row> {
    (0..n).map(|i| Row { day: format!("2026-{:02}-{:02}", i / 28 + 1, i % 28 + 1), close: base + step * i as f64 }).collect()
}

fn flat(n: usize, close: f64) -> Vec<Row> {
    (0..n).map(|i| Row { day: format!("2026-{:02}-{:02}", i / 28 + 1, i % 28 + 1), close }).collect()
}

#[test]
fn moving_average_of_a_flat_series_is_that_value() {
    assert!((moving_average(&flat(50, 70.0), 50).unwrap() - 70.0).abs() < 1e-9);
}

#[test]
fn moving_average_refuses_fewer_rows_than_the_window() {
    assert!(moving_average(&flat(49, 70.0), 50).is_err());
    assert!(moving_average(&flat(50, 70.0), 50).is_ok());
}

#[test]
fn ma_rising_needs_n_plus_lookback_rows() {
    // Python's ma_rising returns False rather than raising when short.
    assert!(!ma_rising(&series(69, 60.0, 0.1), 50, 20), "69 rows is one short of 50+20");
    assert!(ma_rising(&series(70, 60.0, 0.1), 50, 20), "70 rows on a rising series must be true");
}

#[test]
fn ma_rising_compares_against_the_window_ending_lookback_rows_earlier() {
    let up = series(90, 60.0, 0.2);
    assert!(ma_rising(&up, 50, 20));
    let down: Vec<Row> = up.iter().rev().enumerate()
        .map(|(i, r)| Row { day: format!("d{i:03}"), close: r.close }).collect();
    assert!(!ma_rising(&down, 50, 20));
}

#[test]
fn ma_rising_uses_a_lookback_of_twenty_not_some_other_span() {
    // The test above pins direction, not the lookback: on a monotone series
    // every plausible lookback agrees, so it cannot see a wrong one. A series
    // that rises then rolls over separates them — 70 rows up at +0.5 to a peak
    // of 94.5, then 20 rows down at -1.2. Verified against the Python: lookback
    // 20 gives true, lookback 10 gives false.
    let mut rows = series(70, 60.0, 0.5);
    let peak = rows.last().unwrap().close;
    for j in 0..20 {
        rows.push(Row { day: format!("e{j:03}"), close: peak - 1.2 * (j + 1) as f64 });
    }
    assert!(ma_rising(&rows, 50, 20), "the 50MA is still above where it was 20 rows back");
    assert!(!ma_rising(&rows, 50, 10), "but not above where it was 10 rows back");
    // The two asserts above call ma_rising directly, so they say nothing about the
    // lookback classify_oil_trend passes. On this fixture the call site is visible:
    // cur 70.5 is under ma50 85.11, so lookback 20 gives rollover and lookback 10
    // gives no-uptrend. Without this line, `ma_rising(rows, 50, 20)` -> `10` inside
    // classify_oil_trend is caught by no test at all.
    assert_eq!(classify_oil_trend(&rows), "rollover");
}

#[test]
fn compute_extremes_reports_the_first_of_a_tied_high_not_the_last() {
    // Python takes `max(enumerate(rows), key=...)`, which is the FIRST maximum.
    // Rust's `max_by` is the LAST, so the obvious translation is wrong on ties —
    // and ties are ordinary in daily closes. `high_day` and `days_since_high` are
    // both rendered, so this is visible output, not an internal detail.
    let mut rows = flat(252, 70.0);
    rows[10].close = 120.0;
    rows[200].close = 120.0; // the tie
    rows[240].close = 40.0;
    let e = compute_extremes(&rows);
    assert_eq!(e.days_since_high, 241, "the earlier of the two highs wins");
    assert_eq!(e.high_day, rows[10].day, "and it is row 10's day that is reported");
}

#[test]
fn pct_below_60d_high_is_zero_at_the_high_and_positive_beneath_it() {
    let mut rows = flat(60, 100.0);
    assert!(pct_below_60d_high(&rows).abs() < 1e-9, "flat series sits at its own high");
    rows.last_mut().unwrap().close = 90.0;
    let p = pct_below_60d_high(&rows);
    assert!((p - 10.0).abs() < 1e-9, "10% under the 60-day high, got {p}");
}

#[test]
fn classify_needs_seventy_rows_exactly() {
    // Pinned by the Python's own boundary test, which asserts the 70 side lands
    // on `uptrend` — not merely "something other than insufficient". Verified:
    // 70 rows at +0.2 give cur 73.8 > ma50 68.9, MA rising, 0.00% off the high.
    assert_eq!(classify_oil_trend(&series(69, 60.0, 0.2)), "insufficient-history");
    assert_eq!(classify_oil_trend(&series(70, 60.0, 0.2)), "uptrend");
}

#[test]
fn price_exactly_equal_to_the_50ma_is_not_an_uptrend() {
    // `current > ma50` is strict. This was a real bug, fixed in f14afa4.
    //
    // The fixture is a solved fixed point, and it has to be. Setting the last
    // close to a previously computed MA does NOT produce equality, because the
    // last close is itself one of the 50 rows the MA averages. Python's own
    // test makes exactly that mistake: it lands on cur 72.250 vs ma50 72.005 —
    // strictly ABOVE — and passes only because pct_below is 13.99%, i.e. for
    // the wrong reason, with a comment that misdescribes it. Copying it here
    // would leave `>` -> `>=` green.
    //
    // Solve instead for x with mean(last 49 rows, x) == x, i.e. x = sum/49.
    // For 80 rows of 60.0 + 0.2i that is exactly 70.8 (mean of indices 30..78).
    let mut rows = series(80, 60.0, 0.2);
    rows.last_mut().unwrap().close = 70.8;
    let ma50 = moving_average(&rows, 50).unwrap();
    assert_eq!(rows.last().unwrap().close, ma50, "fixture must sit exactly on the MA");
    assert!(ma_rising(&rows, 50, 20), "and the MA must be rising, or the else branch is untested");
    // Strict `>` fails, MA rising -> rollover. Under `>=` this becomes `uptrend`
    // (pct_below is 6.35%), which is what makes the mutation observable.
    assert_eq!(classify_oil_trend(&rows), "rollover");
}

#[test]
fn pct_below_exactly_ten_percent_is_still_an_uptrend() {
    // `pct_below <= 10.0` is inclusive. No other fixture sits on the boundary:
    // the uptrend one is at 0.00% and the weakening one at 11.00%, so `<= 10.0`
    // -> `< 10.0` is invisible to both.
    //
    // 90 rows of 6.0 + 0.5i put the 60-day high at index 88 = 50.0; the last
    // close is forced to 45.0. (50 - 45) / 50 * 100 is exactly 10.0 in f64 —
    // most 10%-looking pairs are not (104 * 0.9 gives 9.999999999999993), so
    // this pair is chosen, not incidental.
    let mut rows = series(90, 6.0, 0.5);
    rows.last_mut().unwrap().close = 45.0;
    assert_eq!(pct_below_60d_high(&rows), 10.0, "fixture must sit exactly on the boundary");
    assert!(rows.last().unwrap().close > moving_average(&rows, 50).unwrap());
    assert!(ma_rising(&rows, 50, 20));
    assert_eq!(classify_oil_trend(&rows), "uptrend");
}

#[test]
fn a_steady_rise_within_ten_percent_of_its_high_is_an_uptrend() {
    assert_eq!(classify_oil_trend(&series(90, 60.0, 0.2)), "uptrend");
}

#[test]
fn above_a_rising_ma_but_far_off_the_high_is_a_weakening_uptrend() {
    // This fixture is fiddly and the obvious construction does not work. Pulling a
    // single close down 15% drops the price BELOW the 50MA, which is `rollover`,
    // not `weakening-uptrend` — verified against the real Python, where that
    // version gave cur 81.26 against ma50 85.51. Reaching this state needs
    // `cur > ma50` AND `pct_below_60d_high > 10` at once, so the rise has to be
    // steep enough that the average lags well behind a shallow pullback.
    //
    // Solved numerically: 90 rows rising 0.6/day, then the last 4 tapering
    // linearly to 11% below the peak. Gives cur 98.790, ma50 97.970, MA rising,
    // 11.00% below the 60-day high.
    let mut rows = series(90, 60.0, 0.6);
    let pull = 4usize;
    let peak = rows[rows.len() - 1 - pull].close;
    for j in (rows.len() - pull)..rows.len() {
        let frac = (j - (rows.len() - 1 - pull)) as f64 / pull as f64;
        rows[j].close = peak * (1.0 - 0.11 * frac);
    }
    let ma50 = moving_average(&rows, 50).unwrap();
    assert!(rows.last().unwrap().close > ma50, "fixture must stay above the 50MA");
    assert!(ma_rising(&rows, 50, 20), "fixture must keep the MA rising");
    assert!(pct_below_60d_high(&rows) > 10.0, "and must sit more than 10% off the high");
    assert_eq!(classify_oil_trend(&rows), "weakening-uptrend");
}

#[test]
fn below_the_ma_with_it_still_rising_is_a_rollover() {
    let mut rows = series(90, 60.0, 0.4);
    rows.last_mut().unwrap().close = 40.0;    // far under the average
    let ma50 = moving_average(&rows, 50).unwrap();
    assert!(rows.last().unwrap().close < ma50);
    assert!(ma_rising(&rows, 50, 20));
    assert_eq!(classify_oil_trend(&rows), "rollover");
}

#[test]
fn below_a_falling_ma_is_no_uptrend() {
    let falling: Vec<Row> = (0..90)
        .map(|i| Row { day: format!("d{i:03}"), close: 100.0 - 0.4 * i as f64 })
        .collect();
    let ma50 = moving_average(&falling, 50).unwrap();
    assert!(falling.last().unwrap().close < ma50);
    assert!(!ma_rising(&falling, 50, 20));
    assert_eq!(classify_oil_trend(&falling), "no-uptrend");
}

#[test]
fn compute_extremes_scans_the_whole_slice_not_a_suffix() {
    // The reason Task 3's backfill guard cannot be a row count: the high and low
    // are taken over every row given, so a short window silently reports a short
    // extreme while the message still says one year.
    let mut rows = flat(252, 70.0);
    rows[10].close = 120.0;    // the high is early
    rows[240].close = 40.0;    // the low is late
    let e = compute_extremes(&rows);
    assert!((e.high_close - 120.0).abs() < 1e-9, "high must come from row 10");
    assert!((e.low_close - 40.0).abs() < 1e-9, "low must come from row 240");
    assert_eq!(e.days_since_high, 241);
    assert_eq!(e.days_since_low, 11);
}

#[test]
fn compute_change_pct_is_the_last_two_closes() {
    let rows = vec![
        Row { day: "d0".into(), close: 100.0 },
        Row { day: "d1".into(), close: 110.0 },
    ];
    assert!((compute_change_pct(&rows) - 10.0).abs() < 1e-9);
}
