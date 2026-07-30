use chipcon::analysis::{classify, consecutive_down, ma, ma_rising, pct, return_n, Row, Status};

/// Ascending series of `n` rows, close = base + i*step. Dates are only labels here.
fn series(n: usize, base: f64, step: f64) -> Vec<Row> {
    (0..n).map(|i| Row { day: format!("d{i:03}"), close: base + step * i as f64 }).collect()
}

fn flat(n: usize, close: f64) -> Vec<Row> {
    (0..n).map(|i| Row { day: format!("d{i:03}"), close }).collect()
}

#[test]
fn insufficient_history_below_twenty_rows() {
    let (s, d) = classify(&series(19, 100.0, 1.0), &[], &[]);
    assert_eq!(s, Status::InsufficientHistory);
    assert_eq!(d.rows, 19);
}

#[test]
fn twenty_rows_is_enough_to_classify() {
    let (s, _) = classify(&series(20, 100.0, 1.0), &[], &[]);
    assert_ne!(s, Status::InsufficientHistory);
}

#[test]
fn ma_needs_n_rows() {
    assert!(ma(&series(4, 10.0, 0.0), 5).is_none());
    assert_eq!(ma(&flat(5, 10.0), 5), Some(10.0));
}

#[test]
fn ma_rising_needs_n_plus_lookback_rows() {
    // The boundary the Python encodes as `len(rows) < n + lookback`.
    assert!(ma_rising(&series(24, 100.0, 1.0), 20, 5).is_none(), "24 rows is one short of 20+5");
    assert!(ma_rising(&series(25, 100.0, 1.0), 20, 5).is_some(), "25 rows is exactly enough");
}

#[test]
fn ma_rising_compares_against_a_window_ending_lookback_rows_earlier() {
    // Not a shifted average of the same span: past = ma(rows[:-lookback], n).
    let rising = series(30, 100.0, 1.0);
    assert_eq!(ma_rising(&rising, 20, 5), Some(true));
    let falling: Vec<Row> = rising.iter().rev()
        .enumerate().map(|(i, r)| Row { day: format!("d{i:03}"), close: r.close }).collect();
    assert_eq!(ma_rising(&falling, 20, 5), Some(false));
}

#[test]
fn return_n_needs_strictly_more_than_n_rows() {
    assert!(return_n(&series(5, 100.0, 1.0), 5).is_none(), "len == n must be None");
    assert!(return_n(&series(6, 100.0, 1.0), 5).is_some());
}

#[test]
fn return_n_is_percent_from_n_rows_ago() {
    let rows = vec![
        Row { day: "d0".into(), close: 100.0 },
        Row { day: "d1".into(), close: 110.0 },
    ];
    assert!((return_n(&rows, 1).unwrap() - 10.0).abs() < 1e-9);
}

#[test]
fn consecutive_down_counts_only_the_trailing_run() {
    let rows = vec![
        Row { day: "d0".into(), close: 100.0 },
        Row { day: "d1".into(), close:  90.0 },  // down
        Row { day: "d2".into(), close:  95.0 },  // up — breaks the run
        Row { day: "d3".into(), close:  94.0 },  // down
        Row { day: "d4".into(), close:  93.0 },  // down
    ];
    assert_eq!(consecutive_down(&rows), 2);
    assert_eq!(consecutive_down(&flat(5, 10.0)), 0, "equal closes are not down days");
}

#[test]
fn pct_is_relative_change_in_percent() {
    assert!((pct(110.0, 100.0) - 10.0).abs() < 1e-9);
    assert!((pct(90.0, 100.0) + 10.0).abs() < 1e-9);
}

#[test]
fn red_when_below_50dma() {
    let mut rows = series(60, 100.0, 1.0);
    rows.last_mut().unwrap().close = 50.0;   // far under both averages
    let (s, d) = classify(&rows, &[], &[]);
    assert_eq!(s, Status::Red);
    assert!(d.reasons.iter().any(|r| r.contains("50DMA")), "{:?}", d.reasons);
}

#[test]
fn red_accumulates_every_reason_that_fires() {
    // The RED block is three independent `if`s, not a chain — a run that trips
    // more than one must report more than one.
    let mut rows = series(60, 100.0, 1.0);
    for r in rows.iter_mut().skip(55) { r.close = 40.0; }  // drags 20DMA under 50DMA too
    let (s, d) = classify(&rows, &[], &[]);
    assert_eq!(s, Status::Red);
    assert!(d.reasons.len() >= 2, "expected several RED reasons, got {:?}", d.reasons);
}

