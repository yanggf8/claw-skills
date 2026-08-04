//! Credit-spread message rendering.
//!
//! **No classification.** No `Status` enum, no `狀態：` line, no summary
//! adjective. The job is to lay out levels, percentiles and coverage honestly.
//! Spreads and yields are not commensurable — they render in separate blocks.

use credit_store::{
    baa_aaa_spread, below_and_total, window_counts, Observation, SeriesKind, SeriesSpec,
    WindowCounts,
};

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

/// Width bound used only as a render-quality gate in tests -- see
/// [`width_bound`] and `every_rendered_line_fits_its_width_bound` in
/// `tests/render.rs`. Production rendering does no column math anymore: the
/// transport (`parse_mode: None`) renders a proportional font, so this bound
/// is a coarse proxy against a line bloating back to a size that breaks even
/// a monospace reader -- it is not a guarantee against wrapping on a phone.
const WIDTH_BOUND: usize = 40;

/// Test seam for [`WIDTH_BOUND`]. Kept private otherwise -- nothing in
/// production rendering needs it once column alignment is gone.
pub fn width_bound() -> usize {
    WIDTH_BOUND
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

/// Day-of-month from an injected `YYYY-MM-DD`. No clock: `main.rs` supplies a
/// CST calendar date, so keying the bound on `as_of` is CST by construction.
fn day_of_month(as_of: &str) -> u32 {
    as_of.get(8..10).and_then(|d| d.parse().ok()).unwrap_or(1)
}

/// Monthly series change once a month; on the other ~29 days they do not earn
/// a third of the message. The split is by publication frequency, never by
/// value, so the rule is identical whichever way the market moves.
fn expand_monthly(as_of: &str, expand_days: u32) -> bool {
    day_of_month(as_of) <= expand_days
}

/// Render precomputed lines into the daily message body as structurally
/// tagged segments -- the seam [`render_lines`] flattens and tests can
/// consult directly (see `render_parts`/`Segment` doc).
///
/// `as_of` is an injected YYYY-MM-DD used only for age-in-days and for the
/// daily/monthly expand decision. No clock. `expand_days` is the configured
/// day-of-month bound (`cds_monthly_expand_days`), read by the caller from
/// the registry `config` table -- never hardcoded here.
pub fn render_parts(lines: &[SeriesLine], as_of: &str, expand_days: u32) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    out.push(prose("💾 信用利差"));
    out.push(blank());

    let expand = expand_monthly(as_of, expand_days);
    // Filtered AFTER analyze() only -- `lines` already carries the derived
    // baa−aaa row built from the `baa`/`aaa` inputs; filtering any earlier
    // would stop it being derived at all.
    let shown: Vec<&SeriesLine> = lines
        .iter()
        .filter(|l| expand || l.frequency == Frequency::Daily)
        .collect();

    let spreads: Vec<&SeriesLine> = shown
        .iter()
        .copied()
        .filter(|l| l.kind == SeriesKind::Spread)
        .collect();
    let yields: Vec<&SeriesLine> = shown
        .iter()
        .copied()
        .filter(|l| l.kind == SeriesKind::Yield)
        .collect();

    out.push(prose("利差 —— 相對某個基準多出的殖利率"));
    out.push(blank());
    push_blocks(&mut out, &spreads);

    out.push(blank());
    out.push(prose(
        "總殖利率 —— 含無風險利率在內的全部借款成本(與上一區不可互比)",
    ));
    out.push(blank());
    push_blocks(&mut out, &yields);

    out.push(blank());
    // Freshness is drawn from what is actually shown today -- a collapsed
    // monthly series must not advertise a date that is not on screen.
    let shown_owned: Vec<SeriesLine> = shown.iter().map(|&l| l.clone()).collect();
    out.push(prose(format_freshness_line(&shown_owned, as_of)));
    // The FULL `lines` (not `shown`) so a missing monthly series is still
    // named on the ~29 days a month the block is collapsed -- filtering the
    // monthly rows out before this check would hide it.
    if let Some(s) = monthly_status_line(lines, expand_days).filter(|_| !expand) {
        out.push(prose(s));
    }
    // The SIGNAL-ONLY marker stays: it is a project-wide boundary marker, not
    // prose. Only the explanation after it became concrete.
    out.push(prose(
        "SIGNAL-ONLY:每個窗口各自回答自己的問題,不可跨列比較——",
    ));
    if let Some(c) = footer_contrast(&shown) {
        out.push(prose(c));
    }

    out
}

