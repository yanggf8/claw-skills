//! Turning a `Quote` into the delivered text.

use crate::quote::Quote;

/// The `+3186.45 (+7.98%)` half of a headline.
///
/// `None` when it cannot be computed — an unparseable price, or a previous
/// close of zero. The Python reached the same outcome by catching ValueError
/// and ZeroDivisionError; expressing it as an Option keeps the "there is no
/// answer" case from being confused with a computed zero.
///
/// The sign is applied once and shared by both numbers: at or above zero they
/// get a `+`, below it they carry their own `-`. Two decimals, which matches
/// Python's `.2f` — both round the underlying double half-to-even, verified on
/// 2.675, 1.005 and 2.345.
pub fn change_suffix(price: f64, prev: f64) -> Option<String> {
    if prev == 0.0 {
        return None;
    }
    let change = price - prev;
    let pct = change / prev * 100.0;
    let sign = if change >= 0.0 { "+" } else { "" };
    Some(format!("{sign}{change:.2} ({sign}{pct:.2}%)"))
}

/// One quote as one or two lines.
///
/// Headline always; the indented detail line only when there is something to
/// put on it.
pub fn line(q: &Quote) -> String {
    let mut out = format!("📈 {}：{}", q.name, q.price);

    if let (Some(p), Some(prev)) = (q.price_num, q.prev) {
        if let Some(suffix) = change_suffix(p, prev) {
            out.push(' ');
            out.push_str(&suffix);
        }
    }

    let detail = match (&q.high, &q.low) {
        (Some(h), Some(l)) => Some(format!("高 {h} / 低 {l}")),
        _ => None,
    };
    match (detail, &q.stamp) {
        (Some(d), Some(s)) => out.push_str(&format!("\n   {d}，{s}")),
        (Some(d), None) => out.push_str(&format!("\n   {d}")),
        (None, Some(s)) => out.push_str(&format!("\n   {s}")),
        (None, None) => {}
    }
    out
}
