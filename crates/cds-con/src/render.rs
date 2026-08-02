//! Credit-spread message rendering.
//!
//! **No classification.** No `Status` enum, no `狀態：` line, no summary
//! adjective. The job is to lay out levels, percentiles and coverage honestly.
//! Spreads and yields are not commensurable — they render in separate blocks.

use credit_store::{
    baa_aaa_spread, percentile_rank, window_stat, Observation, SeriesKind, SeriesSpec, WindowStat,
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

/// A trailing-window percentile that the series can actually support.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowPct {
    /// Display label: `1年`, `10年`, or `全庫`.
    pub label: &'static str,
    pub pctile: f64,
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


/// Display width: CJK / fullwidth characters occupy two columns.
///
/// Column padding cannot use `{:<N}`, which counts chars. A label like
/// 「品質利差」 is 4 chars but 8 columns, so char-based padding collapses every
/// column to its right the moment a label stops being ASCII.
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
                || (0xFFE0..=0xFFE6).contains(&c)
                || (0x20000..=0x3FFFD).contains(&c);
            if wide { 2 } else { 1 }
        })
        .sum()
}

/// Pad `s` on the right to `width` display columns.
fn pad_to(s: &str, width: usize) -> String {
    let mut out = s.to_string();
    for _ in display_width(s)..width {
        out.push(' ');
    }
    out
}

/// A percentile is a rank, so it shows as a whole number -- `p60.0` implied a
/// precision the rank does not have.
///
/// Truncated, NOT rounded. Rounding turns 99.6 into `p100`, which asserts that
/// nothing in the window sits above this value while 0.4% of it does. `p99`
/// understates by less than one percentile and stays true: at least 99% of the
/// window is below. The display may never claim a higher rank than the data
/// supports. The stored value keeps full precision.
fn fmt_pct(p: f64) -> String {
    format!("p{}", p.floor() as i64)
}

/// Display key for the derived quality spread (Unicode minus U+2212).
pub const BAA_AAA_KEY: &str = "baa−aaa";

