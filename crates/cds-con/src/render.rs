//! Credit-spread message rendering.
//!
//! **No classification.** No `Status` enum, no `狀態：` line, no summary
//! adjective. The job is to lay out levels, percentiles and coverage honestly.
//! Spreads and yields are not commensurable — they render in separate blocks.

use credit_store::{
    baa_aaa_spread, below_and_total, window_counts, Observation, SeriesKind, SeriesSpec,
    WindowCounts,
};

/// Truncated (never rounded) tenths-of-a-percent share of `below` within `n`,
/// e.g. `(2, 3)` -> `"66.6"`, never `"66.7"`. Integer arithmetic on the exact
/// values that are also printed as the count, so the share can never disagree
/// with `{below} 筆比現在低` on the same line -- there is no separate
/// percentage computed from a different source.
fn truncated_pct_str(below: usize, n: usize) -> String {
    if n == 0 {
        return "0.0".into();
    }
    let tenths = (below as u64 * 1000) / n as u64;
    format!("{}.{}", tenths / 10, tenths % 10)
}

/// Publication frequency shown on each series line and on the freshness line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frequency {
    Daily,
    Monthly,
}

impl Frequency {
    pub fn as_str(self) -> &'static str {
        match self {
            Frequency::Daily => "daily",
            Frequency::Monthly => "monthly",
        }
    }
}

/// One configured series plus its stored observations (empty = missing).
#[derive(Debug, Clone)]
pub struct SeriesInput {
    pub spec: SeriesSpec,
    pub rows: Vec<Observation>,
    pub frequency: Frequency,
}

/// A trailing-window count that the series can actually support.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowPct {
    /// Display label: `近1年`, `近10年`, or the coverage start year (`自1986`).
    pub label: String,
    /// Observations in the window strictly below the latest value.
    pub below: usize,
    /// Observations in the window.
    pub n: usize,
}

/// Fully computed series row, ready to format. Tests that pin layout inject
/// these directly; production goes through [`analyze`] then [`render_lines`].
#[derive(Debug, Clone)]
pub struct SeriesLine {
    pub key: String,
    /// Human-facing name. Comes from the `cds_series` config's Label field --
    /// it is data, not code, so translating the message never touches Rust.
    /// The derived quality spread is the one exception: it is computed here, so
    /// its name is a constant here too.
    pub label: String,
    pub kind: SeriesKind,
    pub value: Option<f64>,
    pub windows: Vec<WindowPct>,
    pub coverage_start: Option<String>,
    pub latest: Option<String>,
    pub frequency: Frequency,
    /// Position in the configured series list; used only as a stable tie-break
    /// when two series share a coverage start. Derived rows use a sentinel.
    pub config_order: usize,
}

/// Loud failure when the family split cannot be rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderError {
    pub message: String,
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for RenderError {}

/// Whether a rendered line is prose (headers, freshness, footer -- allowed to
/// wrap on a phone) or a per-series data row (must fit [`WIDTH_BOUND`]).
///
/// This is a STRUCTURAL tag the renderer attaches to every line it emits, not
/// a guess reconstructed from the line's text. Series labels come from the
/// DB-backed `cds_series` config, so a future label that happens to start
/// with `利差` or `殖利率` must never silently exempt a real data row from the
/// width-bound guard -- which it would if the guard matched on text prefixes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Prose,
    Data,
}

/// One line of the rendered message, tagged with how the renderer built it.
/// `render_parts` is the structural seam: tests consult `kind`, never the
/// text, to decide whether a line is subject to the width bound.
#[derive(Debug, Clone)]
pub struct Segment {
    pub text: String,
    pub kind: LineKind,
}

fn prose(text: impl Into<String>) -> Segment {
    Segment { text: text.into(), kind: LineKind::Prose }
}

fn data(text: impl Into<String>) -> Segment {
    Segment { text: text.into(), kind: LineKind::Data }
}

