//! Fed-braking watch (doc 191 composite, mechanical half only).
//!
//! The composite watch condition is "bubble-context persists (human-read) ×
//! EFFR +100bp over 6 months (mechanical)". This module automates ONLY the
//! mechanical half. Pinned here:
//! - rule math is month-index based on FEDFUNDS (latest vs 6 observations
//!   earlier), never a daily-series approximation (Codex 2026-10-02 finding 4)
//! - the daily change detector keeps a persisted cursor: first run is a
//!   baseline, unchanged is a no-op, and the cursor advances only after a
//!   successful delivery (finding 2 — the blocker: value-vs-previous-distinct
//!   re-alerts forever)
//! - rendered output carries NO status vocabulary and no advice wording —
//!   the braking section is deliberately ladder-free (cds-con precedent;
//!   finding 6)
//! - monthly report failure of the braking series omits the section without
//!   degrading the inflation skill status (finding 1: separate runs).

use inflation_con::analysis::Obs;
use inflation_con::braking::{
    baseline_line, detect, no_op_line, render_daily_message, render_monthly_section, rule_math,
    Cursor, Decision, RULE_BP,
};
use inflation_con::run::{run, Env};
use market_fetch::fred::CreditError;

/// Monthly FEDFUNDS-shaped rows, 2026-03 onward.
fn monthly(vals: &[f64]) -> Vec<Obs> {
    (0..vals.len())
        .map(|i| Obs {
            day: format!("2026-{:02}-01", i + 3),
            value: vals[i],
        })
        .collect()
}

/// Explicit daily DFEDTARU-shaped rows.
fn daily(pairs: &[(&str, f64)]) -> Vec<Obs> {
    pairs
        .iter()
        .map(|(d, v)| Obs {
            day: d.to_string(),
            value: *v,
        })
        .collect()
}

/// Enough history for classify() in monthly-mode tests (12 "months").
fn rows(n: usize) -> Vec<Obs> {
    (0..n)
        .map(|i| Obs {
            day: format!("2026-{:02}-01", i % 12 + 1),
            value: 100.0 + i as f64,
        })
        .collect()
}

const NOW: &str = "2026-10-02 05:30:00 CST";

// ── pure: rule math ─────────────────────────────────────────────────────────

#[test]
fn rule_math_is_month_index_not_calendar() {
    // 2026-03..2026-09, seven monthly observations; +12bp over the window.
    let ff = monthly(&[3.63, 3.63, 3.63, 3.63, 3.63, 3.63, 3.75]);
    let r = rule_math(&ff).expect("7 monthly obs must compute");
    assert_eq!(r.latest_month, "2026-09");
    assert_eq!(r.earlier_month, "2026-03");
    assert_eq!(r.latest, 3.75);
    assert_eq!(r.earlier, 3.63);
    assert_eq!(r.rise_bp, 12);
    assert_eq!(RULE_BP, 100.0);
}

#[test]
fn rule_math_needs_seven_months() {
    assert!(rule_math(&monthly(&[3.63; 6])).is_none());
}

#[test]
fn rule_math_rounds_half_a_basis_point() {
    // 3.625 -> 3.75 is +12.5bp; rendered as an integer bp figure.
    let ff = monthly(&[3.625, 3.625, 3.625, 3.625, 3.625, 3.625, 3.75]);
    assert_eq!(rule_math(&ff).unwrap().rise_bp, 13);
}

// ── pure: change detection against the cursor ───────────────────────────────

#[test]
fn first_run_without_cursor_is_a_baseline_not_a_delivery() {
    let t = daily(&[("2026-09-01", 3.75), ("2026-10-01", 3.75)]);
    match detect(None, &t) {
        Decision::Baseline { cursor } => {
            assert_eq!(cursor.date, "2026-10-01");
            assert_eq!(cursor.value, 3.75);
        }
        other => panic!("expected Baseline, got {other:?}"),
    }
}

#[test]
fn unchanged_target_is_a_noop() {
    let t = daily(&[("2026-09-01", 3.75), ("2026-10-01", 3.75)]);
    let c = Cursor {
        date: "2026-09-01".into(),
        value: 3.75,
    };
    assert!(matches!(detect(Some(&c), &t), Decision::Unchanged));
}

#[test]
fn a_change_after_the_cursor_is_detected() {
    let t = daily(&[
        ("2026-09-01", 3.75),
        ("2026-10-28", 3.75),
        ("2026-10-29", 4.00),
        ("2026-10-30", 4.00),
    ]);
    let c = Cursor {
        date: "2026-09-01".into(),
        value: 3.75,
    };
    match detect(Some(&c), &t) {
        Decision::Changed { from, to, intermediate } => {
            assert_eq!(from, ("2026-09-01".to_string(), 3.75));
            assert_eq!(to, ("2026-10-30".to_string(), 4.00));
            assert!(intermediate.is_empty(), "{intermediate:?}");
        }
        other => panic!("expected Changed, got {other:?}"),
    }
}