/// A series that reaches YELLOW with TWO of its three conditions true.
///
/// Getting here is fiddly and the obvious construction does not work. `current <
/// ma20` together with `rising20 == false` is ORANGE, not YELLOW, so the only
/// route is `current >= ma20` (a bounce) with a still-falling 20DMA — and a
/// bounce makes smh5 strongly positive, so QQQ has to be given an even larger
/// 5-day gain to land rel_qqq5 in (-4, -2]. Solved numerically, not guessed:
///   low 70 → peak 110 over 51 rows, an 8-row dip to 0.86×peak, then a bounce
///   70% of the way back. current 105.380, ma20 104.104, ma50 95.402,
///   rising20 false, rel_qqq5 exactly -3.000, down_days 0.
fn yellow_two_conditions() -> (Vec<Row>, Vec<Row>) {
    let (low, peak, dip_len, dip_to, bounce) = (70.0_f64, 110.0_f64, 8usize, 0.86_f64, 0.70_f64);
    let n_rise = 60 - dip_len - 1;
    let mut smh: Vec<f64> = (0..n_rise)
        .map(|i| low + (peak - low) * i as f64 / (n_rise - 1) as f64)
        .collect();
    for i in 0..dip_len {
        smh.push(peak - (peak - peak * dip_to) * (i + 1) as f64 / dip_len as f64);
    }
    let last = *smh.last().unwrap();
    smh.push(last + (peak - last) * bounce);

    let smh5 = (smh[59] / smh[54] - 1.0) * 100.0;
    let q5 = smh5 + 3.0;                      // rel_qqq5 == -3.0 by construction
    let mut qqq = vec![100.0_f64; 55];
    for i in 1..=5 {
        qqq.push(100.0 * (1.0 + q5 / 100.0 * i as f64 / 5.0));
    }
    let row = |v: &Vec<f64>| -> Vec<Row> {
        v.iter().enumerate().map(|(i, c)| Row { day: format!("d{i:03}"), close: *c }).collect()
    };
    (row(&smh), row(&qqq))
}

#[test]
fn yellow_attaches_exactly_one_reason() {
    // The YELLOW block is if/elif/elif. This fixture makes the second AND third
    // conditions true, so an elif chain yields one reason and three independent
    // ifs yield two. Without a two-condition fixture the distinction is
    // unobservable and the test is decoration.
    let (smh, qqq) = yellow_two_conditions();
    let (s, d) = classify(&smh, &qqq, &[]);
    assert_eq!(s, Status::Yellow, "fixture must reach YELLOW; got {s:?} reasons {:?}", d.reasons);
    assert!(d.rising20 == Some(false), "second condition must hold");
    assert!(d.rel_qqq5.unwrap() <= -2.0, "third condition must hold too: {:?}", d.rel_qqq5);
    assert_eq!(d.reasons.len(), 1, "YELLOW is elif-chained: {:?}", d.reasons);
}

#[test]
fn ok_when_the_trend_is_intact() {
    let rows = series(60, 100.0, 1.0);
    let (s, d) = classify(&rows, &rows, &rows);
    assert_eq!(s, Status::Ok);
    assert!(d.reasons.is_empty(), "{:?}", d.reasons);
}

#[test]
fn yellow_when_underperforming_qqq_by_two_percent() {
    let smh = series(60, 100.0, 0.10);
    let qqq = series(60, 100.0, 1.00);   // QQQ far stronger over the last 5
    let (s, d) = classify(&smh, &qqq, &[]);
    assert!(matches!(s, Status::Yellow | Status::Orange | Status::Red), "got {s:?}");
    assert!(d.rel_qqq5.unwrap() < 0.0);
}

#[test]
fn profit_protect_needs_extension_and_a_down_day() {
    // status still OK, >= 8% above the 20DMA, and at least one down day.
    let mut rows = flat(60, 100.0);
    for r in rows.iter_mut().skip(58) { r.close = 130.0; }
    rows.last_mut().unwrap().close = 129.0;   // one down day, still far extended
    let (s, d) = classify(&rows, &[], &[]);
    assert_eq!(s, Status::ProfitProtect, "reasons {:?} distance20 {:?}", d.reasons, d.distance20);
    assert!(d.distance20.unwrap() >= 8.0);
    assert!(d.down_days >= 1);
}

#[test]
fn extension_without_a_down_day_stays_ok() {
    // The falsifier for PROFIT_PROTECT's `down_days >= 1`. Without this case,
    // deleting that condition changes nothing any test can see, because the
    // profit-protect fixture already has a down day.
    let mut rows = flat(60, 100.0);
    rows.last_mut().unwrap().close = 130.0;    // 28% above the 20DMA, but rising
    let (s, d) = classify(&rows, &[], &[]);
    assert_eq!(s, Status::Ok, "no down day means no PROFIT_PROTECT: {:?}", d.reasons);
    assert!(d.distance20.unwrap() >= 8.0, "the extension condition alone does hold");
    assert_eq!(d.down_days, 0);
}

#[test]
fn details_carries_every_field_the_message_renders() {
    let rows = series(60, 100.0, 1.0);
    let (_, d) = classify(&rows, &rows, &rows);
    assert!(d.ma20.is_some() && d.ma50.is_some());
    assert!(d.distance20.is_some() && d.distance50.is_some());
    assert!(d.smh5.is_some() && d.qqq5.is_some());
    assert_eq!(d.rows, 60);
    assert_eq!(d.day, "d059");
}

#[test]
fn missing_secondary_series_leaves_relatives_none_without_panicking() {
    // update_state hands an empty vec for a ticker whose fetch failed.
    let rows = series(60, 100.0, 1.0);
    let (_, d) = classify(&rows, &[], &[]);
    assert!(d.qqq5.is_none() && d.rel_qqq5.is_none());
    assert!(d.soxx5.is_none() && d.rel_soxx5.is_none());
}
