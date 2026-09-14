//! Watchdog for the CCT *generator*: did the day's reports get written, and
//! how late?
//!
//! The reader can only report what arrived, never what didn't, so this module
//! reads the worker's run history and names the gap — a missed trigger
//! surfaces as "never ran", a late one as drift against its nominal minute.
//!
//! The backward check (2026-09-14). Every evaluation also looks back at days
//! no live check may have witnessed — the previous trading day's dailies,
//! plus the weekly when the checked day is a Monday or Tuesday. The
//! 2026-09-11 hole stayed invisible for three days because its only
//! scheduled witness — the Sat 00:05Z run — sat on the same box that died,
//! and every later run asked about its own day and called the weekend quiet.
//! The backward windows check the two hard verdicts only (never ran, ended
//! non-success); drift stays a day-of concern.
//!
//! Read-time truth comes from cron.db, not from a copy in this file: the cct
//! read schedule has already drifted out of SKILL.md once, and a second place
//! to keep it current is a second place to be wrong. When the DB cannot be
//! read the drift verdict still runs and the "did it land after the read"
//! column says unknown.

use std::collections::HashMap;
use std::time::Duration;

use jiff::civil::{Date, Weekday};
use jiff::{Timestamp, ToSpan};
use serde_json::Value;

pub const USER_AGENT: &str = "nullclaw-cct/1.0";

const WEEKDAYS: &[Weekday] = &[
    Weekday::Monday,
    Weekday::Tuesday,
    Weekday::Wednesday,
    Weekday::Thursday,
    Weekday::Friday,
];

/// The four slots, exactly as `scheduler.ts` matches them and as the four
/// triggers have carried since 2026-09-02 (and GH Actions `schedule:` before
/// that): fixed UTC minutes, weekday-masked.
pub struct Slot {
    pub report_type: &'static str,
    pub hour: i8,
    pub minute: i8,
    pub weekdays: &'static [Weekday],
}

pub const SLOTS: &[Slot] = &[
    Slot { report_type: "pre-market", hour: 12, minute: 30, weekdays: WEEKDAYS },
    Slot { report_type: "intraday", hour: 16, minute: 0, weekdays: WEEKDAYS },
    Slot { report_type: "end-of-day", hour: 20, minute: 5, weekdays: WEEKDAYS },
    Slot { report_type: "weekly", hour: 14, minute: 0, weekdays: &[Weekday::Sunday] },
];

/// report_type -> the `--mode` the cct skill is invoked with, keying the
/// cron.db lookup for the "did it land after the read" column.
fn consumer_mode(report_type: &str) -> &'static str {
    match report_type {
        "pre-market" => "--mode pre-market",
        "intraday" => "--mode intraday",
        "end-of-day" => "--mode eod",
        "weekly" => "--mode weekly",
        other => unreachable!("unknown report_type {other}"),
    }
}

/// One reason the generator is not on time, rendered for the cron alert.
#[derive(Debug)]
pub struct Finding {
    pub head: String,
    pub detail: String,
}

/// A consumer read's scheduled minute, straight out of cron.db.
#[derive(Clone, Copy)]
pub struct ConsumerRead {
    pub hour: i8,
    pub minute: i8,
    pub offset_s: i64,
}

pub type Reads = HashMap<String, ConsumerRead>;

fn nominal_for(day: Date, hour: i8, minute: i8) -> Timestamp {
    day.to_datetime(jiff::civil::time(hour, minute, 0, 0))
        .to_zoned(jiff::tz::TimeZone::UTC)
        .expect("UTC is a valid zone")
        .timestamp()
}

fn parse_started(raw: &str) -> Option<Timestamp> {
    raw.parse::<Timestamp>().ok()
}

/// Short human form of a timestamp, always in UTC (the times the crons speak).
fn utc_stamp(t: Timestamp, fmt: &str) -> String {
    t.to_zoned(jiff::tz::TimeZone::UTC)
        .strftime(fmt)
        .to_string()
}

