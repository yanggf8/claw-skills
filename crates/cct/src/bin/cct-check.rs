//! Watchdog binary: did the CCT generator write the day's reports, and how
//! late? See `cct::watchdog` for the logic and the 2026-09-11 backward-check
//! story; this file is the argument shell around it.
//!
//! Usage:
//!   cct-check                        # today's ET date
//!   cct-check --date 2026-09-12      # replays the 09-11 hole
//!   cct-check --from-file runs.json  # offline, from a capture
//!   cct-check --grace 2.0            # hours of tolerated lag
//!
//! Exit: 0 quiet, 1 a finding, 2 could not evaluate.

use std::io::Write;

use cct::api;
use cct::calendar::weekday_short;
use cct::watchdog;

struct Args {
    date: Option<String>,
    base: Option<String>,
    limit: usize,
    grace: f64,
    from_file: Option<String>,
    timeout: u64,
    db: Option<String>,
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let need = |argv: &[String], i: usize| -> Result<String, String> {
        argv.get(i + 1)
            .cloned()
            .ok_or_else(|| format!("{} requires a value", argv[i]))
    };
    let mut args = Args {
        date: None,
        base: None,
        limit: 200,
        grace: 2.0,
        from_file: None,
        timeout: 15,
        db: None,
    };
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--date" => args.date = Some(need(argv, i)?),
            "--base" => args.base = Some(need(argv, i)?),
            "--limit" => args.limit = need(argv, i)?.parse().map_err(|_| "--limit is a number")?,
            "--grace" => args.grace = need(argv, i)?.parse().map_err(|_| "--grace is a number")?,
            "--from-file" => args.from_file = Some(need(argv, i)?),
            "--timeout" => {
                args.timeout = need(argv, i)?.parse().map_err(|_| "--timeout is a number")?
            }
            "--db" => args.db = Some(need(argv, i)?),
            other => return Err(format!("unknown argument {other}")),
        }
        i += 2;
    }
    Ok(args)
}

fn fmt_grace(g: f64) -> String {
    if (g - g.round()).abs() < f64::EPSILON {
        format!("{}", g as i64)
    } else {
        format!("{g}")
    }
}

fn main() {
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();

    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&argv) {
        Ok(a) => a,
        Err(e) => {
            let _ = writeln!(err, "[ERROR: {e}]");
            std::process::exit(2);
        }
    };

    // The ET business date. jiff bundles the tzdb, so there is no fallback
    // and none is needed: a date derived from UTC arithmetic is the one
    // thing this repo never does.
    let day = match &args.date {
        Some(s) => match s.parse() {
            Ok(d) => d,
            Err(_) => {
                let _ = writeln!(err, "[ERROR: --date is not YYYY-MM-DD] {s}");
                std::process::exit(2);
            }
        },
        None => jiff::Timestamp::now()
            .in_tz("America/New_York")
            .expect("tzdb is bundled")
            .date(),
    };

    let runs = match &args.from_file {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(v) => v
                    .pointer("/data/runs")
                    .or_else(|| v.get("runs"))
                    .and_then(|r| r.as_array())
                    .cloned()
                    .unwrap_or_default(),
                Err(e) => {
                    let _ = writeln!(out, "⚠️ cct generator check could not run: {e}");
                    std::process::exit(2);
                }
            },
            Err(e) => {
                let _ = writeln!(out, "⚠️ cct generator check could not run: {e}");
                std::process::exit(2);
            }
        },
        None => {
            let base = args.base.clone().unwrap_or_else(api::base);
            match watchdog::fetch_runs(
                &base,
                args.limit,
                std::time::Duration::from_secs(args.timeout),
            ) {
                Ok(r) => r,
                Err(e) => {
                    let _ = writeln!(out, "⚠️ cct generator check could not run: {e}");
                    std::process::exit(2);
                }
            }
        }
    };

    let db = args.db.unwrap_or_else(|| {
        std::env::var("NULLCLAW_CRON_DB")
            .unwrap_or_else(|_| format!("{}/.nullclaw/cron.db", std::env::var("HOME").unwrap_or_default()))
    });
    let reads = watchdog::read_times(&db);
    if reads.is_empty() {
        let _ = writeln!(out, "note: cct read times unknown (could not read cron.db) — drift only");
    }

    let mut findings = watchdog::evaluate(&runs, day, args.grace, &reads);
    let (backward, note) = watchdog::backward_findings(&runs, day);
    findings.extend(backward);
    if let Some(note) = note {
        let _ = writeln!(out, "note: {note}");
    }

    let types: Vec<&str> = watchdog::SLOTS
        .iter()
        .filter(|s| s.weekdays.contains(&day.weekday()))
        .map(|s| s.report_type)
        .collect();
    if findings.is_empty() {
        if types.is_empty() {
            let _ = writeln!(
                out,
                "✅ cct generator: no scheduled reports for {day} ({})",
                weekday_short(day.weekday())
            );
            std::process::exit(0);
        }
        let slots = types
            .iter()
            .map(|t| format!("{t} ≤{}h", fmt_grace(args.grace)))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(out, "✅ cct generator on time for {day} ({slots})");
        std::process::exit(0);
    }

    // The header does not repeat {day}: backward findings name earlier days,
    // and an operator sent to check the wrong date stands down too early.
    let _ = writeln!(out, "⚠️ cct generator: {} finding(s)", findings.len());
    for f in &findings {
        let _ = writeln!(out, "   - {}\n     {}", f.head, f.detail);
    }
    let _ = writeln!(
        out,
        "   The worker serves the latest snapshot, so the cct push degrades to\n     the last available report until the generator catches up. The cause\n     is upstream of the skill: the worker's cron triggers (manual fallback:\n     the cct-trigger binary)."
    );
    std::process::exit(1);
}