#[test]
fn a_round_trip_back_to_the_cursor_value_is_still_a_change() {
    // 3.75 -> 4.00 -> 3.75 while we were away: latest equals the cursor, but
    // a hike happened and was cut. Reporting "unchanged" would hide a rise.
    let t = daily(&[
        ("2026-09-01", 3.75),
        ("2026-10-29", 4.00),
        ("2026-11-02", 3.75),
    ]);
    let c = Cursor {
        date: "2026-09-01".into(),
        value: 3.75,
    };
    match detect(Some(&c), &t) {
        Decision::Changed { from, to, intermediate } => {
            assert_eq!(from, ("2026-09-01".to_string(), 3.75));
            assert_eq!(to, ("2026-11-02".to_string(), 3.75));
            assert_eq!(intermediate, vec![("2026-10-29".to_string(), 4.00)]);
        }
        other => panic!("expected Changed, got {other:?}"),
    }
}

#[test]
fn intermediates_collapse_runs_of_equal_values() {
    // 4.00 for several days is ONE intermediate water level, not five.
    let t = daily(&[
        ("2026-09-01", 3.75),
        ("2026-10-29", 4.00),
        ("2026-10-30", 4.00),
        ("2026-10-31", 4.00),
        ("2026-11-02", 4.25),
    ]);
    let c = Cursor {
        date: "2026-09-01".into(),
        value: 3.75,
    };
    match detect(Some(&c), &t) {
        Decision::Changed { intermediate, .. } => {
            assert_eq!(
                intermediate,
                vec![("2026-10-29".to_string(), 4.00)]
            );
        }
        other => panic!("expected Changed, got {other:?}"),
    }
}

// ── render: wording contract ────────────────────────────────────────────────

/// Words a ladder-free section must never contain. The English half is
/// inflation-con's status vocabulary (case-sensitive, as rendered); the
/// Chinese half is verdict/advice wording (Codex finding 6).
const FORBIDDEN: [&str; 10] = [
    "OK", "WATCH", "YELLOW", "RED", "成立", "觸發", "警戒", "建議", "買", "賣",
];

fn assert_ladder_free(text: &str) {
    for w in FORBIDDEN {
        assert!(!text.contains(w), "braking output must not contain {w}: {text}");
    }
}

#[test]
fn monthly_section_renders_arithmetic_and_the_human_half_note() {
    let ff = monthly(&[3.63, 3.63, 3.63, 3.63, 3.63, 3.63, 3.75]);
    let r = rule_math(&ff).unwrap();
    let t = Obs {
        day: "2026-10-01".into(),
        value: 3.75,
    };
    let s = render_monthly_section(Some(&r), 7, Some(&t));
    assert!(s.contains("Fed braking"), "{s}");
    assert!(s.contains("FEDFUNDS 2026-09 月均:3.75%"), "{s}");
    assert!(s.contains("六個月前(2026-03):3.63%"), "{s}");
    assert!(s.contains("+12bp"), "{s}");
    assert!(s.contains("參考線:+100bp"), "{s}");
    assert!(s.contains("DFEDTARU:3.75%(2026-10-01)"), "{s}");
    assert!(s.contains("泡沫語境由人判讀;本工具不判讀"), "{s}");
    assert_ladder_free(&s);
}

#[test]
fn monthly_section_with_insufficient_history_says_so_without_guessing() {
    let s = render_monthly_section(None, 6, None);
    assert!(s.contains("月觀察值不足(6/7)"), "{s}");
    assert!(s.contains("DFEDTARU:n/a"), "{s}");
    assert_ladder_free(&s);
}

#[test]
fn monthly_section_is_empty_when_the_series_was_unavailable() {
    assert_eq!(render_monthly_section(None, 0, None), "");
}

#[test]
fn daily_message_states_the_move_and_the_rule_without_a_verdict() {
    let ff = monthly(&[3.63, 3.63, 3.63, 3.63, 3.63, 3.63, 3.75]);
    let r = rule_math(&ff).unwrap();
    let m = render_daily_message(
        &("2026-09-01".to_string(), 3.75),
        &("2026-10-30".to_string(), 4.00),
        &[("2026-10-15".to_string(), 3.50)],
        Some(&r),
    );
    assert!(m.contains("3.75%(2026-09-01) → 4.00%(2026-10-30)"), "{m}");
    assert!(m.contains("變動量:+25bp"), "{m}");
    assert!(m.contains("期間另有水位:3.50%(2026-10-15)"), "{m}");
    assert!(m.contains("參考線:+100bp"), "{m}");
    assert_ladder_free(&m);
}