/// One finding per report type that has something to say about `day`.
pub fn evaluate(runs: &[Value], day: Date, grace_h: f64, reads: &Reads) -> Vec<Finding> {
    let mut findings = Vec::new();
    for slot in SLOTS {
        if !slot.weekdays.contains(&day.weekday()) {
            continue;
        }
        let rows: Vec<&Value> = runs
            .iter()
            .filter(|r| {
                r.get("report_type").and_then(|v| v.as_str()) == Some(slot.report_type)
                    && r.get("scheduled_date").and_then(|v| v.as_str())
                        == Some(day.to_string().as_str())
            })
            .collect();

        if rows.is_empty() {
            match crate::calendar::holidays(day.year()) {
                Some(list) if list.contains(&day) => continue,
                known => {
                    let note = match known {
                        Some(_) => String::new(),
                        None => format!(" (holiday table has no data for {})", day.year()),
                    };
                    findings.push(Finding {
                        head: format!("{} never ran on {}{}", slot.report_type, day, note),
                        detail: format!(
                            "expected the worker cron trigger at {:02}:{:02}Z",
                            slot.hour, slot.minute
                        ),
                    });
                    continue;
                }
            }
        }

        let last = rows
            .into_iter()
            .max_by_key(|r| {
                r.get("started_at").and_then(|v| v.as_str()).unwrap_or("").to_string()
            })
            .expect("rows is non-empty");
        let status = last.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");
        let started = match last.get("started_at").and_then(|v| v.as_str()).and_then(parse_started)
        {
            Some(t) => t,
            None => {
                findings.push(Finding {
                    head: format!("{} has an unparsable started_at on {}", slot.report_type, day),
                    detail: format!("status={status}"),
                });
                continue;
            }
        };

        // Measure against the nearest nominal trigger, not the one the row is
        // stamped with. A run delivered just after midnight belongs to the
        // previous day's cron — against the new day's nominal time it prints
        // a "-15.2h" drift that reads like a clock bug instead of an
        // eight-hour-late trigger (the 2026-08-28 lesson). Day arithmetic
        // happens on the civil date; a Timestamp cannot take `days` units.
        let candidates = [0i64, 1, 2].map(|k| nominal_for(day - k.days(), slot.hour, slot.minute));
        let nearest = candidates
            .iter()
            .copied()
            .min_by_key(|c| (c.as_second() - started.as_second()).abs())
            .expect("three candidates");
        let drift_h = (started.as_second() - nearest.as_second()) as f64 / 3600.0;
        let stamped = if utc_stamp(nearest, "%Y-%m-%d") != day.to_string() {
            format!(
                " [row stamped {}, nearest trigger {}Z]",
                day.strftime("%m-%d"),
                utc_stamp(nearest, "%m-%d %H:%M")
            )
        } else {
            String::new()
        };

        let mut missed_push = false;
        let mut after_read = String::new();
        if let Some(read) = reads.get(consumer_mode(slot.report_type)) {
            let read_dt = Timestamp::from_second(
                nominal_for(day, read.hour, read.minute).as_second() - read.offset_s,
            )
            .expect("a valid read timestamp");
            if started > read_dt {
                missed_push = true;
                after_read = format!(", after the read at {}Z", utc_stamp(read_dt, "%H:%M"));
            }
        }

        if status != "success" {
            findings.push(Finding {
                head: format!("{} {} on {}", slot.report_type, status, day),
                detail: format!(
                    "started {}Z (drift {drift_h:+.1}h) stage={}{}{}",
                    utc_stamp(started, "%m-%d %H:%M"),
                    last.get("current_stage").and_then(|v| v.as_str()).unwrap_or(""),
                    after_read,
                    stamped
                ),
            });
        } else if drift_h > grace_h || missed_push {
            let mut why = format!(
                "landed {drift_h:+.1}h late ({}Z vs {}Z)",
                utc_stamp(started, "%H:%M"),
                utc_stamp(nearest, "%H:%M")
            );
            if missed_push {
                why.push_str(&after_read);
            }
            why.push_str(&stamped);
            findings.push(Finding {
                head: format!("{} {why}", slot.report_type),
                detail: format!(
                    "status={status} run_id={}",
                    last.get("run_id").and_then(|v| v.as_str()).unwrap_or("")
                ),
            });
        }
    }
    findings
}

