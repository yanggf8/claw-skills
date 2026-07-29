use weather::orchestrate::{chat_id_for_delivery, run, status_of, Sources};
use weather::sources::hko::{HkoData, HkoForecast};
use weather::sources::open_meteo::OmData;
use claw_core::delivery::{deliver, DeliverOptions, DeliveryOutcome};
use claw_core::marker::SkillStatus;

fn v(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

/// Minimal valid CWA location JSON for a single named place (past-only slot).
fn cwa_loc_json(name: &str) -> String {
    format!(
        r#"{{
            "locationName": "{name}",
            "weatherElement": [
                {{"elementName": "Wx", "time": [{{"startTime": "2020-01-01 00:00:00", "parameter": {{"parameterName": "晴"}}}}]}},
                {{"elementName": "MinT", "time": [{{"startTime": "2020-01-01 00:00:00", "parameter": {{"parameterName": "20"}}}}]}},
                {{"elementName": "MaxT", "time": [{{"startTime": "2020-01-01 00:00:00", "parameter": {{"parameterName": "28"}}}}]}},
                {{"elementName": "PoP", "time": [{{"startTime": "2020-01-01 00:00:00", "parameter": {{"parameterName": "10"}}}}]}}
            ]
        }}"#
    )
}

/// Body with only the named locations present (partial match = omit some).
fn cwa_body_for(locs: &[&str]) -> String {
    let parts: Vec<String> = locs.iter().map(|n| cwa_loc_json(n)).collect();
    format!(r#"{{"records":{{"location":[{}]}}}}"#, parts.join(","))
}

/// Body with good locations plus one poison location whose weatherElement
/// item lacks `elementName` — Python's `el["elementName"]` raises KeyError
/// mid-loop after earlier locations have already been formatted.
fn cwa_body_poison(good: &[&str], poison: &str) -> String {
    let mut parts: Vec<String> = good.iter().map(|n| cwa_loc_json(n)).collect();
    parts.push(format!(
        r#"{{
            "locationName": "{poison}",
            "weatherElement": [{{"time": []}}]
        }}"#
    ));
    format!(r#"{{"records":{{"location":[{}]}}}}"#, parts.join(","))
}

fn sample_hko() -> HkoData {
    HkoData {
        forecasts: vec![HkoForecast {
            wx: "多雲".into(),
            min_t: "24".into(),
            max_t: "30".into(),
            psr: "高".into(),
        }],
    }
}

fn sample_om() -> OmData {
    OmData {
        weather_codes: vec![1],
        max_temps: vec![30.0],
        min_temps: vec![24.0],
        pops: vec![10.0],
    }
}

/// Scriptable fake. `cwa_body` None means the fetch itself fails.
struct Fake {
    cwa_body: Option<String>,
    om_ok: bool,
    /// When set, the body encodes a malformed record for this location so the
    /// format step inside the CWA try raises (B1). Kept for test readability;
    /// the poison is baked into `cwa_body` at construction.
    #[allow(dead_code)]
    cwa_poison: Option<String>,
    hko: Result<HkoData, String>,
}

impl Fake {
    fn cwa_with(locs: &[&str]) -> Self {
        Self {
            cwa_body: Some(cwa_body_for(locs)),
            om_ok: true,
            cwa_poison: None,
            hko: Ok(sample_hko()),
        }
    }

    fn om_ok() -> Self {
        Self {
            cwa_body: None,
            om_ok: true,
            cwa_poison: None,
            hko: Ok(sample_hko()),
        }
    }

    fn om_all_fail() -> Self {
        Self {
            cwa_body: None,
            om_ok: false,
            cwa_poison: None,
            hko: Ok(sample_hko()),
        }
    }

    fn cwa_poison_on(poison: &str) -> Self {
        // Both requested TW locations appear in the body; the poison one raises
        // at format time. 臺北市 is the non-poison sibling used by the B1 test.
        let good: &[&str] = if poison == "臺北市" {
            &["高雄市"]
        } else {
            &["臺北市"]
        };
        Self {
            cwa_body: Some(cwa_body_poison(good, poison)),
            om_ok: true,
            cwa_poison: Some(poison.to_string()),
            hko: Ok(sample_hko()),
        }
    }

