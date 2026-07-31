//! Characterization tests for the traffic skill, written before the port.
//!
//! `traffic/scripts/run.py` shipped with no tests at all, so there was no
//! oracle to port — these were written by reading the Python and pinning what
//! it observably does. Every expected string here was taken from the Python
//! source, not from the Rust implementation, so the Rust has to meet the
//! Python rather than the tests being written to whatever the Rust happens to
//! produce.
//!
//! Where a case exists because Python and Rust disagree by default, the test
//! says so.

use traffic::locations::{resolve, ResolveError};
use traffic::render::{body, label, minutes_from_seconds};
use traffic::route::{status_message, transport_message, travel_time_seconds, RouteError};

fn locs() -> Vec<(String, String)> {
    vec![
        ("淡水安泰登峰".to_string(), "25.1802,121.4432".to_string()),
        ("昌吉街重北路口".to_string(), "25.0672,121.5158".to_string()),
    ]
}

// ── locations::resolve ───────────────────────────────────────────────────────

#[test]
fn a_known_name_resolves_to_its_coordinates() {
    assert_eq!(
        resolve("淡水安泰登峰", &locs()).unwrap(),
        "25.1802,121.4432"
    );
}

#[test]
fn a_raw_lat_lon_passes_through_unchanged() {
    assert_eq!(resolve("25.1,121.5", &locs()).unwrap(), "25.1,121.5");
}

#[test]
fn a_raw_lat_lon_keeps_its_whitespace() {
    // run.py:45-47 parses `parts[0].strip()` to decide, then returns `name`
    // itself — the un-stripped original. Trimming here would silently change
    // the URL that gets built.
    assert_eq!(resolve(" 25.1 , 121.5 ", &locs()).unwrap(), " 25.1 , 121.5 ");
}

#[test]
fn a_name_in_the_map_wins_over_looking_like_coordinates() {
    // run.py:40 checks the map first, unconditionally.
    let table = vec![("1,2".to_string(), "9.9,9.9".to_string())];
    assert_eq!(resolve("1,2", &table).unwrap(), "9.9,9.9");
}

#[test]
fn three_comma_separated_parts_are_not_coordinates() {
    assert!(matches!(
        resolve("1,2,3", &locs()),
        Err(ResolveError::Unknown(_))
    ));
}

#[test]
fn a_non_numeric_pair_is_not_coordinates() {
    assert!(matches!(
        resolve("here,there", &locs()),
        Err(ResolveError::Unknown(_))
    ));
}

#[test]
fn the_unknown_location_message_is_reproduced_exactly() {
    // This string reaches the user through
    // `print(f"[WARN: traffic unavailable - {e}]")` at run.py:121, so its
    // wording and punctuation are part of the delivered output.
    let err = resolve("台北車站", &locs()).unwrap_err();
    assert_eq!(
        err.to_string(),
        "Unknown location: '台北車站'. Add it to locations.json or use 'lat,lon'."
    );
}

// ── render::minutes_from_seconds ─────────────────────────────────────────────

#[test]
fn a_half_minute_rounds_up() {
    // Deliberately NOT what the Python did.
    //
    // Python's round() rounds halves to even, so it reported 150 seconds as
    // 2 minutes and 210 as 4 — the same rule giving down then up on adjacent
    // half-minutes. Nobody chose that for travel time; it is a property of
    // round() that leaked into the output.
    //
    // A minute display should round the way a reader expects. Half goes up,
    // every time.
    assert_eq!(minutes_from_seconds(90), 2); // 1.5 → 2
    assert_eq!(minutes_from_seconds(150), 3); // 2.5 → 3   (Python said 2)
    assert_eq!(minutes_from_seconds(210), 4); // 3.5 → 4
    assert_eq!(minutes_from_seconds(270), 5); // 4.5 → 5   (Python said 4)
    assert_eq!(minutes_from_seconds(330), 6); // 5.5 → 6
}

#[test]
fn minutes_round_normally_away_from_the_halves() {
    assert_eq!(minutes_from_seconds(0), 0);
    assert_eq!(minutes_from_seconds(29), 0);
    assert_eq!(minutes_from_seconds(31), 1);
    assert_eq!(minutes_from_seconds(1_500), 25);
}

// ── render::label ────────────────────────────────────────────────────────────

#[test]
fn a_two_point_label_joins_with_an_arrow() {
    assert_eq!(label("A", None, "B"), "A→B");
}

#[test]
fn a_via_point_sits_in_the_middle() {
    assert_eq!(label("A", Some("M"), "B"), "A→M→B");
}

