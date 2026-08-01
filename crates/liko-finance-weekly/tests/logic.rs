//! Tests for the two pieces of this skill that are logic rather than
//! orchestration. Everything else shells out to persona-core, and pinning a
//! subprocess invocation in a unit test pins the call, not the behaviour.

use liko_finance_weekly::issues::{already_done, status_of};
use liko_finance_weekly::schedule::next_sunday_taipei;

fn at(s: &str) -> jiff::Timestamp {
    s.parse().expect("timestamp")
}

// ── next_sunday_taipei ───────────────────────────────────────────────────────

#[test]
fn a_sunday_returns_itself() {
    // 2026-08-02 is a Sunday. Not "next week" — the cron fires 09:00 Sunday
    // Taipei and the issue it prepares is for that same day.
    assert_eq!(next_sunday_taipei(at("2026-08-02T01:00:00Z")), "2026-08-02");
}

#[test]
fn a_monday_looks_six_days_ahead() {
    assert_eq!(next_sunday_taipei(at("2026-08-03T01:00:00Z")), "2026-08-09");
}

#[test]
fn a_saturday_looks_one_day_ahead() {
    assert_eq!(next_sunday_taipei(at("2026-08-01T01:00:00Z")), "2026-08-02");
}

#[test]
fn the_date_is_taipeis_not_utcs() {
    // 2026-08-01 17:00 UTC is already Sunday 2026-08-02 01:00 in Taipei. Read
    // in UTC this is a Saturday and would return 08-02 as *tomorrow*; read in
    // Taipei it is Sunday and returns 08-02 as *today*. Same string, different
    // reason — so the case that actually distinguishes them is the one below.
    assert_eq!(next_sunday_taipei(at("2026-08-01T17:00:00Z")), "2026-08-02");
}

#[test]
fn a_late_sunday_utc_is_already_monday_in_taipei() {
    // 2026-08-02 20:00 UTC is Monday 2026-08-03 04:00 Taipei. In UTC it is
    // still Sunday, which would return 08-02 — an issue for a date that has
    // already passed in Taipei. This is the case the timezone matters for.
    assert_eq!(next_sunday_taipei(at("2026-08-02T20:00:00Z")), "2026-08-09");
}

#[test]
fn the_cron_slot_lands_on_its_own_sunday() {
    // The job runs 09:00 Sunday Taipei = 01:00 Sunday UTC.
    assert_eq!(next_sunday_taipei(at("2026-08-02T01:00:00Z")), "2026-08-02");
    assert_eq!(next_sunday_taipei(at("2026-08-09T01:00:00Z")), "2026-08-09");
}

// ── issue status parsing ─────────────────────────────────────────────────────

// The longer id comes FIRST on purpose. Without a trailing space in the
// needle, `id=iss-002` prefix-matches the `iss-0021` line, and `lines().find`
// returns the first match — so a listing ordered the other way round hides the
// bug entirely. Discovered by breaking the needle and watching the suite stay
// green.
const LISTING: &str = "\
id=iss-001 status=draft target_date=2026-07-26
id=iss-0021 status=draft target_date=2026-08-09
id=iss-002 status=published target_date=2026-08-02
";

#[test]
fn the_status_of_a_listed_issue_is_read_back() {
    assert_eq!(status_of(LISTING, "iss-002").as_deref(), Some("published"));
}

#[test]
fn a_shorter_id_does_not_match_a_longer_one_listed_before_it() {
    // The case that actually distinguishes the two implementations. Looking up
    // the SHORT id against a listing where the LONG one appears first: with a
    // trailing space the needle skips iss-0021 and finds iss-002; without one
    // it stops at iss-0021 and reports the wrong issue's status.
    assert_eq!(status_of(LISTING, "iss-002").as_deref(), Some("published"));
}

#[test]
fn a_longer_id_is_found_on_its_own_line() {
    assert_eq!(status_of(LISTING, "iss-0021").as_deref(), Some("draft"));
}

#[test]
fn an_unlisted_issue_has_no_status() {
    assert!(status_of(LISTING, "iss-999").is_none());
}

#[test]
fn a_line_without_a_status_field_yields_none() {
    assert!(status_of("id=iss-003 target_date=2026-08-16", "iss-003").is_none());
}

#[test]
fn published_and_delivered_both_mean_there_is_nothing_to_do() {
    assert!(already_done(Some("published")));
    assert!(already_done(Some("delivered")));
}

#[test]
fn any_other_state_means_the_run_proceeds() {
    // Notably `skipped`: a run that failed validation last week should be
    // retried, not treated as finished.
    assert!(!already_done(Some("draft")));
    assert!(!already_done(Some("validated")));
    assert!(!already_done(Some("skipped")));
    assert!(!already_done(None));
}

// ── agent body markers ───────────────────────────────────────────────────────

use liko_finance_weekly::proc::body_between;

const B: &str = "BEGIN_ISSUE_BODY";
const E: &str = "END_ISSUE_BODY";

#[test]
fn the_body_is_taken_from_between_the_markers() {
    let reply = "Let me research this.\nBEGIN_ISSUE_BODY\n本週訊號\n…\nEND_ISSUE_BODY\nHope that helps.";
    assert_eq!(body_between(reply, B, E), "本週訊號\n…");
}

#[test]
fn narration_before_the_marker_is_dropped() {
    // The reason the markers exist. A model told to research narrates first,
    // and that narration would otherwise be published.
    let reply = "I'll check Tier A sources first…\nBEGIN_ISSUE_BODY\nreal body\nEND_ISSUE_BODY";
    assert!(!body_between(reply, B, E).contains("Tier A sources first"));
}

#[test]
fn a_reply_without_markers_is_kept_whole() {
    // Not discarded. The model answered; the validator downstream decides
    // whether the answer is usable. Dropping it would turn a formatting slip
    // into a missing week.
    assert_eq!(body_between("本週訊號\n…", B, E), "本週訊號\n…");
}

#[test]
fn an_unclosed_marker_keeps_what_followed_it() {
    // Cut off mid-write. What came after the opening marker is the start of the
    // body, not narration.
    let reply = "prelude\nBEGIN_ISSUE_BODY\n本週訊號\n第一段";
    assert_eq!(body_between(reply, B, E), "本週訊號\n第一段");
}