fn blank() -> Segment {
    prose(String::new())
}

/// Width bound the renderer *enforces* on every per-series title line (see
/// [`series_block`]/[`display_width`]), and the gate
/// `every_rendered_line_fits_its_width_bound` in `tests/render.rs` checks the
/// rest of a series' block against via [`width_bound`]. The transport
/// (`parse_mode: None`) renders a proportional font, so this CJK-is-2-columns
/// model is a coarse proxy, not a guarantee against wrapping on a real phone
/// -- but as of 2026-08 it is an active guarantee for the title line, not a
/// hope that the `cds_series` config's label stays short: a title line that
/// would exceed this bound splits (label alone, then `  value   [key]`
/// indented like a window row) instead of quietly growing past it.
///
/// The one line this cannot guarantee: a label that alone exceeds the bound
/// still overflows on its own line. The renderer never truncates a
/// configured label to force a fit -- discarding real `cds_series` data
/// would be worse than one wrapped line. See
/// `overlong_ascii_label_splits_the_title_line` /
/// `overlong_cjk_label_splits_the_title_line` in `tests/render.rs`.
///
/// Raised from 40 to 48 for v3: the value moved onto the title line (v3 §3),
/// and the widest title measured against the live 2026-07-30 data --
/// `Baa 比 10年期美債多出的殖利率  1.63%   [baa10y]` -- is 47 display
/// columns under the CJK-is-2 model below.
const WIDTH_BOUND: usize = 48;

/// Test seam for [`WIDTH_BOUND`]. Kept private otherwise -- nothing else in
/// production rendering needs the raw constant once [`series_block`] does its
/// own comparison.
pub fn width_bound() -> usize {
    WIDTH_BOUND
}

/// Display-column width of `s` under the CJK-is-2 model [`WIDTH_BOUND`]
/// assumes: CJK ideographs, Hangul syllables and fullwidth forms count as 2,
/// everything else as 1. This is the one place production rendering does
/// column math -- [`series_block`] uses it to decide whether a title line
/// must split, which is what turns the width bound from a hope about
/// `cds_series.label` staying short into something the renderer actually
/// enforces. Mirrors (as an independent copy, not a shared import) the model
/// `tests/render.rs` checks output against.
fn display_width(s: &str) -> usize {
    s.chars()
        .map(|c| {
            let c = c as u32;
            let wide = (0x1100..=0x115F).contains(&c)
                || (0x2E80..=0xA4CF).contains(&c)
                || (0xAC00..=0xD7A3).contains(&c)
                || (0xF900..=0xFAFF).contains(&c)
                || (0xFE30..=0xFE6F).contains(&c)
                || (0xFF00..=0xFF60).contains(&c)
                || (0xFFE0..=0xFFE6).contains(&c);
            if wide {
                2
            } else {
                1
            }
        })
        .sum()
}

/// Display key for the derived quality spread (Unicode minus U+2212).
pub const BAA_AAA_KEY: &str = "baa−aaa";

/// Name for the derived quality spread. Unlike the configured series, whose
/// labels come from `cds_series`, this row is computed here and so is named
/// here -- there is no config entry to carry it.
pub const BAA_AAA_LABEL: &str = "Baa 比 Aaa 多出的殖利率";

/// Derived-series config_order: not in `cds_series`, so it cannot tie-break by
/// config position. Sort by coverage only; this sentinel is never compared when
/// coverage starts differ (the normal case).
const DERIVED_ORDER: usize = usize::MAX / 2;

