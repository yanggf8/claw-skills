//! Strip agent/harness artifacts from nested-agent stdout before delivery.
//!
//! Port of `lib/skill_runner.strip_agent_artifacts`. Byte-exact contract with
//! the Python oracle — see Phase ② Task 1.

use regex::Regex;
use std::sync::OnceLock;

/// Clean agent stdout before it is delivered to Telegram (or stdout).
///
/// Matches `lib/skill_runner.strip_agent_artifacts` step for step:
/// 1. Paired `<ncchoices>…</ncchoices>` (lazy, dotall, case-insensitive)
/// 2. Unclosed `<ncchoices>` through EOF (dotall, case-insensitive)
/// 3. Harness/cron marker lines (whole line only)
/// 4. Bare job-id lines (whole line only)
/// 5. Optionally collapse blank-line runs
/// 6. Python-compatible strip of edges
pub fn strip_agent_artifacts(text: &str, collapse_blank_lines: bool) -> String {
    static PAIRED: OnceLock<Regex> = OnceLock::new();
    static UNCLOSED: OnceLock<Regex> = OnceLock::new();
    static MARKER: OnceLock<Regex> = OnceLock::new();
    static JOB_ID: OnceLock<Regex> = OnceLock::new();
    static BLANK_RUNS: OnceLock<Regex> = OnceLock::new();

    let paired = PAIRED.get_or_init(|| {
        Regex::new(r"(?is)<ncchoices>.*?</ncchoices>").expect("paired ncchoices regex")
    });
    let unclosed = UNCLOSED.get_or_init(|| {
        Regex::new(r"(?is)<ncchoices>.*$").expect("unclosed ncchoices regex")
    });
    let marker = MARKER.get_or_init(|| {
        Regex::new(r"(?m)^\[(?:skill-status|trace|skill-event)[:\]].*$")
            .expect("marker line regex")
    });
    let job_id = JOB_ID.get_or_init(|| {
        Regex::new(r"(?im)^\s*skill-[0-9a-f-]{8,}(?:-[0-9a-f]+)*:\d+\s*$")
            .expect("job-id line regex")
    });
    let blank_runs = BLANK_RUNS.get_or_init(|| {
        Regex::new(r"\n\n+").expect("blank-run regex")
    });

    let mut text = paired.replace_all(text, "").into_owned();
    text = unclosed.replace_all(&text, "").into_owned();
    text = marker.replace_all(&text, "").into_owned();
    text = job_id.replace_all(&text, "").into_owned();
    if collapse_blank_lines {
        text = blank_runs.replace_all(&text, "\n").into_owned();
    }
    python_strip(&text)
}

/// Python `str.strip()`: Unicode White_Space plus C0 separators that Python
/// treats as whitespace but Rust's `trim()` does not (`\x1c`–`\x1f`, `\x85`).
fn python_strip(s: &str) -> String {
    let is_strip_char = |c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '\u{1c}' | '\u{1d}' | '\u{1e}' | '\u{1f}' | '\u{85}'
            )
    };
    s.trim_matches(is_strip_char).to_string()
}
