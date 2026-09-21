//! CLI parsing and pure body-assembly helpers for weather.
//!
//! Extracted from `main.rs` so `cargo test` can reach arg parsing and the
//! advice/footer contract without spawning the binary or hitting the network.

use crate::sources::Row;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// Accumulated `--location` values, in order. Empty when the flag is
    /// never passed — `routing::with_default` applies `["臺北市"]` later,
    /// matching Python's `args.locations or ["臺北市"]`.
    pub locations: Vec<String>,
    pub deliver_to: Option<String>,
    pub account: String,
}

/// Parse CLI flags from an argv slice (no program name).
pub fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut a = Args {
        locations: Vec::new(),
        deliver_to: None,
        account: "main".into(),
    };
    let mut i = 0;
    while i < argv.len() {
        let need = |i: usize| -> Result<String, String> {
            argv.get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{} requires a value", argv[i]))
        };
        match argv[i].as_str() {
            "--location" => {
                a.locations.push(need(i)?);
                i += 2;
            }
            "--deliver-to" => {
                a.deliver_to = Some(need(i)?);
                i += 2;
            }
            "--account" => {
                a.account = need(i)?;
                i += 2;
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(a)
}

/// LLM prompt summary — byte-compared by the differential harness.
///
/// Uses EN DASH (U+2013) between temps, and applies `降雨{pop}%` even when
/// `pop` is HKO's qualitative PSR (e.g. `高` → `降雨高%`). That looks like a
/// bug. It is the contract. Do not repair it.
pub fn advice_prompt(rows: &[Row]) -> String {
    let summary = rows
        .iter()
        .map(|d| {
            format!(
                "{}: {}, {}–{}°C, 降雨{}%",
                d.location, d.wx, d.min_t, d.max_t, d.pop
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!(
        "根據以下天氣資料，用繁體中文給出簡短的穿搭建議（1-2句話），\
         包含具體衣物建議和是否需要雨具。只回覆建議本身，不要重複天氣資料。\
         只回覆純文字建議，勿附加 ncchoices、按鈕、選擇清單或任何標記。\n\
         天氣：{summary}"
    )
}

/// Prefix non-empty sanitized advice with the necktie emoji.
///
/// Layering decided in Task 3: `call_agent` returns the sanitized text only
/// (no emoji). The `👔 ` prefix is added HERE, and only when non-empty (B13 —
/// timeout / empty reply omits the line entirely).
pub fn format_advice_line(advice: &str) -> Option<String> {
    if advice.is_empty() {
        None
    } else {
        Some(format!("👔 {advice}"))
    }
}

/// Final message body: weather lines, then optional advice, then optional
/// whole-body job-id footer. Ordering is load-bearing (run.py 320–341).
pub fn assemble_body(lines: &[String], advice_line: Option<&str>, job_id: Option<&str>) -> String {
    let mut parts: Vec<&str> = lines.iter().map(String::as_str).collect();
    if let Some(a) = advice_line {
        parts.push(a);
    }
    let mut output = parts.join("\n");
    if let Some(id) = job_id {
        if !id.is_empty() {
            output.push_str(&format!("\n\n`{id}`"));
        }
    }
    output
}