#[test]
fn no_op_and_baseline_lines_are_quiet_and_factual() {
    let t = Obs {
        day: "2026-10-01".into(),
        value: 3.75,
    };
    let n = no_op_line(Some(&t));
    assert!(n.contains("no-op") && n.contains("3.75%(2026-10-01)"), "{n}");
    assert_ladder_free(&n);
    let b = baseline_line(&Cursor {
        date: "2026-10-01".into(),
        value: 3.75,
    });
    assert!(b.contains("baseline") && b.contains("不遞送"), "{b}");
    assert_ladder_free(&b);
}

// ── run-level: mode dispatch, cursor persistence, delivery ──────────────────

fn env(job: Option<&str>, home: &std::path::Path) -> Env {
    Env {
        job_id: job.map(String::from),
        home: home.to_path_buf(),
    }
}

fn tmp() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "inflation-con-b-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn cursor_path(home: &std::path::Path) -> std::path::PathBuf {
    home.join(".nullclaw")
        .join("skills")
        .join("inflation-con")
        .join("braking-cursor.json")
}

fn go(
    argv: &[&str],
    job: Option<&str>,
    home: &std::path::Path,
    f: &dyn Fn(&str) -> Result<Vec<Obs>, CreditError>,
) -> (i32, String, String) {
    let a: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    let (mut o, mut e) = (Vec::new(), Vec::new());
    let code = run(&a, &env(job, home), f, NOW, &mut o, &mut e);
    (
        code,
        String::from_utf8(o).unwrap(),
        String::from_utf8(e).unwrap(),
    )
}

/// Braking fetch stub: the two braking series plus working inflation series.
fn braking_fetch(sid: &str) -> Result<Vec<Obs>, CreditError> {
    match sid {
        "FEDFUNDS" => Ok(monthly(&[3.63, 3.63, 3.63, 3.63, 3.63, 3.63, 3.75])),
        "DFEDTARU" => Ok(daily(&[
            ("2026-09-01", 3.75),
            ("2026-10-28", 3.75),
            ("2026-10-29", 4.00),
        ])),
        _ => Ok(rows(12)),
    }
}

fn write_seed_cursor(home: &std::path::Path, c: &Cursor) {
    let p = cursor_path(home);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, c.to_json()).unwrap();
}

fn read_cursor(home: &std::path::Path) -> Cursor {
    let p = cursor_path(home);
    Cursor::from_json(&std::fs::read_to_string(&p).unwrap()).unwrap()
}

