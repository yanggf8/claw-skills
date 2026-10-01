//! Index derivation and message layout.

use crate::pizzint::RawIndex;

pub const NO_DATA: &str = "-1";

/// "No data" is NOT `index == 0`. Zero collapses to the sentinel only when
/// every place reports null popularity; an empty place list also counts as
/// all-null. A genuine zero with real data stays zero, and a non-integer index
/// is passed through verbatim exactly as Python's f-string would.
///
/// Returns the rendered index string, because Python never narrows the type.
pub fn derive_index(raw_index: &RawIndex, popularity_is_null: &[bool]) -> String {
    let all_null = popularity_is_null.is_empty() || popularity_is_null.iter().all(|n| *n);
    match raw_index {
        RawIndex::Missing => NO_DATA.to_string(),
        // Zero-ness is Python's `== 0`, which covers 0, 0.0, -0.0 and False —
        // not just the integer zero.
        RawIndex::Present { is_zero: true, .. } if all_null => NO_DATA.to_string(),
        RawIndex::Present { rendered, .. } => rendered.clone(),
    }
}

/// A level the API actually sent. `parse` renders an absent `defcon_level`
/// as "?" and a null one as "None" (Python `dict.get` parity); neither
/// carries information, so deliver treats them as no-data.
pub fn level_is_real(level: &str) -> bool {
    level != "?" && level != "None"
}

pub fn format_body(level: &str, index: &str, updated: &str, job_id: Option<&str>) -> String {
    // Owner decision 2026-10-01: the sentinel is a log value, not a reader
    // value. PizzINT's null popularity — the daily reality since 09-26 — must
    // not print as "-1"; 暫缺 says the same thing honestly. A real index is
    // untouched and this reverts on its own when the field returns.
    let shown = if index == NO_DATA { "暫缺" } else { index };
    let mut s = format!(
        "🍕 DOUGHCON 情報\n目前等級：DOUGHCON {level}\n指數：{shown}\n更新：{updated}"
    );
    if let Some(id) = job_id {
        s.push_str(&format!("\n\n`{id}`"));
    }
    s
}
