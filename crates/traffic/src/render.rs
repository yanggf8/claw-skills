//! Output formatting.
//!
//! Ports the tail of `main()` in traffic/scripts/run.py:134-147.

/// Seconds to whole minutes.
///
/// Half rounds up: 150 seconds is 3 minutes, not 2.
///
/// The Python this replaces used `round()`, which in Python 3 rounds halves to
/// even — so it showed 2. That was never a decision anybody made about travel
/// time; it is what `round()` happens to do, and it is wrong for the purpose.
/// A commuter reading "2 分鐘" for a two-and-a-half-minute leg is being told
/// something false to no benefit, and the same route at 210 seconds already
/// rounded up to 4. Reproducing that inconsistency for parity's sake would
/// have made the Python's accident into this program's specification.
pub fn minutes_from_seconds(seconds: i64) -> i64 {
    (seconds as f64 / 60.0).round() as i64
}

/// The human-readable route, built from the names the caller passed rather
/// than the resolved coordinates (run.py:135-139).
pub fn label(origin: &str, via: Option<&str>, dest: &str) -> String {
    let mut parts = vec![origin];
    if let Some(v) = via {
        parts.push(v);
    }
    parts.push(dest);
    parts.join("→")
}

/// Assemble the delivered message.
///
/// `advice` is the already-decorated string (with its 💡) or empty, matching
/// how run.py:142-143 treats the LLM result.
pub fn body(label: &str, minutes: i64, advice: &str, job_id: Option<&str>) -> String {
    // Full-width colon, as in run.py:140.
    let mut out = format!("🚗 {label}：{minutes}分鐘");
    if !advice.is_empty() {
        out.push('\n');
        out.push_str(advice);
    }
    if let Some(id) = job_id {
        out.push_str(&format!("\n\n`{id}`"));
    }
    out
}