/// Analyze configured series: require kind, derive baa−aaa, compute windows,
/// sort each family by longest coverage first (earliest start), config order
/// on ties.
pub fn analyze(series: &[SeriesInput]) -> Result<Vec<SeriesLine>, RenderError> {
    // Missing kind fails loudly — never a guessed default.
    for s in series {
        s.spec.require_kind().map_err(|message| RenderError { message })?;
    }

    let mut lines: Vec<SeriesLine> = Vec::with_capacity(series.len() + 1);

    for (i, s) in series.iter().enumerate() {
        let kind = s.spec.kind.expect("require_kind checked above");
        lines.push(series_line_from_rows(
            s.spec.key.clone(),
            s.spec.label.clone(),
            kind,
            &s.rows,
            s.frequency,
            i,
        ));
    }

    // Derive baa−aaa when both inputs have data. Inner join may leave it older
    // than either input; that lag is visible on its own latest date.
    let baa = series.iter().find(|s| s.spec.key == "baa");
    let aaa = series.iter().find(|s| s.spec.key == "aaa");
    if let (Some(baa), Some(aaa)) = (baa, aaa) {
        if !baa.rows.is_empty() && !aaa.rows.is_empty() {
            let derived = baa_aaa_spread(&baa.rows, &aaa.rows);
            if !derived.is_empty() {
                lines.push(series_line_from_rows(
                    BAA_AAA_KEY.to_string(),
                    BAA_AAA_LABEL.to_string(),
                    SeriesKind::Spread,
                    &derived,
                    Frequency::Monthly,
                    DERIVED_ORDER,
                ));
            }
        }
    }

    // Sort within each kind: earliest coverage first (longest history leads);
    // missing coverage sorts last; ties by config_order.
    lines.sort_by(|a, b| {
        let kind_ord = kind_block_order(a.kind).cmp(&kind_block_order(b.kind));
        if kind_ord != std::cmp::Ordering::Equal {
            return kind_ord;
        }
        match (&a.coverage_start, &b.coverage_start) {
            (Some(ca), Some(cb)) => ca.cmp(cb).then(a.config_order.cmp(&b.config_order)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.config_order.cmp(&b.config_order),
        }
    });

    Ok(lines)
}

fn kind_block_order(kind: SeriesKind) -> u8 {
    match kind {
        SeriesKind::Spread => 0,
        SeriesKind::Yield => 1,
    }
}

fn series_line_from_rows(
    key: String,
    label: String,
    kind: SeriesKind,
    rows: &[Observation],
    frequency: Frequency,
    config_order: usize,
) -> SeriesLine {
    if rows.is_empty() {
        return SeriesLine {
            key,
            label,
            kind,
            value: None,
            windows: vec![],
            coverage_start: None,
            latest: None,
            frequency,
            config_order,
        };
    }

    let last = rows.last().unwrap();
    let mut windows = Vec::new();

    // 近1年, 近10年: omit unreachable windows — never print `insufficient-coverage`.
    for (years, label) in [(1u32, "近1年"), (10u32, "近10年")] {
        if let WindowCounts::Computed { below, n } = window_counts(rows, years) {
            windows.push(WindowPct { label: label.to_string(), below, n });
        }
    }

    // Full stored history (always available when rows are non-empty), labeled
    // by its coverage start year rather than a fixed placeholder word, so a
    // 107-year window and a 3-year window read as two visibly different
    // rulers instead of the same word.
    let vals: Vec<f64> = rows.iter().map(|r| r.value).collect();
    let (below, n) = below_and_total(&vals, last.value);
    windows.push(WindowPct {
        label: format!("自{}", year_str(&rows[0].date)),
        below,
        n,
    });

    SeriesLine {
        key,
        label,
        kind,
        value: Some(last.value),
        windows,
        coverage_start: Some(rows[0].date.clone()),
        latest: Some(last.date.clone()),
        frequency,
        config_order,
    }
}

/// Render precomputed lines into the daily message body as structurally
/// tagged segments -- the seam [`render_lines`] flattens and tests can
/// consult directly (see `render_parts`/`Segment` doc).
///
/// `lines` is already the exact, ordered set the message shows -- selection
/// against `cds_message_series` happens in [`select_message_series`], before
/// this function ever runs. There is no daily/monthly split here: v2 built
/// one to fit a 58-line message; v3 is 28 lines and the mechanism would have
/// hidden the yield-vs-spread contrast the message exists to show on the
/// ~29 days a month it stayed collapsed.
///
/// `as_of` is an injected YYYY-MM-DD used only for age-in-days on the
/// freshness line. No clock.
pub fn render_parts(lines: &[SeriesLine], as_of: &str) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    match header_date(lines) {
        Some(d) => out.push(prose(format!("💾 信用利差 · {d}"))),
        None => out.push(prose("💾 信用利差")),
    }
    out.push(blank());

    let spreads: Vec<&SeriesLine> = lines.iter().filter(|l| l.kind == SeriesKind::Spread).collect();
    let yields: Vec<&SeriesLine> = lines.iter().filter(|l| l.kind == SeriesKind::Yield).collect();

    out.push(prose("利差 —— 相對某個基準多出的殖利率"));
    out.push(prose("已扣掉利率,動的是市場對「借錢給公司」的要價"));
    out.push(blank());
    push_blocks(&mut out, &spreads);

    out.push(blank());
    out.push(prose(
        "總殖利率 —— 含利率在內的全部借款成本,與上一區不可互比",
    ));
    out.push(prose(
        "留這一條當對照:同一批 Baa 債,上面那條扣掉了利率,這條沒扣",
    ));
    out.push(blank());
    push_blocks(&mut out, &yields);

    out.push(blank());
    out.push(prose(format_freshness_line(lines, as_of)));
    // Fixed prose, never a computed contrast between today's specific
    // rulers -- v2's dynamic footer sentence is gone along with the
    // daily/monthly split it depended on to know which rulers were on
    // screen. No verdict, no adjective about level: only what a window is
    // for.
    out.push(prose(
        "SIGNAL-ONLY:窗口越短對當下越敏感,越長越穩定。它們回答不同的問題,不可跨列比。",
    ));

    out
}