/// Missed-or-failed findings for days no live check may have witnessed.
///
/// Two windows:
///
/// - the previous trading day's dailies, so a miss is flagged on the next
///   run even when the run that owned it was lost — 2026-09-11's only
///   witness (the Sat 00:05Z slot) sat on the box that died;
/// - the weekly of the most recent Sunday, when `day` is Monday or Tuesday:
///   weekly's only other witness is the Monday 00:05Z run, the same
///   single-witness shape this check exists to close.
///
/// An unresolved miss therefore alerts on consecutive runs until the
/// calendar carries it out of range. Deliberate: the repeat is the recovery
/// net for exactly the case where the first alert was lost.
///
/// Returns the findings, plus a note when the run history does not reach
/// back far enough to judge the checked days — a retention cutoff must not
/// read as "never ran".
pub fn backward_findings(runs: &[Value], day: Date) -> (Vec<Finding>, Option<String>) {
    let mut findings = Vec::new();
    let prev = crate::calendar::prev_trading_day(day);
    let mut checks: Vec<(&Slot, Date)> = SLOTS
        .iter()
        .filter(|s| s.report_type != "weekly")
        .map(|s| (s, prev))
        .collect();
    if matches!(day.weekday(), Weekday::Monday | Weekday::Tuesday) {
        let back = match day.weekday() {
            Weekday::Monday => 1,
            _ => 2,
        };
        let sunday = day - i64::from(back).days();
        let weekly = SLOTS
            .iter()
            .find(|s| s.report_type == "weekly")
            .expect("weekly slot");
        checks.push((weekly, sunday));
    }

    let oldest = runs
        .iter()
        .filter_map(|r| r.get("scheduled_date").and_then(|v| v.as_str()))
        .min()
        .map(str::to_string);
    let mut note = None;
    for (slot, checked) in checks {
        if let Some(o) = oldest.as_deref() {
            if o > checked.to_string().as_str() {
                note = Some(format!(
                    "run history starts at {o}; the backward check cannot judge earlier days"
                ));
                continue;
            }
        }
        if let Some(f) = missing_or_failed(runs, slot, checked) {
            findings.push(f);
        }
    }
    (findings, note)
}

/// The hard verdicts for one slot on one day: never ran, or ended
/// non-success. Holidays are excused. Rows are attributed by nearest nominal
/// trigger rather than by the `scheduled_date` stamp — the stamp is what the
/// worker filed the run under, and a trigger late enough crosses that
/// boundary (the same distrust `evaluate` already learned).
fn missing_or_failed(runs: &[Value], slot: &Slot, day: Date) -> Option<Finding> {
    if !slot.weekdays.contains(&day.weekday()) {
        return None;
    }
    let known_year = crate::calendar::holidays(day.year()).is_some();
    if let Some(list) = crate::calendar::holidays(day.year()) {
        if list.contains(&day) {
            return None;
        }
    }

    let nominal = nominal_for(day, slot.hour, slot.minute);
    let stamps = [day, day + 1.day()];
    let mut attributed: Option<(&Value, Timestamp)> = None;
    for r in runs {
        if r.get("report_type").and_then(|v| v.as_str()) != Some(slot.report_type) {
            continue;
        }
        let Some(stamp) = r.get("scheduled_date").and_then(|v| v.as_str()) else {
            continue;
        };
        if !stamps.iter().any(|d| d.to_string() == stamp) {
            continue;
        }
        let Some(t) = r.get("started_at").and_then(|v| v.as_str()).and_then(parse_started)
        else {
            continue;
        };
        let neighbours = [
            nominal_for(day - 1.day(), slot.hour, slot.minute),
            nominal,
            nominal_for(day + 1.day(), slot.hour, slot.minute),
        ];
        let nearest = neighbours
            .iter()
            .copied()
            .min_by_key(|n| (n.as_second() - t.as_second()).abs())
            .expect("three candidates");
        if nearest.as_second() != nominal.as_second() {
            continue;
        }
        let is_newer = attributed
            .as_ref()
            .map(|(_, prev_t)| t > *prev_t)
            .unwrap_or(true);
        if is_newer {
            attributed = Some((r, t));
        }
    }

    let Some((row, _)) = attributed else {
        let note = if known_year {
            String::new()
        } else {
            format!(" (holiday table has no data for {})", day.year())
        };
        return Some(Finding {
            head: format!("{} never ran on {}{}", slot.report_type, day, note),
            detail: format!(
                "the expected {:02}:{:02}Z trigger left no attributable run",
                slot.hour, slot.minute
            ),
        });
    };

    let status = row.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");
    if status != "success" {
        return Some(Finding {
            head: format!("{} {status} on {}", slot.report_type, day),
            detail: format!(
                "started {} stage={}",
                row.get("started_at").and_then(|v| v.as_str()).unwrap_or(""),
                row.get("current_stage").and_then(|v| v.as_str()).unwrap_or("")
            ),
        });
    }
    None
}