#[test]
fn braking_first_run_is_a_silent_baseline() {
    let home = tmp();
    let (code, out, err) = go(
        &["inflation-con", "--mode", "braking"],
        Some("job-b1"),
        &home,
        &braking_fetch,
    );
    assert_eq!(code, 0);
    assert!(err.is_empty(), "{err}");
    assert!(out.contains("baseline"), "{out}");
    assert!(out.contains("不遞送"), "{out}");
    // no delivery body on a baseline — the historical target level is not news
    assert!(!out.contains("變動"), "{out}");
    assert!(out.contains("[skill-status:ok]"));
    assert!(out.contains("[trace:job-b1]"));
    let c = read_cursor(&home);
    assert_eq!(c.date, "2026-10-29");
    assert_eq!(c.value, 4.00);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn braking_unchanged_run_is_a_noop() {
    let home = tmp();
    write_seed_cursor(
        &home,
        &Cursor {
            date: "2026-10-29".into(),
            value: 4.00,
        },
    );
    let (code, out, err) = go(
        &["inflation-con", "--mode", "braking"],
        Some("job-b2"),
        &home,
        &braking_fetch,
    );
    assert_eq!(code, 0);
    assert!(err.is_empty(), "{err}");
    assert!(out.contains("no-op"), "{out}");
    assert!(!out.contains("變動"), "{out}");
    assert!(out.contains("[skill-status:ok]"));
    // cursor untouched by a no-op
    let c = read_cursor(&home);
    assert_eq!(c.value, 4.00);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn braking_change_delivers_and_advances_the_cursor() {
    let home = tmp();
    // cursor before the 2026-10-29 hike; latest target is 4.00
    write_seed_cursor(
        &home,
        &Cursor {
            date: "2026-09-01".into(),
            value: 3.75,
        },
    );
    let (code, out, err) = go(
        &["inflation-con", "--mode", "braking"],
        Some("job-b3"),
        &home,
        &braking_fetch,
    );
    assert_eq!(code, 0);
    assert!(err.is_empty(), "{err}");
    // no --deliver-to: the body prints to stdout (lib/delivery.py None path)
    assert!(out.contains("FED BRAKING"), "{out}");
    assert!(out.contains("3.75%(2026-09-01) → 4.00%(2026-10-29)"), "{out}");
    assert!(out.contains("變動量:+25bp"), "{out}");
    assert!(out.contains("[skill-status:ok]"));
    // the cursor advanced to the delivered value
    let c = read_cursor(&home);
    assert_eq!(c.date, "2026-10-29");
    assert_eq!(c.value, 4.00);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn braking_failed_delivery_keeps_the_cursor() {
    let home = tmp();
    write_seed_cursor(
        &home,
        &Cursor {
            date: "2026-09-01".into(),
            value: 3.75,
        },
    );
    // --deliver-to with a temp HOME: no telegram config exists there, so the
    // send fails and deliver returns FailedFatal (fail_on_delivery_error).
    let (code, _out, _err) = go(
        &["inflation-con", "--mode", "braking", "--deliver-to", "12345"],
        Some("job-b4"),
        &home,
        &braking_fetch,
    );
    assert_eq!(code, 1, "a failed delivery must exit non-zero");
    // cursor NOT advanced: the next run must re-report the change
    let c = read_cursor(&home);
    assert_eq!(c.date, "2026-09-01");
    assert_eq!(c.value, 3.75);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn braking_target_series_failure_is_a_hard_error() {
    let home = tmp();
    let f = |sid: &str| -> Result<Vec<Obs>, CreditError> {
        match sid {
            "DFEDTARU" => Err(CreditError::Http("network down".into())),
            "FEDFUNDS" => Ok(monthly(&[3.63; 7])),
            _ => Ok(rows(12)),
        }
    };
    let (code, out, err) = go(
        &["inflation-con", "--mode", "braking"],
        Some("job-b5"),
        &home,
        &f,
    );
    assert_eq!(code, 1);
    assert!(err.contains("INFLATION-CON failed"), "{err}");
    assert!(err.contains("DFEDTARU"), "{err}");
    assert!(out.contains("[skill-status:failed]"), "{out}");
    // no cursor file — nothing was delivered
    assert!(!cursor_path(&home).exists());
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_corrupt_cursor_is_refused_not_silently_rebaselined() {
    let home = tmp();
    let p = cursor_path(&home);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, "{not json").unwrap();
    let (code, _out, err) = go(
        &["inflation-con", "--mode", "braking"],
        Some("job-b6"),
        &home,
        &braking_fetch,
    );
    assert_eq!(code, 1);
    assert!(err.contains("braking cursor"), "{err}");
    // the corrupt file must still be there — no silent overwrite
    assert!(p.exists());
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn monthly_report_gains_a_bounded_braking_section() {
    let home = tmp();
    let (code, out, err) = go(&["inflation-con"], Some("job-b7"), &home, &braking_fetch);
    assert_eq!(code, 0);
    assert!(err.is_empty(), "{err}");
    assert!(out.contains("──── Fed braking"), "{out}");
    assert!(out.contains("FEDFUNDS 2026-09 月均:3.75%"), "{out}");
    assert!(out.contains("參考線:+100bp"), "{out}");
    // the inflation core still governs the skill status
    assert!(out.contains("[skill-status:ok]"));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn monthly_braking_failure_omits_the_section_without_degrading() {
    let home = tmp();
    let f = |sid: &str| -> Result<Vec<Obs>, CreditError> {
        match sid {
            "FEDFUNDS" => Err(CreditError::Http("boom".into())),
            "DFEDTARU" => Ok(daily(&[("2026-10-01", 3.75)])),
            _ => Ok(rows(12)),
        }
    };
    let (code, out, _err) = go(&["inflation-con"], Some("job-b8"), &home, &f);
    assert_eq!(code, 0);
    assert!(!out.contains("Fed braking"), "section must be omitted: {out}");
    // inflation status is unaffected by the braking add-on
    assert!(out.contains("[skill-status:ok]"), "{out}");
    let _ = std::fs::remove_dir_all(&home);
}

// ── argument validation ─────────────────────────────────────────────────────

#[test]
fn braking_is_a_valid_mode_and_the_refusals_still_hold() {
    let home = tmp();
    let (code, out, err) = go(
        &["inflation-con", "--mode", "braking"],
        None,
        &home,
        &braking_fetch,
    );
    assert_eq!(code, 0);
    assert!(out.contains("baseline"), "first run with no job id: {out}");
    assert!(!out.contains("[skill-status:"), "no markers without a job id");

    let (code, _, err) = go(
        &["inflation-con", "--mode", "brake"],
        None,
        &tmp(),
        &braking_fetch,
    );
    assert_eq!(code, 2);
    assert!(err.contains("invalid choice: 'brake'"), "{err}");
    assert!(err.contains("'braking'"), "the valid set must list braking: {err}");

    let (code, out, _) = go(
        &["inflation-con", "--mode", "braking", "--bogus"],
        None,
        &tmp(),
        &braking_fetch,
    );
    assert_eq!(code, 2);
    assert!(out.is_empty(), "nothing delivered on refusal: {out}");
    let _ = std::fs::remove_dir_all(&home);
}