/// Flatten [`render_parts`] into the plain-text message body actually sent.
pub fn render_lines(lines: &[SeriesLine], as_of: &str) -> String {
    render_parts(lines, as_of)
        .into_iter()
        .map(|s| s.text)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Date shown on the header line: the most recent daily observation among
/// the rendered series, falling back to the most recent monthly one if no
/// daily series is present. This is a fact about the data ("what date do
/// these numbers reflect"), not the run date -- `as_of` (today) can be days
/// ahead of it, which is exactly what the freshness line's age-in-days
/// already states.
fn header_date(lines: &[SeriesLine]) -> Option<String> {
    min_latest(lines, Frequency::Daily).or_else(|| min_latest(lines, Frequency::Monthly))
}

/// Push each series' block, with a blank line separating consecutive
/// blocks -- none before the first, so it sits directly under the header.
/// Every line a block contributes is tagged `Data`: a series' title line
/// (`Label  value   [key]`) wraps just as badly as its window lines if it
/// bloats back toward the old table's column width.
fn push_blocks(out: &mut Vec<Segment>, series: &[&SeriesLine]) {
    for (i, line) in series.iter().enumerate() {
        if i > 0 {
            out.push(blank());
        }
        out.extend(series_block(line).into_iter().map(data));
    }
}

/// One series as a block of lines: title (label, value, and key together),
/// then one line per trailing window. No padding across series --
/// `parse_mode: None` means Telegram renders this in a proportional font, so
/// column alignment across rows was never reaching the reader. See the
/// 2026-08-04 design note.
///
/// Title is `Label  value   [key]` (v3 §3: the value moved here from its own
/// line, since the owner could not read the count-only v2 message without
/// seeing the size of the number). The reader and the operator are the same
/// person: the key is what he types into `price cds show <key>` and what he
/// edits in `cds_series`, so dropping it from the daily message would force a
/// lookup. The FRED series id stays out -- it is longer and cannot be passed
/// to anything.
///
/// The combined title line comes from the `cds_series` config's label, which
/// no Rust test can see -- so this bound was previously a hope, not a
/// guarantee (a two-character label edit was enough to silently overflow
/// [`WIDTH_BOUND`] with every test still green). When the combined line would
/// exceed the bound, it splits: the label alone on its own line, then
/// `  value   [key]` indented like a window row on the next -- the same shape
/// as today's single-line form when nothing needs to split, so short config
/// renders byte-identical to before. See [`display_width`] and the
/// `overlong_*_label_splits_the_title_line` tests in `tests/render.rs`.
fn series_block(line: &SeriesLine) -> Vec<String> {
    let combined = format!("{}  {}   [{}]", line.label, value_str(line), line.key);
    let mut out = if display_width(&combined) <= WIDTH_BOUND {
        vec![combined]
    } else {
        vec![
            line.label.clone(),
            format!("  {}   [{}]", value_str(line), line.key),
        ]
    };
    out.extend(window_lines(line));
    out
}

/// One line per trailing window the series can support, total named before
/// the part (v3 §4: `{n} 筆裡 {below} 筆比現在低`, not `{below}/{n}`), with
/// the share alongside in parentheses (v3 §3, reversing v2 §2 -- the owner
/// could not read a bare count). The share is truncated from the very
/// `below`/`n` printed on the same line via [`truncated_pct_str`], never
/// computed separately, so the two can never disagree.
fn window_lines(line: &SeriesLine) -> Vec<String> {
    line.windows
        .iter()
        .map(|w| {
            format!(
                "  {} {} 筆裡 {} 筆比現在低({}%)",
                w.label,
                w.n,
                w.below,
                truncated_pct_str(w.below, w.n)
            )
        })
        .collect()
}

/// Analyze, select the configured message series, then render.
///
/// `message_keys` is `cds_message_series` (v3 §5), parsed by
/// [`parse_message_series`] and resolved by [`select_message_series`] --
/// filtered AFTER `analyze()`, never before, so the derived `baa−aaa` row (only
/// built once `baa`/`aaa` have both been analyzed) can still be named in it.
pub fn format_message(
    series: &[SeriesInput],
    as_of: &str,
    message_keys: &[String],
) -> Result<String, RenderError> {
    let lines = analyze(series)?;
    let shown = select_message_series(&lines, message_keys)?;
    Ok(render_lines(&shown, as_of))
}

/// Parse `cds_message_series`: a comma-separated list of series keys in
/// display order (v3 §5). Every token must be non-empty -- a blank value, a
/// leading/trailing/doubled comma, is unparseable and fails the run loudly,
/// the same standard `cds_series` and a missing `kind` already hold. This
/// function does not check the keys against any known series; that check
/// happens in [`select_message_series`], which is the only place that knows
/// what series exist.
pub fn parse_message_series(raw: &str) -> Result<Vec<String>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(
            "cds_message_series is empty; expected a comma-separated list of series keys".into(),
        );
    }
    let mut out = Vec::new();
    for (i, tok) in trimmed.split(',').enumerate() {
        let key = tok.trim();
        if key.is_empty() {
            return Err(format!(
                "cds_message_series has an empty key at position {} in '{trimmed}'",
                i + 1
            ));
        }
        out.push(key.to_string());
    }
    Ok(out)
}

