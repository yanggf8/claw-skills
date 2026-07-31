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
    /// Display label: `1y`, `10y`, or `全庫`.
    pub label: &'static str,
    pub pctile: f64,
}

/// Fully computed series row, ready to format. Tests that pin layout inject
/// these directly; production goes through [`analyze`] then [`render_lines`].
#[derive(Debug, Clone)]
pub struct SeriesLine {
    pub key: String,
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

/// Display key for the derived quality spread (Unicode minus U+2212).
pub const BAA_AAA_KEY: &str = "baa−aaa";

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
    kind: SeriesKind,
    rows: &[Observation],
    frequency: Frequency,
    config_order: usize,
) -> SeriesLine {
    if rows.is_empty() {
        return SeriesLine {
            key,
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
    for (years, label) in [(1u32, "1y"), (10u32, "10y")] {
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
    out.push("💾 CDS-CON 信用利差".into());
    out.push(String::new());

    let spreads: Vec<&SeriesLine> = lines
        .iter()
        .filter(|l| l.kind == SeriesKind::Spread)
        .collect();
    let yields: Vec<&SeriesLine> = lines
        .iter()
        .filter(|l| l.kind == SeriesKind::Yield)
        .collect();

    out.push("利差(已扣無風險利率)".into());
    for line in &spreads {
        out.push(format_series_row(line));
    }

    out.push(String::new());
    out.push("殖利率(含無風險利率 — 水位高低多半反映利率,不是信用壓力)".into());
    for line in &yields {
        out.push(format_series_row(line));
    }

    out.push(String::new());
    out.push(format_freshness_line(lines, as_of));
    out.push("SIGNAL-ONLY:百分位是窗口內的排名,窗口會翻轉結論。".into());

    out.join("\n")
}

/// Analyze then render. Entry point for the skill once Task 3 wires the store.
pub fn format_message(series: &[SeriesInput], as_of: &str) -> Result<String, RenderError> {
    let lines = analyze(series)?;
    Ok(render_lines(&lines, as_of))
}

fn format_series_row(line: &SeriesLine) -> String {
    let value_str = match line.value {
        Some(v) => format!("{v:.2}"),
        None => "n/a".into(),
    };

    let windows_str = if line.windows.is_empty() {
        String::new()
    } else {
        line.windows
            .iter()
            .map(|w| format!("{} p{:.1}", w.label, w.pctile))
            .collect::<Vec<_>>()
            .join(" · ")
    };

    // Windows field width 34 left-aligns the coverage column across rows that
    // show two vs three windows (see plan golden).
    let windows_field = format!("{:<34}", windows_str);

    let coverage = match &line.coverage_start {
        Some(start) => format!("{start}→ {}", line.frequency.as_str()),
        None => format!("—→ {}", line.frequency.as_str()),
    };

    // key width 9, value width 5, then two spaces before the windows field.
    format!(
        "  {:<9} {:<5}  {}{}",
        line.key, value_str, windows_field, coverage
    )
}

fn format_freshness_line(lines: &[SeriesLine], as_of: &str) -> String {
    let daily_latest = min_latest(lines, Frequency::Daily);
    let monthly_latest = min_latest(lines, Frequency::Monthly);

    let mut parts: Vec<String> = Vec::new();

    if let Some(d) = daily_latest {
        let age = days_between(&d, as_of).unwrap_or(0);
        parts.push(format!("daily 至 {d}({age} 天前)"));
    }
    if let Some(m) = monthly_latest {
        // Monthly: state the date only — no age parenthetical in the golden.
        parts.push(format!("monthly 至 {m}"));
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