    fn hko_ok() -> Self {
        Self {
            cwa_body: None,
            om_ok: true,
            cwa_poison: None,
            hko: Ok(sample_hko()),
        }
    }

    fn cwa_empty_records() -> Self {
        Self {
            cwa_body: Some(r#"{"records":{"location":[]}}"#.to_string()),
            om_ok: true,
            cwa_poison: None,
            hko: Ok(sample_hko()),
        }
    }
}

impl Sources for Fake {
    fn hko(&self) -> Result<HkoData, String> {
        self.hko.clone()
    }

    fn cwa(&self, _locs: &[String]) -> Result<String, String> {
        match &self.cwa_body {
            Some(b) => Ok(b.clone()),
            None => Err("CWA fetch failed (fake)".into()),
        }
    }

    fn open_meteo(&self, _loc: &str) -> Result<OmData, String> {
        if self.om_ok {
            Ok(sample_om())
        } else {
            Err("Open-Meteo fetch failed (fake)".into())
        }
    }
}

#[test]
fn happy_path_is_ok_with_no_fallback() {
    let out = run(&[], &v(&["臺北市"]), "key", &Fake::cwa_with(&["臺北市"]));
    assert_eq!(out.lines.len(), 1);
    assert_eq!(out.rows.len(), 1);
    assert!(!out.fallback_used);
    assert!(out.fallback_event.is_none());
    assert_eq!(status_of(&out), SkillStatus::Ok);
}

#[test]
fn empty_api_key_is_treated_as_unset() {
    // B8
    let out = run(&[], &v(&["臺北市"]), "", &Fake::om_ok());
    assert!(out.fallback_used);
    assert_eq!(
        out.fallback_event.unwrap().reason,
        "CWA_API_KEY is not set in the environment"
    );
}

#[test]
fn partial_match_falls_back_only_for_unmatched() {
    let out = run(&[], &v(&["臺北市", "高雄市"]), "key", &Fake::cwa_with(&["臺北市"]));
    assert!(out.fallback_used);
    assert_eq!(
        out.fallback_event.unwrap().reason,
        "CWA did not return data for 1 of 2 locations"
    );
    assert_eq!(out.lines.len(), 2, "one CWA line + one Open-Meteo line");
}

#[test]
fn partial_progress_then_error_keeps_the_lines_and_refetches_everything() {
    // B1 — THE case. 臺北市 formats fine, 高雄市 raises. The Python keeps the
    // 臺北市 CWA line AND falls back for BOTH locations, so 臺北市 appears twice.
    // A Rust `?` would drop the first line; falling back only for unmatched
    // would drop the duplicate. Both are wrong.
    let out = run(&[], &v(&["臺北市", "高雄市"]), "key", &Fake::cwa_poison_on("高雄市"));
    let taipei_lines = out.lines.iter().filter(|l| l.contains("臺北市")).count();
    assert_eq!(taipei_lines, 2, "expected the CWA line AND its Open-Meteo duplicate");
    assert!(out.fallback_event.unwrap().reason.starts_with("CWA request failed with"));
}

#[test]
fn fallback_used_is_set_even_when_every_open_meteo_call_fails() {
    // B2, first half.
    let out = run(&[], &v(&["臺北市"]), "", &Fake::om_all_fail());
    assert!(out.fallback_used, "the attempt counts, not its success");
    assert!(out.fallback_event.is_some(), "the [skill-event] is still emitted");
    assert!(out.rows.is_empty());
}

#[test]
fn failed_outranks_degraded() {
    // B2, second half. No rows => failed, even though fallback_used is true.
    let out = run(&[], &v(&["臺北市"]), "", &Fake::om_all_fail());
    assert_eq!(status_of(&out), SkillStatus::Failed);
}

/// Option A: hard-failure suppresses the chat id so deliver() never hits
/// Telegram. Ok / Degraded must keep the chat id (degraded still delivers).
#[test]
fn chat_id_suppressed_only_on_failed() {
    assert_eq!(
        chat_id_for_delivery(SkillStatus::Failed, Some("42")),
        None,
        "Failed must not Telegram"
    );
    assert_eq!(
        chat_id_for_delivery(SkillStatus::Ok, Some("42")),
        Some("42"),
        "Ok path must be unchanged"
    );
    assert_eq!(
        chat_id_for_delivery(SkillStatus::Degraded, Some("42")),
        Some("42"),
        "Degraded still delivers by design"
    );
    assert_eq!(
        chat_id_for_delivery(SkillStatus::Failed, None),
        None
    );
}

/// End-to-end through deliver(): Failed + --deliver-to still writes the body
/// to stdout and never attempts a Telegram send (PrintedToStdout, empty err).
///
/// Why a pure chat_id assert is not enough: deliver(None) is the contract that
/// preserves cron_runs.output; a chat_id helper could return None while main
/// still called deliver(Some(...)). Composing both is the plumbing assertion
/// phase-1 lessons demand.
///
/// Config has no bot token so a leaked chat_id fails immediately (FailedFatal
/// + [delivery] on stderr) instead of hanging on a real network attempt.
#[test]
fn failed_path_echoes_body_without_telegram() {
    let out = run(&[], &v(&["臺北市"]), "", &Fake::om_all_fail());
    assert_eq!(status_of(&out), SkillStatus::Failed);
    let body = out.lines.join("\n");
    assert!(!body.is_empty(), "diagnostic body must exist to capture");

    let chat = chat_id_for_delivery(status_of(&out), Some("42"));

    let mut cfg = std::env::temp_dir();
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    cfg.push(format!("weather-del-{}-{}.json", std::process::id(), n));
    std::fs::write(&cfg, br#"{"channels":{"telegram":{}}}"#).unwrap();

    let opts = DeliverOptions {
        config_path: Some(cfg.clone()),
        // Never contacted when chat is None or token is missing.
        base_url: Some("http://127.0.0.1:1".into()),
        fail_on_delivery_error: true,
        ..Default::default()
    };
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let outcome = deliver(chat, &body, &opts, &mut stdout, &mut stderr);
    let _ = std::fs::remove_file(&cfg);

    assert_eq!(
        outcome,
        DeliveryOutcome::PrintedToStdout,
        "Failed must not attempt Telegram (leaked chat_id → FailedFatal)"
    );
    let printed = String::from_utf8_lossy(&stdout);
    assert!(
        printed.contains(body.trim_end()) || printed.contains(&body),
        "body must still reach stdout for cron_runs.output; got {printed:?}"
    );
    assert!(
        String::from_utf8_lossy(&stderr).is_empty(),
        "no [delivery] diagnostic when Telegram is never attempted; got {:?}",
        String::from_utf8_lossy(&stderr)
    );
}

#[test]
fn degraded_when_the_fallback_produced_something() {
    let out = run(&[], &v(&["臺北市"]), "", &Fake::om_ok());
    assert_eq!(status_of(&out), SkillStatus::Degraded);
}

#[test]
fn scope_is_singular_for_one_location() {
    let out = run(&[], &v(&["臺北市"]), "", &Fake::om_ok());
    assert_eq!(out.fallback_event.unwrap().scope, "1 Taiwan location");
    let out2 = run(&[], &v(&["臺北市", "高雄市"]), "", &Fake::om_ok());
    assert_eq!(out2.fallback_event.unwrap().scope, "2 Taiwan locations");
}

#[test]
fn hk_locations_never_trigger_a_fallback_event() {
    let out = run(&v(&["香港"]), &[], "key", &Fake::hko_ok());
    assert!(!out.fallback_used);
    assert!(out.fallback_event.is_none());
}

#[test]
fn repeated_hk_aliases_produce_one_line_each_from_one_fetch() {
    // B4
    let out = run(&v(&["香港", "九龍", "香港"]), &[], "key", &Fake::hko_ok());
    assert_eq!(out.lines.len(), 3);
    assert_eq!(out.rows.len(), 3);
}

#[test]
fn empty_records_uses_the_empty_list_reason() {
    let out = run(&[], &v(&["臺北市"]), "key", &Fake::cwa_empty_records());
    assert_eq!(out.fallback_event.unwrap().reason, "CWA returned an empty record list");
}
