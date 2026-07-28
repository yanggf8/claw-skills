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

pub fn format_body(level: &str, index: &str, updated: &str, job_id: Option<&str>) -> String {
    let mut s = format!(
        "🍕 DOUGHCON 情報\n目前等級：DOUGHCON {level}\n指數：{index}\n更新：{updated}"
    );
    if let Some(id) = job_id {
        s.push_str(&format!("\n\n`{id}`"));
    }
    s
}
