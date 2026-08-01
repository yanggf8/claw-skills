//! Weather fallback state machine.
//!
//! Oracle: weather/scripts/run.py lines 255–336 (the HK / CWA / Open-Meteo
//! orchestration inside `main`, without delivery, advice, or markers).
//!
//! B1 and B2 live here. Both look like bugs; both are the contract.

use std::collections::HashMap;

use claw_core::marker::SkillStatus;
use serde_json::Value;

use crate::sources::hko::{self, HkoData};
use crate::sources::open_meteo::{self, OmData};
use crate::sources::{cwa, Row};

/// One run's accumulated output (body lines + advice rows + fallback state).
#[derive(Debug, Clone)]
pub struct Outcome {
    pub lines: Vec<String>,
    pub rows: Vec<Row>,
    pub fallback_used: bool,
    pub fallback_event: Option<FallbackEvent>,
}

/// Fields needed later for `emit_fallback` (elapsed_ms is timed by the caller).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FallbackEvent {
    pub reason: String,
    pub scope: String,
    /// Milliseconds spent in the Open-Meteo window ONLY — matching the Python,
    /// which brackets exactly `open_meteo_for_locations(targets)` (run.py:301-303).
    /// Timing the whole run instead would report a different QUANTITY, not just a
    /// noisier one, and the [skill-event] line is a production diagnostic an agent
    /// reads later. The differential harness masks the digits to absorb jitter; it
    /// cannot absorb measuring the wrong thing.
    pub elapsed_ms: u64,
}

/// Injected sources so unit tests need no network.
pub trait Sources {
    fn hko(&self) -> Result<HkoData, String>;
    /// Raw CWA response body (JSON string).
    fn cwa(&self, locs: &[String]) -> Result<String, String>;
    fn open_meteo(&self, loc: &str) -> Result<OmData, String>;
}

/// B2 ordering: empty rows → failed, else fallback_used → degraded, else ok.
pub fn status_of(out: &Outcome) -> SkillStatus {
    if out.rows.is_empty() {
        SkillStatus::Failed
    } else if out.fallback_used {
        SkillStatus::Degraded
    } else {
        SkillStatus::Ok
    }
}

/// Option A (scheduler contract): hard-failure must not Telegram.
///
/// `claw_core::delivery::deliver(None, …)` echoes the body to stdout and
/// never calls `telegram::send` — that is how cron_runs.output still keeps
/// the diagnostic when we suppress the chat id. Ok / Degraded pass
/// `deliver_to` through unchanged so a stale-but-real report still reaches
/// the user (and still trips retry_once — that is a separate open issue).
pub fn chat_id_for_delivery(status: SkillStatus, deliver_to: Option<&str>) -> Option<&str> {
    match status {
        SkillStatus::Failed => None,
        SkillStatus::Ok | SkillStatus::Degraded => deliver_to,
    }
}

/// Run the HK + TW orchestration against injected sources.
///
/// `api_key` is the resolved `CWA_API_KEY` (empty string ≡ unset — B8).
pub fn run(hk: &[String], tw: &[String], api_key: &str, src: &dyn Sources) -> Outcome {
    let mut lines: Vec<String> = Vec::new();
    let mut rows: Vec<Row> = Vec::new();

    // ── Hong Kong locations via HKO ──────────────────────────────
    // One fetch, N formatted lines (B4). On error, one WARN per alias.
    if !hk.is_empty() {
        match src.hko() {
            Ok(hko_data) => {
                for loc in hk {
                    let (line, data) = hko::format(loc, &hko_data);
                    lines.push(line);
                    if let Some(row) = data {
                        rows.push(row);
                    }
                }
            }
            Err(e) => {
                for _loc in hk {
                    lines.push(format!("[WARN: HKO weather unavailable - {e}]"));
                }
            }
        }
    }

    // ── Taiwan locations via CWA, Open-Meteo fallback ────────────
    let mut fallback_used = false;
    let mut fallback_event: Option<FallbackEvent> = None;

    if !tw.is_empty() {
        let mut cwa_failed_reason: Option<String> = None;
        let mut cwa_unmatched: Vec<String> = Vec::new();

        // B8: empty key is the same as unset.
        if api_key.is_empty() {
            cwa_failed_reason = Some("CWA_API_KEY is not set in the environment".to_string());
        } else {
            // Fallible region: fetch + per-location format share one error
            // boundary. Lines/rows already pushed STAY on error (B1). Do NOT
            // use `?` in a way that abandons this outer accumulation.
            match cwa_try(tw, src, &mut lines, &mut rows) {
                Ok(CwaTryOk {
                    unmatched,
                    empty_records,
                }) => {
                    cwa_unmatched = unmatched;
                    if empty_records {
                        cwa_failed_reason =
                            Some("CWA returned an empty record list".to_string());
                    }
                }
                Err(e) => {
                    // Python: f"CWA request failed with {type(e).__name__}: {e}"
                    cwa_failed_reason = Some(format!("CWA request failed with {e}"));
                }
            }
        }

        // On any CWA failure reason, fall back for ALL tw locs (not just
        // unmatched) — that is what produces B1's deliberate duplicate.
        let targets: Vec<String> = if cwa_failed_reason.is_some() {
            tw.to_vec()
        } else {
            cwa_unmatched.clone()
        };

        if !targets.is_empty() {
            let om_t0 = std::time::Instant::now();
            let (fb_lines, fb_data) = open_meteo_for_locations(&targets, src);
            let elapsed_ms = om_t0.elapsed().as_millis() as u64;
            lines.extend(fb_lines);
            rows.extend(fb_data);
            // B2: set whenever targets is non-empty — success of OM is irrelevant.
            fallback_used = true;
            let reason = cwa_failed_reason.unwrap_or_else(|| {
                format!(
                    "CWA did not return data for {} of {} locations",
                    cwa_unmatched.len(),
                    tw.len()
                )
            });
            let scope = format!(
                "{} Taiwan location{}",
                targets.len(),
                if targets.len() == 1 { "" } else { "s" }
            );
            fallback_event = Some(FallbackEvent { reason, scope, elapsed_ms });
        }
    }

    if lines.is_empty() {
        lines.push("[WARN: no valid locations provided]".to_string());
    }

    Outcome {
        lines,
        rows,
        fallback_used,
        fallback_event,
    }
}