/// GET the run history from the worker, unwrapping the runs envelope.
pub fn fetch_runs(base: &str, limit: usize, timeout: Duration) -> Result<Vec<Value>, String> {
    let url = format!("{}/api/v1/jobs/runs?limit={limit}", base.trim_end_matches('/'));
    let resp = claw_core::http::agent(timeout)
        .get(&url)
        .set("X-API-Key", &crate::api::api_key())
        .set("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(code, _) => format!("HTTP {code}"),
            other => format!("{other}"),
        })?;
    let text = resp
        .into_string()
        .map_err(|e| format!("response body unreadable - {e}"))?;
    let body: Value = serde_json::from_str(&text)
        .map_err(|e| format!("response is not JSON - {e}"))?;
    if body.get("success").and_then(|v| v.as_bool()) != Some(true) {
        let head: String = body.to_string().chars().take(160).collect();
        return Err(format!("envelope not successful: {head}"));
    }
    body.pointer("/data/runs")
        .and_then(|v| v.as_array())
        .cloned()
        .ok_or_else(|| "envelope carries no data.runs".to_string())
}

/// mode arg -> (hour, minute, tz_offset_s) for the live cct read jobs.
///
/// cron.db is WAL and the daemon holds it, so the file set is copied to a
/// temp dir first — the same thing a backup does — and the copy is opened.
/// Any failure degrades to an empty map: the caller then says the read
/// column is unknown, and the drift verdict still runs.
pub fn read_times(db_path: &str) -> Reads {
    read_times_impl(db_path).unwrap_or_default()
}

fn read_times_impl(db_path: &str) -> Result<Reads, String> {
    let tmp = std::env::temp_dir().join(format!("cct-watchdog-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    for suffix in ["", "-wal", "-shm"] {
        let src = format!("{db_path}{suffix}");
        if std::path::Path::new(&src).exists() {
            std::fs::copy(&src, tmp.join(format!("cron.db{suffix}")))
                .map_err(|e| e.to_string())?;
        }
    }
    let result = query_reads(&tmp.join("cron.db"));
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

fn query_reads(copy: &std::path::Path) -> Result<Reads, String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async {
        let db = libsql::Builder::new_local(copy)
            .build()
            .await
            .map_err(|e| e.to_string())?;
        let conn = db.connect().map_err(|e| e.to_string())?;
        let mut rows = conn
            .query(
                "select skill_args, expression, tz_offset_s from cron_jobs \
                 where skill_name = 'cct' and enabled = 1 and paused = 0",
                libsql::params!(),
            )
            .await
            .map_err(|e| e.to_string())?;
        let mut out = Reads::new();
        while let Some(row) = rows.next().await.map_err(|e| e.to_string())? {
            let args: String = row.get(0).map_err(|e| e.to_string())?;
            let expr: String = row.get(1).map_err(|e| e.to_string())?;
            let off: i64 = row.get(2).map_err(|e| e.to_string())?;
            let fields: Vec<&str> = expr.split_whitespace().collect();
            if fields.len() < 5 {
                continue;
            }
            let minute: i8 = fields[0].parse().map_err(|_| "bad cron minute")?;
            let hour: i8 = fields[1].parse().map_err(|_| "bad cron hour")?;
            out.insert(args.trim().to_string(), ConsumerRead { hour, minute, offset_s: off });
        }
        Ok(out)
    })
}