/// Select and order the series the message actually shows, from
/// `cds_message_series`'s parsed key list. Every key must name a series
/// present in `lines` -- an unknown key fails loudly, by name, rather than
/// being silently dropped (v3 §5). Order follows `keys`, not `analyze`'s
/// coverage-first sort: the config *is* the display order.
///
/// Must run AFTER [`analyze`]: the derived `baa−aaa` row only exists in its
/// output, built from the `baa`/`aaa` inputs, so selecting against the raw
/// `SeriesInput`s first would mean it is never derived at all.
pub fn select_message_series(
    lines: &[SeriesLine],
    keys: &[String],
) -> Result<Vec<SeriesLine>, RenderError> {
    let mut out = Vec::with_capacity(keys.len());
    for key in keys {
        match lines.iter().find(|l| &l.key == key) {
            Some(line) => out.push(line.clone()),
            None => {
                let available: Vec<&str> = lines.iter().map(|l| l.key.as_str()).collect();
                return Err(RenderError {
                    message: format!(
                        "cds_message_series names unknown series '{key}'; known series are: {}",
                        available.join(", ")
                    ),
                });
            }
        }
    }
    Ok(out)
}

/// Values carry `%`. Every configured FRED series is denominated in percent, and
/// the unit used to ride along inside the English label text (`... pct`); short
/// Chinese labels dropped it, leaving the number bare. OAS in particular is often
/// quoted in basis points, so a unitless 2.84 is genuinely ambiguous.
fn value_str(line: &SeriesLine) -> String {
    match line.value {
        Some(v) => format!("{v:.2}%"),
        None => "n/a".into(),
    }
}