struct CwaTryOk {
    unmatched: Vec<String>,
    empty_records: bool,
}

/// The Python `try` body for CWA: fetch, map, format loop, empty-list check.
/// Accumulates into the caller's `lines`/`rows` as it goes — on Err those
/// pushes are intentionally not rolled back (B1).
fn cwa_try(
    tw: &[String],
    src: &dyn Sources,
    lines: &mut Vec<String>,
    rows: &mut Vec<Row>,
) -> Result<CwaTryOk, String> {
    let body = src.cwa(tw).map_err(|e| format!("Error: {e}"))?;
    let v: Value = serde_json::from_str(&body).map_err(|e| format!("JSONDecodeError: {e}"))?;

    // records = cwa_data.get("records", {}).get("location", []) or []
    let records = location_array(&v);

    // loc_map = {r["locationName"]: r for r in records}
    // Python raises KeyError if locationName is missing — same here.
    let mut loc_map: HashMap<String, &Value> = HashMap::new();
    for r in &records {
        let name = r
            .get("locationName")
            .and_then(|x| x.as_str())
            .ok_or_else(|| "KeyError: 'locationName'".to_string())?;
        loc_map.insert(name.to_string(), *r);
    }

    let mut unmatched: Vec<String> = Vec::new();
    for loc in tw {
        if let Some(raw) = loc_map.get(loc.as_str()) {
            // format_cwa_location is INSIDE the try — a raise mid-loop leaves
            // earlier pushes in place and sets cwa_failed_reason via Err.
            let (line, row) = format_cwa_location(loc, raw)?;
            lines.push(line);
            // B7: CWA appends unconditionally (format always yields a row).
            rows.push(row);
        } else {
            unmatched.push(loc.clone());
        }
    }

    // if not records and not loc_map:
    let empty_records = records.is_empty() && loc_map.is_empty();
    Ok(CwaTryOk {
        unmatched,
        empty_records,
    })
}

/// `data.get("records", {}).get("location", []) or []`
fn location_array(v: &Value) -> Vec<&Value> {
    let Some(obj) = v.as_object() else {
        return Vec::new();
    };
    let records = match obj.get("records") {
        None | Some(Value::Null) => return Vec::new(),
        Some(Value::Object(m)) => m,
        Some(_) => return Vec::new(),
    };
    match records.get("location") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a.iter().collect(),
        Some(_) => Vec::new(),
    }
}

/// Python `format_cwa_location` — can raise KeyError/TypeError on malformed
/// weatherElement items (the B1 mid-loop path). Happy path delegates to
/// `cwa::format` for line/row parity with the adapter unit tests.
fn format_cwa_location(loc_name: &str, loc_data: &Value) -> Result<(String, Row), String> {
    // elements = loc_data.get("weatherElement", [])
    let elements: &[Value] = match loc_data.get("weatherElement") {
        None | Some(Value::Null) => &[],
        Some(Value::Array(a)) => a.as_slice(),
        // Non-list: Python iterates (e.g. string chars) then el["elementName"] TypeErrors.
        Some(_) => {
            return Err("TypeError: string indices must be integers".to_string());
        }
    };

    // by_name[el["elementName"]] = el.get("time", []) — KeyError if missing.
    for el in elements {
        let obj = el
            .as_object()
            .ok_or_else(|| "TypeError: string indices must be integers".to_string())?;
        if !obj.contains_key("elementName") {
            return Err("KeyError: 'elementName'".to_string());
        }
    }

    // Soft-parse via the adapter and format (always yields a row for CWA).
    let wrapped = serde_json::json!({ "records": { "location": [loc_data] } });
    let body = serde_json::to_string(&wrapped).map_err(|e| format!("Error: {e}"))?;
    let recs = cwa::records(&body).map_err(|e| format!("Error: {e}"))?;
    let rec = recs
        .first()
        .ok_or_else(|| "Error: empty CWA location after parse".to_string())?;
    let (line, row) = cwa::format(loc_name, rec);
    let row = row.ok_or_else(|| "Error: CWA format produced no row".to_string())?;
    Ok((line, row))
}

/// Oracle: `open_meteo_for_locations` — per-location coords check then fetch/format.
fn open_meteo_for_locations(locations: &[String], src: &dyn Sources) -> (Vec<String>, Vec<Row>) {
    let mut lines = Vec::new();
    let mut weather_data = Vec::new();
    for loc in locations {
        if open_meteo::tw_coords().get(loc.as_str()).is_none() {
            lines.push(format!(
                "[WARN: weather unavailable - no fallback coordinates for '{loc}']"
            ));
            continue;
        }
        match src.open_meteo(loc) {
            Ok(data) => {
                let (line, wd) = open_meteo::format(loc, &data);
                lines.push(line);
                if let Some(row) = wd {
                    weather_data.push(row);
                }
            }
            Err(e) => {
                lines.push(format!("[WARN: Open-Meteo unavailable for {loc} - {e}]"));
            }
        }
    }
    (lines, weather_data)
}
