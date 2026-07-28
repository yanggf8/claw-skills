//! CLI parsing and body-assembly contract for weather::main.
//!
//! Body helpers are pure so the mutation gate can catch a missing necktie
//! prefix or a job-id footer placed before the advice line without spawning
//! the binary or hitting the network.

use weather::cli::{
    advice_prompt, assemble_body, format_advice_line, parse_args, Args,
};
use weather::sources::Row;

fn argv(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

// ── parse_args ───────────────────────────────────────────────────

#[test]
fn empty_argv_yields_empty_locations_default_applied_later() {
    // Python: args.locations is None when --location is never passed;
    // `or ["臺北市"]` / with_default runs AFTER parse.
    let a = parse_args(&argv(&[])).unwrap();
    assert!(a.locations.is_empty());
    assert!(a.deliver_to.is_none());
    assert_eq!(a.account, "main");
}

#[test]
fn repeated_location_accumulates_in_order() {
    let a = parse_args(&argv(&[
        "--location",
        "臺北市",
        "--location",
        "香港",
        "--location",
        "高雄市",
    ]))
    .unwrap();
    assert_eq!(a.locations, vec!["臺北市", "香港", "高雄市"]);
}

#[test]
fn deliver_to_and_account_parse() {
    let a = parse_args(&argv(&[
        "--deliver-to",
        "42",
        "--account",
        "nunu",
        "--location",
        "臺北市",
    ]))
    .unwrap();
    assert_eq!(a.deliver_to.as_deref(), Some("42"));
    assert_eq!(a.account, "nunu");
    assert_eq!(a.locations, vec!["臺北市"]);
}

#[test]
fn rejects_unknown_flags_and_missing_values() {
    assert!(parse_args(&argv(&["--nope"])).is_err());
    assert!(parse_args(&argv(&["--location"])).is_err());
    assert!(parse_args(&argv(&["--deliver-to"])).is_err());
    assert!(parse_args(&argv(&["--account"])).is_err());
}

#[test]
fn defaults_struct_matches_python() {
    let a = Args {
        locations: vec![],
        deliver_to: None,
        account: "main".into(),
    };
    let parsed = parse_args(&argv(&[])).unwrap();
    assert_eq!(parsed, a);
}

// ── advice prompt (byte-compared by the differential harness) ────

#[test]
fn advice_prompt_uses_en_dash_and_applies_pop_percent_even_to_qualitative() {
    // Contract: EN DASH between temps; 降雨{pop}% even when pop is HKO's
    // qualitative PSR ("高") → literal "降雨高%". Looks like a bug. It is not.
    let rows = vec![
        Row {
            location: "香港".into(),
            wx: "多雲".into(),
            min_t: "24".into(),
            max_t: "30".into(),
            pop: "高".into(),
        },
        Row {
            location: "臺北市".into(),
            wx: "晴".into(),
            min_t: "20".into(),
            max_t: "28".into(),
            pop: "10".into(),
        },
    ];
    let p = advice_prompt(&rows);
    assert!(
        p.contains("香港: 多雲, 24–30°C, 降雨高%"),
        "prompt was: {p}"
    );
    assert!(
        p.contains("臺北市: 晴, 20–28°C, 降雨10%"),
        "prompt was: {p}"
    );
    assert!(p.contains('–'), "must use EN DASH U+2013, not ASCII hyphen");
    assert!(!p.contains("24-30"), "ASCII hyphen between temps is wrong");
    // Two rows joined with "; "
    assert!(p.contains("; "), "rows joined with '; '");
}

// ── necktie prefix (mutation gate a) ─────────────────────────────

#[test]
fn advice_line_gets_necktie_prefix_only_when_nonempty() {
    // Layering decided in Task 3: call_agent returns sanitized text with NO
    // emoji; weather adds "👔 " at the call site only when non-empty.
    assert_eq!(
        format_advice_line("記得帶傘，早晚偏涼").as_deref(),
        Some("👔 記得帶傘，早晚偏涼")
    );
    assert_eq!(format_advice_line("").as_deref(), None);
}

// ── body assembly / job-id footer order (mutation gate b) ────────

#[test]
fn assemble_body_appends_advice_then_job_id_footer() {
    // Ordering is load-bearing (run.py 320–341):
    //   lines → advice line → whole-body job-id footer → deliver
    // Moving the footer BEFORE the advice must turn this red.
    let lines = vec!["🌤 臺北市：晴，低溫20°C / 高溫28°C".to_string()];
    let advice = "👔 記得帶傘";
    let body = assemble_body(&lines, Some(advice), Some("job-abc:1"));
    assert_eq!(
        body,
        "🌤 臺北市：晴，低溫20°C / 高溫28°C\n👔 記得帶傘\n\n`job-abc:1`"
    );
    // Explicit order: advice before the footer backticks.
    let advice_pos = body.find("👔 記得帶傘").expect("advice present");
    let footer_pos = body.find("\n\n`job-abc:1`").expect("footer present");
    assert!(
        advice_pos < footer_pos,
        "job-id footer must come AFTER the advice line"
    );
}

#[test]
fn assemble_body_omits_empty_advice_and_empty_job_id() {
    let lines = vec!["line".to_string()];
    assert_eq!(assemble_body(&lines, None, None), "line");
    assert_eq!(assemble_body(&lines, None, Some("")), "line");
    assert_eq!(
        assemble_body(&lines, Some("👔 x"), None),
        "line\n👔 x"
    );
}