#[test]
fn the_label_uses_the_names_given_not_the_resolved_coordinates() {
    // run.py:135-139 builds the label from args, not from `waypoints`.
    assert_eq!(label("淡水安泰登峰", None, "昌吉街重北路口"), "淡水安泰登峰→昌吉街重北路口");
}

// ── render::body ─────────────────────────────────────────────────────────────

#[test]
fn the_body_is_the_route_line_alone_when_there_is_no_advice() {
    assert_eq!(body("A→B", 25, "", None), "🚗 A→B：25分鐘");
}

#[test]
fn advice_goes_on_its_own_line() {
    assert_eq!(
        body("A→B", 25, "💡 路況順暢", None),
        "🚗 A→B：25分鐘\n💡 路況順暢"
    );
}

#[test]
fn a_job_id_is_appended_in_backticks_after_a_blank_line() {
    assert_eq!(body("A→B", 25, "", Some("job-7")), "🚗 A→B：25分鐘\n\n`job-7`");
}

#[test]
fn the_job_id_follows_the_advice_when_both_are_present() {
    assert_eq!(
        body("A→B", 25, "💡 x", Some("job-7")),
        "🚗 A→B：25分鐘\n💡 x\n\n`job-7`"
    );
}

// ── route::travel_time_seconds ───────────────────────────────────────────────

#[test]
fn the_first_route_summary_supplies_the_travel_time() {
    let payload = serde_json::json!({
        "routes": [
            {"summary": {"travelTimeInSeconds": 1_500}},
            {"summary": {"travelTimeInSeconds": 9_999}}
        ]
    });
    assert_eq!(travel_time_seconds(&payload).unwrap(), 1_500);
}

#[test]
fn an_empty_route_list_is_the_no_routes_error() {
    // run.py:65 — the message is user-visible via the WARN line.
    let payload = serde_json::json!({"routes": []});
    let err = travel_time_seconds(&payload).unwrap_err();
    assert_eq!(err.to_string(), "No routes returned by TomTom API");
}

#[test]
fn an_absent_route_list_is_the_same_error() {
    // Python's `data.get("routes", [])` treats absent and empty alike.
    let payload = serde_json::json!({"status": "error"});
    assert!(matches!(
        travel_time_seconds(&payload),
        Err(RouteError::NoRoutes)
    ));
}

#[test]
fn a_missing_summary_is_an_error_rather_than_a_zero() {
    // Python raises KeyError here, which main() catches and reports as
    // `[WARN: traffic unavailable - ...]` with exit 0. What must not happen is
    // a silent 0 minutes, which would render as a plausible "0分鐘" route.
    let payload = serde_json::json!({"routes": [{}]});
    assert!(travel_time_seconds(&payload).is_err());
}

// ── route errors must never carry the API key ────────────────────────────────

#[test]
fn an_http_error_never_leaks_the_request_url() {
    // Found by adversarial review. `RouteError::Http(e.to_string())` on a ureq
    // error rendered the whole request URL, and that string is printed as
    //
    //     [WARN: traffic unavailable - https://…?key=REAL_KEY&traffic=true: status code 401]
    //
    // …on stdout, which under commute IS the delivered Telegram message. The
    // Python it replaces says only "HTTP Error 401: Unauthorized".
    //
    // Asserting on the absence of "key=" rather than on an exact string: the
    // wording may change, the leak must not come back under any wording.
    let err = RouteError::Http(status_message(401));
    let rendered = err.to_string();
    assert!(!rendered.contains("key="), "leaked the key: {rendered}");
    assert!(!rendered.contains("api.tomtom.com"), "leaked the URL: {rendered}");
    assert_eq!(rendered, "HTTP Error 401: Unauthorized");
}

#[test]
fn a_transport_error_is_reported_without_the_url() {
    let err = RouteError::Http(transport_message("dns error"));
    let rendered = err.to_string();
    assert!(!rendered.contains("key="), "leaked the key: {rendered}");
    assert!(!rendered.contains("api.tomtom.com"), "leaked the URL: {rendered}");
}

#[test]
fn known_status_codes_read_like_pythons_urllib() {
    assert_eq!(status_message(401), "HTTP Error 401: Unauthorized");
    assert_eq!(status_message(403), "HTTP Error 403: Forbidden");
    assert_eq!(status_message(429), "HTTP Error 429: Too Many Requests");
    // An unlisted code still renders, without inventing a reason phrase.
    assert_eq!(status_message(418), "HTTP Error 418");
}
