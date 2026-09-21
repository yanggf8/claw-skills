//! The generator watchdog, against the days this incident produced.
//!
//! Rows are synthetic but shaped exactly as the worker's
//! `/api/v1/jobs/runs` serves them; the dates are the September 2026 week
//! the 09-11 hole came from, which is the case this module exists to catch.

use cct::watchdog::{backward_findings, evaluate, ConsumerRead, Reads};
use jiff::civil::Date;
use serde_json::json;

fn d(s: &str) -> Date {
    s.parse().unwrap()
}

fn run(report_type: &str, scheduled_date: &str, status: &str, started_at: &str) -> serde_json::Value {
    json!({
        "report_type": report_type,
        "scheduled_date": scheduled_date,
        "status": status,
        "started_at": started_at,
        "current_stage": null,
        "run_id": "r1"
    })
}

fn healthy_day(date: &str) -> Vec<serde_json::Value> {
    vec![
        run("pre-market", date, "success", &format!("{date}T12:31:00Z")),
        run("intraday", date, "success", &format!("{date}T16:00:30Z")),
        run("end-of-day", date, "success", &format!("{date}T20:06:00Z")),
    ]
}

#[test]
fn a_healthy_thursday_is_quiet_in_both_directions() {
    let mut runs = healthy_day("2026-09-09");
    runs.extend(healthy_day("2026-09-10"));
    assert!(evaluate(&runs, d("2026-09-10"), 2.0, &Reads::new()).is_empty());
    let (backward, note) = backward_findings(&runs, d("2026-09-10"));
    assert!(backward.is_empty(), "{backward:?}");
    assert!(note.is_none());
}

#[test]
fn the_0911_hole_is_flagged_by_the_runs_that_followed_it() {
    // Friday never ran; the worker's history only knows the healthy days
    // around it — the state the live API served on 2026-09-14.
    let mut runs = healthy_day("2026-09-09");
    runs.extend(healthy_day("2026-09-10"));
    runs.push(run("weekly", "2026-09-13", "success", "2026-09-13T14:00:08Z"));

    // Saturday's and Sunday's checks both look back to Friday through
    // prev_trading_day, and Monday's adds the Sunday weekly (ran, so quiet).
    for day in ["2026-09-12", "2026-09-13", "2026-09-14"] {
        let (backward, _) = backward_findings(&runs, d(day));
        assert_eq!(backward.len(), 3, "{day}: {backward:?}");
        assert!(
            backward
                .iter()
                .all(|f| f.head.contains("never ran on 2026-09-11")),
            "{day}: {backward:?}"
        );
    }

    // Tuesday's check moves off the hole: Monday's dailies and the Sunday
    // weekly are both present, so the alert window closes on its own.
    runs.extend(healthy_day("2026-09-14"));
    let (backward, _) = backward_findings(&runs, d("2026-09-15"));
    assert!(backward.is_empty(), "{backward:?}");
}

#[test]
fn a_late_run_stamped_the_next_day_still_covers_its_slot() {
    // The stamp distrust: eod fired eight hours late, crossed into the next
    // ET day, and the worker filed it under the stamp it fired at — the
    // backward check must not call Friday's eod missing because of it.
    let runs = vec![
        run("pre-market", "2026-09-11", "success", "2026-09-11T12:31:00Z"),
        run("intraday", "2026-09-11", "success", "2026-09-11T16:01:00Z"),
        run("end-of-day", "2026-09-12", "success", "2026-09-12T04:05:00Z"),
    ];
    let (backward, _) = backward_findings(&runs, d("2026-09-12"));
    // All three are attributed to Friday — the late one included — so the
    // window is quiet.
    assert!(backward.is_empty(), "{backward:?}");
}

#[test]
fn a_failed_run_is_a_hard_finding_through_the_backward_window() {
    let runs = vec![run("pre-market", "2026-09-11", "failed", "2026-09-11T12:31:00Z")];
    let (backward, _) = backward_findings(&runs, d("2026-09-12"));
    assert_eq!(backward.len(), 3, "{backward:?}");
    assert!(
        backward
            .iter()
            .any(|f| f.head.contains("pre-market failed on 2026-09-11")),
        "{backward:?}"
    );
}

#[test]
fn history_that_ends_before_the_checked_day_is_reported_not_fabricated() {
    // A retention cutoff must not read as "never ran": the note says the
    // window cannot be judged, and no finding fires.
    let runs = vec![run("pre-market", "2026-09-10", "success", "2026-09-10T12:31:00Z")];
    let (backward, note) = backward_findings(&runs, d("2026-09-10"));
    assert!(backward.is_empty(), "{backward:?}");
    assert!(note.is_some());
}

#[test]
fn a_sunday_weekly_missed_after_a_lost_monday_run_is_still_witnessed() {
    // Weekly's only other witness is the Monday 00:05Z run — the same
    // single-witness shape the daily backward check was built to close. If
    // that run is lost, Tuesday's check looks back to Sunday itself.
    let runs = healthy_day("2026-09-11"); // Friday dailies ran; no weekly row
    let (backward, _) = backward_findings(&runs, d("2026-09-14"));
    assert_eq!(backward.len(), 1, "only the Sunday weekly is missing: {backward:?}");
    assert!(
        backward
            .iter()
            .any(|f| f.head.contains("weekly never ran on 2026-09-13")),
        "{backward:?}"
    );

    // And a healthy weekly clears it.
    let mut runs = healthy_day("2026-09-11");
    runs.push(run("weekly", "2026-09-13", "success", "2026-09-13T14:00:08Z"));
    let (backward, _) = backward_findings(&runs, d("2026-09-14"));
    assert!(backward.is_empty(), "{backward:?}");
}

#[test]
fn drift_beyond_grace_is_a_day_of_finding() {
    // GH-era shape: pre-market eight hours late; the rest of the day healthy.
    let mut runs = healthy_day("2026-09-11");
    runs[0] = run("pre-market", "2026-09-11", "success", "2026-09-11T20:30:00Z");
    let f = evaluate(&runs, d("2026-09-11"), 2.0, &Reads::new());
    assert_eq!(f.len(), 1, "{f:?}");
    assert!(f[0].head.contains("landed +8.0h late"), "{}", f[0].head);
}

#[test]
fn a_run_that_landed_after_the_consumer_read_is_flagged() {
    let mut runs = healthy_day("2026-09-11");
    runs[0] = run("pre-market", "2026-09-11", "success", "2026-09-11T16:00:00Z");
    let mut reads = Reads::new();
    reads.insert(
        "--mode pre-market".to_string(),
        ConsumerRead { hour: 15, minute: 35, offset_s: 0 },
    );
    let f = evaluate(&runs, d("2026-09-11"), 2.0, &reads);
    assert_eq!(f.len(), 1, "{f:?}");
    assert!(f[0].head.contains("after the read"), "{}", f[0].head);
}
