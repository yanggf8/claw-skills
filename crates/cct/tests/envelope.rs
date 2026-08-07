//! Why every rejected envelope has to say why.
//!
//! On 2026-08-06 the pre-market cron reported `contract_degraded` and the alert
//! carried **no stderr at all**. The worker was answering 200 with
//! `success: true` and no `data` key — a DO cache-hit branch that had picked up
//! an extra `.data` in cct@387f62a — and `api::get` dropped that on the floor
//! through a bare `?`. Every path out of the envelope was silent except the two
//! HTTP ones, so the skill said "尚未產生或暫時無法存取" and diagnosis cost a
//! manual curl.
//!
//! `unwrap_envelope` exists so that sentence is never printed without a reason
//! beside it. These tests pin the reason, not just the `None`.

use cct::api::unwrap_envelope;

/// Captured live from the pre-market route during the incident.
const EMPTY: &str = include_str!("empty_envelope.json");

fn run(text: &str) -> (Option<cct::api::Report>, String) {
    let mut err: Vec<u8> = Vec::new();
    let got = unwrap_envelope(text, &mut err);
    (got, String::from_utf8(err).expect("warnings are utf-8"))
}

#[test]
fn the_incident_envelope_is_rejected_with_a_reason() {
    let (got, warn) = run(EMPTY);
    assert!(got.is_none());
    // The reason has to name the missing field. "CCT unavailable" would have
    // sent the reader to the network, which was fine all along.
    assert!(warn.contains("data"), "got: {warn}");
}

#[test]
fn the_incident_warning_carries_the_cache_provenance() {
    // `source=do_cache` is the whole diagnosis: a 200 with no data and
    // do_cache attached means the cached entry is malformed, not that the
    // pipeline is down. Without it the alert cannot distinguish the two.
    let (_, warn) = run(EMPTY);
    assert!(warn.contains("do_cache"), "got: {warn}");
    assert!(warn.contains("cached=true"), "got: {warn}");
}

#[test]
fn a_healthy_envelope_unwraps_to_its_payload_in_silence() {
    let (got, warn) = run(r#"{"success":true,"data":{"type":"pre_market_briefing","symbols_analyzed":5}}"#);
    assert_eq!(got.unwrap().data["symbols_analyzed"], 5);
    assert_eq!(warn, "", "a good payload must not warn");
}

#[test]
fn a_null_data_field_is_rejected_like_a_missing_one() {
    // `data: null` and no `data` are the same failure to a reader, and
    // JSON.stringify on the worker turns `undefined` into the latter — but a
    // future serialiser could just as well emit the former.
    let (got, warn) = run(r#"{"success":true,"data":null,"metadata":{"source":"do_cache"}}"#);
    assert!(got.is_none());
    assert!(warn.contains("data"), "got: {warn}");
}

#[test]
fn an_outer_failure_is_reported_with_the_servers_own_words() {
    let (got, warn) = run(r#"{"success":false,"error":"Unauthorized","message":"API key required."}"#);
    assert!(got.is_none());
    assert!(warn.contains("Unauthorized"), "got: {warn}");
}

#[test]
fn an_outer_failure_without_an_error_field_still_warns() {
    let (got, warn) = run(r#"{"success":false}"#);
    assert!(got.is_none());
    assert!(!warn.is_empty(), "silence is the bug this file exists for");
}

#[test]
fn a_non_json_body_names_itself_as_such() {
    // A Cloudflare error page is HTML with a 200 more often than it should be.
    let (got, warn) = run("<!DOCTYPE html><title>error</title>");
    assert!(got.is_none());
    assert!(warn.contains("JSON"), "got: {warn}");
}

#[test]
fn the_inner_envelope_failure_is_still_caught() {
    // weekly serves a DO-cache miss as outer success:true wrapping inner
    // success:false. Pre-existing behaviour; pinned so the refactor keeps it.
    let (got, warn) = run(r#"{"success":true,"data":{"success":false,"error":"cache miss"}}"#);
    assert!(got.is_none());
    assert!(warn.contains("cache miss"), "got: {warn}");
}

#[test]
fn a_payload_that_merely_omits_success_is_not_caught_by_the_inner_test() {
    // The other routes omit the key entirely. Testing for falsy instead of an
    // explicit `false` would reject every healthy pre-market report.
    let (got, warn) = run(r#"{"success":true,"data":{"type":"pre_market_briefing","symbols_analyzed":5}}"#);
    assert!(got.is_some());
    assert_eq!(warn, "");
}

#[test]
fn the_envelopes_provenance_survives_the_unwrap() {
    // The whole point of the change. `unwrap_envelope` returned `Some(data)` and
    // dropped everything around it, so the worker could publish business_date
    // perfectly and the skill would still never see it.
    let (got, warn) = run(
        r#"{"success":true,"data":{"symbols_analyzed":5},
            "metadata":{"business_date":"2026-08-06","has_content":true,"source":"d1_fallback"}}"#,
    );
    let report = got.expect("a payload");
    assert_eq!(report.data["symbols_analyzed"], 5);
    assert_eq!(report.business_date.as_deref(), Some("2026-08-06"));
    assert_eq!(report.has_content, Some(true));
    assert_eq!(warn, "");
}

#[test]
fn an_envelope_without_the_fields_is_still_accepted() {
    // A worker that has not shipped the field yet — which is every deploy order
    // where this skill lands first. Absent is not an error; it selects the
    // fallback path rather than costing the reader a report.
    let (got, _) = run(r#"{"success":true,"data":{"symbols_analyzed":5}}"#);
    let report = got.expect("a payload");
    assert_eq!(report.business_date, None);
    assert_eq!(report.has_content, None);
}

#[test]
fn a_business_date_that_is_not_a_string_is_treated_as_absent() {
    // Fail soft, not loud. A malformed provenance field must not cost the reader
    // a report they could otherwise have had; the fallback still answers.
    let (got, _) = run(
        r#"{"success":true,"data":{"symbols_analyzed":5},"metadata":{"business_date":20260806}}"#,
    );
    assert_eq!(got.expect("a payload").business_date, None);
}

#[test]
fn has_content_false_is_carried_through_rather_than_rejected() {
    // An envelope that says it holds nothing is still a valid envelope: the
    // skill needs the day and the flag to say *why* it is degrading. Rejecting
    // it here would put it back on the silent path this file exists for.
    let (got, warn) = run(
        r#"{"success":true,"data":{"type":"end_of_day_summary"},
            "metadata":{"business_date":"2026-08-07","has_content":false}}"#,
    );
    let report = got.expect("an empty report is still a report");
    assert_eq!(report.has_content, Some(false));
    assert_eq!(report.business_date.as_deref(), Some("2026-08-07"));
    assert_eq!(warn, "");
}