/// The 4-char year prefix of a `YYYY-MM-DD` date. The day and month carry no
/// meaning for the reader; the year is what makes a 1-year window on a
/// 3-year history read differently from one on 107 years. Used to build the
/// full-history window's `自{year}` label in [`series_line_from_rows`].
fn year_str(date: &str) -> &str {
    &date[..4.min(date.len())]
}

fn format_freshness_line(lines: &[SeriesLine], as_of: &str) -> String {
    let daily_latest = min_latest(lines, Frequency::Daily);
    let monthly_latest = min_latest(lines, Frequency::Monthly);

    let mut parts: Vec<String> = Vec::new();

    if let Some(d) = daily_latest {
        let age = days_between(&d, as_of).unwrap_or(0);
        parts.push(format!("日 至 {d}({age} 天前)"));
    }
    if let Some(m) = monthly_latest {
        // Monthly: state the date only — no age parenthetical in the golden.
        parts.push(format!("月 至 {}", &m[..7.min(m.len())]));
    }

    let missing: Vec<&str> = lines
        .iter()
        .filter(|l| l.value.is_none())
        .map(|l| l.key.as_str())
        .collect();
    if !missing.is_empty() {
        parts.push(format!("缺 {}", missing.join(",")));
    }

    format!("資料:{}", parts.join("・"))
}

/// Minimum latest date across series of the given frequency.
/// Using the minimum (not the maximum) is load-bearing for monthly: the
/// derived baa−aaa can lag both inputs after an inner-join drop.
fn min_latest(lines: &[SeriesLine], freq: Frequency) -> Option<String> {
    lines
        .iter()
        .filter(|l| l.frequency == freq)
        .filter_map(|l| l.latest.as_ref())
        .min()
        .cloned()
}

/// Whole days from `earlier` to `later` (YYYY-MM-DD). No clock, no timezone.
fn days_between(earlier: &str, later: &str) -> Option<i64> {
    let (y1, m1, d1) = parse_ymd(earlier)?;
    let (y2, m2, d2) = parse_ymd(later)?;
    Some(date_to_rata_die(y2, m2, d2) - date_to_rata_die(y1, m1, d1))
}

fn parse_ymd(s: &str) -> Option<(i32, u32, u32)> {
    if s.len() < 10 {
        return None;
    }
    let y: i32 = s[0..4].parse().ok()?;
    let m: u32 = s[5..7].parse().ok()?;
    let d: u32 = s[8..10].parse().ok()?;
    Some((y, m, d))
}

/// Days since 0001-01-01 (proleptic Gregorian), after Hinnant.
fn date_to_rata_die(y: i32, m: u32, d: u32) -> i64 {
    let y = y as i64;
    let m = m as i64;
    let d = d as i64;
    let (y, m) = if m <= 2 {
        (y - 1, m + 9)
    } else {
        (y, m - 3)
    };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * m + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 306
}