/// Flatten [`render_parts`] into the plain-text message body actually sent.
pub fn render_lines(lines: &[SeriesLine], as_of: &str, expand_days: u32) -> String {
    render_parts(lines, as_of, expand_days)
        .into_iter()
        .map(|s| s.text)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Collapsed monthly summary. Carries the month reached AND any missing
/// monthly series -- without the latter, filtering the rows out would hide a
/// missing series for the ~29 days the block is collapsed.
fn monthly_status_line(lines: &[SeriesLine], expand_days: u32) -> Option<String> {
    let monthly: Vec<&SeriesLine> = lines
        .iter()
        .filter(|l| l.frequency == Frequency::Monthly)
        .collect();
    if monthly.is_empty() {
        return None;
    }
    let reached = monthly
        .iter()
        .filter_map(|l| l.latest.as_ref())
        .min()
        .map(|d| d[..7.min(d.len())].to_string())
        .unwrap_or_else(|| "—".into());
    let mut s = format!(
        "月頻 {} 列 資料至 {},未展開(每月 1–{} 日展開)",
        monthly.len(),
        reached,
        expand_days
    );
    let missing: Vec<&str> = monthly
        .iter()
        .filter(|l| l.value.is_none())
        .map(|l| l.key.as_str())
        .collect();
    if !missing.is_empty() {
        s.push_str(&format!("・缺 {}", missing.join(",")));
    }
    Some(s)
}

/// Two rulers that are actually on screen today. Naming a collapsed series'
/// start year would point the reader at something not in the message.
fn footer_contrast(shown: &[&SeriesLine]) -> Option<String> {
    let mut with_cov: Vec<&&SeriesLine> = shown
        .iter()
        .filter(|l| l.coverage_start.is_some() && !l.windows.is_empty())
        .collect();
    with_cov.sort_by_key(|l| l.coverage_start.clone());
    let (first, last) = (with_cov.first()?, with_cov.last()?);
    if coverage_year(first) == coverage_year(last) {
        return None;
    }
    let n = |l: &SeriesLine| l.windows.last().map(|w| w.n).unwrap_or(0);
    Some(format!(
        "自{} 的 {} 筆和自{} 的 {} 筆不是同一把尺。",
        coverage_year(last)?,
        n(last),
        coverage_year(first)?,
        n(first)
    ))
}

/// Push each series' block, with a blank line separating consecutive
/// blocks -- none before the first, so it sits directly under the header.
/// Every line a block contributes is tagged `Data`: a series' title line
/// (`Label [key]`) wraps just as badly as its window lines if it bloats back
/// toward the old table's column width.
fn push_blocks(out: &mut Vec<Segment>, series: &[&SeriesLine]) {
    for (i, line) in series.iter().enumerate() {
        if i > 0 {
            out.push(blank());
        }
        out.extend(series_block(line).into_iter().map(data));
    }
}

/// One series as a block of lines: title, then value + coverage, then one
/// line per trailing window. No padding across series -- `parse_mode: None`
/// means Telegram renders this in a proportional font, so column alignment
/// across rows was never reaching the reader. See the 2026-08-04 design note.
///
/// Title is `Label [key]`. The reader and the operator are the same person:
/// the key is what he types into `price cds show <key>` and what he edits in
/// `cds_series`, so dropping it from the daily message would force a lookup.
/// The FRED series id stays out -- it is longer and cannot be passed to
/// anything.
fn series_block(line: &SeriesLine) -> Vec<String> {
    let mut out = vec![format!("{} [{}]", line.label, line.key)];
    out.push(format!("  {}  {}", value_str(line), coverage_str(line)));
    out.extend(window_lines(line));
    out
}

/// A trailing-window count that the series can actually support. A percentile
/// is not printed; the count is the definition, so the reader never has to
/// know what `p` meant.
fn window_lines(line: &SeriesLine) -> Vec<String> {
    line.windows
        .iter()
        .map(|w| format!("  {}  {}/{} 筆低於本次", w.label, w.below, w.n))
        .collect()
}

/// Analyze then render. Entry point for the skill once Task 3 wires the store.
///
/// `expand_days` gates the monthly block (see [`expand_monthly`]) and must
/// come from the caller's config read, never a literal here.
pub fn format_message(
    series: &[SeriesInput],
    as_of: &str,
    expand_days: u32,
) -> Result<String, RenderError> {
    let lines = analyze(series)?;
    Ok(render_lines(&lines, as_of, expand_days))
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
/// 3-year history read differently from one on 107 years. Shared by
/// [`coverage_year`] and the full-history window label in
/// [`series_line_from_rows`] so the slice bound lives in exactly one place.
fn year_str(date: &str) -> &str {
    &date[..4.min(date.len())]
}

/// Coverage start year, the only part of a start date that carries meaning:
/// it is what makes a 1-year window over a 3-year history read differently
/// from one over 107 years.
fn coverage_year(line: &SeriesLine) -> Option<&str> {
    line.coverage_start.as_deref().map(year_str)
}

/// `日頻・自1986` rather than `1986-01-02→ daily`: frequency leads because it
/// is the axis that changes between blocks (daily vs monthly), while within
/// a block the day and month of a coverage start carry no meaning for the
/// reader -- the year is what makes a 1-year percentile on a 3-year history
/// read differently from one on 107 years.
fn coverage_str(line: &SeriesLine) -> String {
    let freq = match line.frequency {
        Frequency::Daily => "日頻",
        Frequency::Monthly => "月頻",
    };
    match coverage_year(line) {
        Some(y) => format!("{freq}・自{y}"),
        None => format!("{freq}・自—"),
    }
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

    format!("資料:{}", parts.join(" · "))
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
