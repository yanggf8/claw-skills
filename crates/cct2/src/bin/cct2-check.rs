//! Durable check for cct2's models journal — the verdict lives in
//! `cct2::models_check`; this file is the file plumbing and the exit code.
//!
//! Exit: 0 the journal's last day is complete and current, 1 anything else,
//! 2 bad usage (the install probe requires an unknown flag to exit 2).

use std::io::Write;

use cct2::models_check;

fn main() {
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();

    let argv: Vec<String> = std::env::args().skip(1).collect();
    if let Some(other) = argv.first() {
        let _ = writeln!(err, "[ERROR: unknown argument {other}]");
        std::process::exit(2);
    }

    let home = std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let path = home.join(".nullclaw/skills/cct2/journal/models.jsonl");
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let _ = writeln!(out, "[WARN: models.jsonl missing] {} not found", path.display());
            std::process::exit(1);
        }
        Err(e) => {
            let _ = writeln!(out, "[WARN: models.jsonl unreadable] {e}");
            std::process::exit(1);
        }
    };
    let entries: Vec<serde_json::Value> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    if entries.is_empty() {
        let _ = writeln!(out, "[WARN: models.jsonl has no valid JSON]");
        std::process::exit(1);
    }

    // jiff bundles the tzdb, so the ET date needs nothing from the host.
    let et_today = jiff::Timestamp::now()
        .in_tz("America/New_York")
        .expect("tzdb is bundled")
        .date();
    let expected = models_check::last_expected_weekday(et_today);

    let verdict = models_check::evaluate(&entries, Some(expected.to_string().as_str()));
    let latest_date = entries
        .last()
        .and_then(|e| e.get("business_date"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if verdict.ok {
        let runs = entries
            .iter()
            .filter(|e| e.get("business_date").and_then(|v| v.as_str()) == Some(latest_date))
            .count();
        let _ = writeln!(
            out,
            "✅ cct2 models.jsonl durable check ok for {latest_date}: {runs} runs, both modes answered"
        );
        std::process::exit(0);
    }
    let _ = writeln!(
        out,
        "⚠️ cct2 models.jsonl check degraded for {latest_date}: {}",
        verdict.reasons.join("; ")
    );
    std::process::exit(1);
}