/// Name for the derived quality spread. Unlike the configured series, whose
/// labels come from `cds_series`, this row is computed here and so is named
/// here -- there is no config entry to carry it.
pub const BAA_AAA_LABEL: &str = "品質利差 Baa−Aaa";

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

    // 1y, 10y: omit unreachable windows — never print `insufficient-coverage`.
    for (years, label) in [(1u32, "1年"), (10u32, "10年")] {
        match window_stat(rows, years) {
            WindowStat::Computed { pctile, .. } => {
                windows.push(WindowPct { label, pctile });
            }
            WindowStat::Insufficient { .. } => {}
        }
    }

    // 全庫: full stored history (always available when rows are non-empty).
    let vals: Vec<f64> = rows.iter().map(|r| r.value).collect();
    let all_pct = percentile_rank(&vals, last.value);
    windows.push(WindowPct {
        label: "全庫",
        pctile: all_pct,
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

/// Render precomputed lines into the daily message body.
///
/// `as_of` is an injected YYYY-MM-DD used only for age-in-days. No clock.
pub fn render_lines(lines: &[SeriesLine], as_of: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    out.push("💾 信用利差".into());
    out.push(String::new());

    let spreads: Vec<&SeriesLine> = lines
        .iter()
        .filter(|l| l.kind == SeriesKind::Spread)
        .collect();
    let yields: Vec<&SeriesLine> = lines
        .iter()
        .filter(|l| l.kind == SeriesKind::Yield)
        .collect();

    // One width set across BOTH blocks, so the two families still line up as a
    // single table even though they must not be compared.
    let all: Vec<SeriesLine> = lines.to_vec();
    let w = row_widths(&all);

    out.push("利差(已扣掉無風險利率 —— 這是信用風險本身的價格)".into());
    for line in &spreads {
        out.push(format_series_row(line, &w));
    }

    out.push(String::new());
    out.push("殖利率(含無風險利率 —— 高低多半是利率在動,不是信用在動)".into());
    for line in &yields {
        out.push(format_series_row(line, &w));
    }

    out.push(String::new());
    out.push(format_freshness_line(lines, as_of));
    // The SIGNAL-ONLY marker stays: it is a project-wide boundary marker, not
    // prose. Only the explanation after it became concrete.
    out.push("SIGNAL-ONLY:百分位 = 在那個窗口裡排第幾,換一把尺就換一個答案。".into());
    if let Some(example) = window_example(lines) {
        out.push(example);
    }

    out.join("\n")
}

/// Show window-dependence with the message's OWN numbers instead of stating it
/// abstractly. An abstract footer gets skipped; a worked example from today's
/// data does not, and it stays a demonstration rather than becoming a verdict.
///
/// Picks the first line carrying at least two windows whose percentiles actually
/// differ -- an example where both windows agree would demonstrate nothing.
/// Emits nothing when no such line exists, rather than inventing one.
fn window_example(lines: &[SeriesLine]) -> Option<String> {
    for l in lines {
        if l.windows.len() < 2 {
            continue;
        }
        let (a, b) = (&l.windows[0], &l.windows[1]);
        if fmt_pct(a.pctile) == fmt_pct(b.pctile) {
            continue;
        }
        return Some(format!(
            "例:{} {} —— {} 排 {},{} 排 {}。不是兩個市場,是兩把尺。",
            l.label,
            value_str(l),
            a.label,
            fmt_pct(a.pctile),
            b.label,
            fmt_pct(b.pctile),
        ));
    }
    None
}

/// Analyze then render. Entry point for the skill once Task 3 wires the store.
pub fn format_message(series: &[SeriesInput], as_of: &str) -> Result<String, RenderError> {
    let lines = analyze(series)?;
    Ok(render_lines(&lines, as_of))
}

/// Column widths are measured across the rows being rendered rather than fixed,
/// because a label's width is now data (config) and a fixed number would be a
/// guess about someone else's config.
struct RowWidths {
    label: usize,
    value: usize,
    windows: usize,
}

/// `Label [key]`. The reader and the operator are the same person: the key is
/// what he types into `price cds show <key>` and what he edits in `cds_series`,
/// so dropping it from the daily message would force a lookup. The FRED series id
/// stays out — it is longer and cannot be passed to anything.
fn label_str(line: &SeriesLine) -> String {
    format!("{} [{}]", line.label, line.key)
}

fn row_widths(lines: &[SeriesLine]) -> RowWidths {
    let mut w = RowWidths { label: 0, value: 0, windows: 0 };
    for l in lines {
        w.label = w.label.max(display_width(&label_str(l)));
        w.value = w.value.max(value_str(l).len());
        w.windows = w.windows.max(display_width(&windows_str(l)));
    }
    w
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

fn windows_str(line: &SeriesLine) -> String {
    line.windows
        .iter()
        .map(|w| format!("{} {}", w.label, fmt_pct(w.pctile)))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// `自1986 日` rather than `1986-01-02→ daily`: the day and month of a coverage
/// start carry no meaning for the reader, while the year is what makes a
/// 1-year percentile on a 3-year history read differently from one on 107 years.
fn coverage_str(line: &SeriesLine) -> String {
    let freq = match line.frequency {
        Frequency::Daily => "日頻",
        Frequency::Monthly => "月頻",
    };
    // `自1986 日` reads like a truncated date; the separator makes it a year plus
    // a frequency, which is what it is.
    match &line.coverage_start {
        Some(start) => format!("自{}・{}", &start[..4.min(start.len())], freq),
        None => format!("自—・{freq}"),
    }
}

fn format_series_row(line: &SeriesLine, w: &RowWidths) -> String {
    format!(
        "  {}  {:>vw$}   {}   {}",
        pad_to(&label_str(line), w.label),
        value_str(line),
        pad_to(&windows_str(line), w.windows),
        coverage_str(line),
        vw = w.value,
    )
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
