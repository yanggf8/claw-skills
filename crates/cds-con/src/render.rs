//! Message rendering for the attribute-2 reading.
//!
//! The live message is [`render_cost_parts`]: the Baa corporate bond yield's
//! own level, its as-of percentile, the 高/不高 label that follows from it,
//! and the trailing windows as collapsible background. `finance-cli`'s
//! `cost level` is the authority for the rule; `crate::cost` mirrors it and
//! this module only lays the result out.
//!
//! **Collapsible evidence.** [`render_cost_parts`] tags every segment with
//! [`Segment::evidence`], marking the 佐證 block (from the `──── 佐證 ────`
//! separator through its last line). [`render_cost_lines`] ignores that tag
//! and stays plain text, unchanged byte for byte -- stdout and an agent
//! reading the run never see markup. [`flatten_html`] consults it to wrap
//! that block in `<blockquote expandable>`, which Telegram (HTML
//! `parse_mode`) collapses behind a tap; nothing else is bold, italic, or
//! otherwise marked up. Every character reaching the HTML payload is escaped
//! by [`escape_html`] first, since the wrapped text includes owner-edited
//! `cds_series` labels and an unescaped `<`/`&` would make Telegram reject
//! the whole message.
//!
//! The retired spread message (the `baa10y`/`baa` lead pair, the 佐證 series
//! blocks, and the four readability passes behind them) is documented in
//! `docs/specs/2026-08-04-cds-con-readability-v2-design.md`. Its renderer was
//! removed once attribute 2 became the Baa yield's own level rather than the
//! Baa−Aaa direction; the historical notes in that spec still explain the
//! shape, but nothing here describes what gets delivered anymore.

use crate::cost::{label, Level};
use credit_store::{below_and_total, window_counts, Observation, SeriesKind, SeriesSpec, WindowCounts};

/// Truncated (never rounded) tenths-of-a-percent share of `below` within `n`,
/// e.g. `(2, 3)` -> `"66.6"`, never `"66.7"`. Integer arithmetic on the exact
/// values that are also printed as the count, so the share can never disagree
/// with `{below} 筆比這一筆低` on the same line -- there is no separate
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
/// these directly; production goes through [`analyze`] then
/// [`render_cost_lines`].
#[derive(Debug, Clone)]
pub struct SeriesLine {
    pub key: String,
    /// Human-facing name. Comes from the `cds_series` config's Label field --
    /// it is data, not code, so translating the message never touches Rust.
    pub label: String,
    pub kind: SeriesKind,
    pub value: Option<f64>,
    pub windows: Vec<WindowPct>,
    pub coverage_start: Option<String>,
    pub latest: Option<String>,
    pub frequency: Frequency,
    /// Position in the configured series list; used only as a stable tie-break
    /// when two series share a coverage start.
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
/// `render_cost_parts` is the structural seam: tests consult `kind`, never the
/// text, to decide whether a line is subject to the width bound.
#[derive(Debug, Clone)]
pub struct Segment {
    pub text: String,
    pub kind: LineKind,
    /// Whether this line belongs to the collapsible 佐證 (evidence) block --
    /// consulted ONLY by [`flatten_html`] to place the `<blockquote
    /// expandable>` boundary. Set structurally by [`render_cost_parts`] as it
    /// pushes segments (an index range it captures itself), never
    /// reconstructed afterward by matching rendered text for
    /// `──── 佐證 ────` -- the same discipline `kind` already holds for the
    /// width bound.
    pub evidence: bool,
}

fn prose(text: impl Into<String>) -> Segment {
    Segment { text: text.into(), kind: LineKind::Prose, evidence: false }
}

fn data(text: impl Into<String>) -> Segment {
    Segment { text: text.into(), kind: LineKind::Data, evidence: false }
}

fn blank() -> Segment {
    prose(String::new())
}

/// Width bound the renderer *enforces* on every title line (see
/// [`title_lines`]/[`display_width`]), and the gate
/// `every_data_line_fits_the_width_bound` in `tests/cost_message.rs` checks
/// the rest of a block against via [`width_bound`]. The transport
/// (`parse_mode: HTML`) renders a proportional font, so this CJK-is-2-columns
/// model is a coarse proxy, not a guarantee against wrapping on a real phone
/// -- but it is an active guarantee for the title line, not a hope that a
/// config label stays short: a title line that would exceed this bound
/// splits (label alone, then `  value` indented like a window row) instead
/// of quietly growing past it.
///
/// The one line this cannot guarantee: a label that alone exceeds the bound
/// still overflows on its own line. The renderer never truncates a
/// configured label to force a fit -- discarding real config data would be
/// worse than one wrapped line.
const WIDTH_BOUND: usize = 48;

/// Test seam for [`WIDTH_BOUND`]. Kept private otherwise -- nothing else in
/// production rendering needs the raw constant once [`title_lines`] does its
/// own comparison.
pub fn width_bound() -> usize {
    WIDTH_BOUND
}

/// Display-column width of `s` under the CJK-is-2 model [`WIDTH_BOUND`]
/// assumes: CJK ideographs, Hangul syllables and fullwidth forms count as 2,
/// everything else as 1. This is the one place production rendering does
/// column math -- [`title_lines`] uses it to decide whether a title line
/// must split, which is what turns the width bound from a hope about
/// `cds_series.label` staying short into something the renderer actually
/// enforces. Mirrors (as an independent copy, not a shared import) the model
/// `tests/cost_message.rs` checks output against.
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

/// Analyze configured series: require kind, compute windows, sort each family
/// by longest coverage first (earliest start), config order on ties.
pub fn analyze(series: &[SeriesInput]) -> Result<Vec<SeriesLine>, RenderError> {
    // Missing kind fails loudly — never a guessed default.
    for s in series {
        s.spec.require_kind().map_err(|message| RenderError { message })?;
    }

    let mut lines: Vec<SeriesLine> = Vec::with_capacity(series.len());

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

/// The attribute-2 message: the Baa yield's own level, its as-of reading, and
/// the trailing windows as collapsible background.
///
/// **The subject is not configurable.** Attribute 2 is defined on the Baa
/// corporate bond yield, so which series answers it is a property of the
/// research, not of a config row. A key that could silently redirect it is the
/// exact shape of drift that made attribute 2 measure the wrong thing for as
/// long as it did.
///
/// `as_of` is an injected YYYY-MM-DD used only for age-in-days on the
/// freshness line. No clock.
///
/// The reading carries a label, because there is now one fixed basis to label
/// against: an as-of expanding window, cut at the median. The trailing windows
/// below it carry **no** label — they are a different basis, and labelling
/// each would put several verdicts on one number, which is the objection that
/// used to rule out labelling anything at all. They are evidence for the
/// reader, not a second opinion.
pub fn render_cost_parts(line: &SeriesLine, level: Option<&Level>, as_of: &str) -> Vec<Segment> {
    let mut out: Vec<Segment> = vec![prose("💾 企業債成本｜attribute 2"), blank()];

    out.extend(title_lines(&line.label, &value_with_date(line)).into_iter().map(data));
    out.push(data(match level {
        Some(l) => format!("狀態：{}（as-of 分位 {}，n={}）", label(l.pct), l.pct, l.n),
        // Not a level of zero, and not silence: the series does not reach this
        // month. `cost level` prints the same row rather than dropping it,
        // because a missing line looks like a query that was never run.
        None => "狀態：無資料".to_string(),
    }));

    out.push(blank());
    // Mechanism, never today's numbers, so no line here can become false on a
    // day the market moves the other way.
    out.push(prose("量的是殖利率本身,不是任何相減後的利差"));
    out.push(prose("分位只用該月之前(含當月)的觀測,不回望未來"));
    out.push(prose("切點是中位數,分位 ≥ 50 為「高」"));
    out.push(blank());

    // The boundary is structural knowledge captured where the segments are
    // pushed, never re-derived by matching rendered text.
    let evidence_start = out.len();
    out.push(prose("──── 佐證 ────"));
    let windows = window_lines(line);
    if !windows.is_empty() {
        out.push(blank());
        out.extend(windows.into_iter().map(data));
        out.push(blank());
        out.push(prose("這幾個窗口不是判定的依據,判定只看上面那個 as-of 分位"));
    }
    let evidence_end = out.len();
    for seg in &mut out[evidence_start..evidence_end] {
        seg.evidence = true;
    }

    out.push(blank());
    out.push(prose(format_freshness_line(std::slice::from_ref(line), as_of)));
    out.push(prose("finance-cli `cost level` 是這條規則的權威,本訊息跟隨它。"));

    out
}

/// Plain-text body of the attribute-2 message.
pub fn render_cost_lines(line: &SeriesLine, level: Option<&Level>, as_of: &str) -> String {
    flatten_plain(&render_cost_parts(line, level, as_of))
}

/// Both delivery representations from ONE [`render_cost_parts`] call, so the
/// delivered body and the stdout fallback can never disagree on content.
pub fn format_cost_variants(
    series: &SeriesInput,
    level: Option<&Level>,
    as_of: &str,
) -> Result<MessageVariants, RenderError> {
    let lines = analyze(std::slice::from_ref(series))?;
    let line = lines
        .first()
        .ok_or_else(|| RenderError { message: "no series line produced".into() })?;
    let parts = render_cost_parts(line, level, as_of);
    Ok(MessageVariants {
        plain: flatten_plain(&parts),
        html: flatten_html(&parts),
    })
}

/// Flatten segments into the plain-text message body -- what stdout and an
/// agent reading the run always see, regardless of transport. No markup of
/// any kind; this project is text-first and this is the oldest/plainest
/// representation.
fn flatten_plain(parts: &[Segment]) -> String {
    parts.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join("\n")
}

/// Escape every segment FIRST, then wrap -- never the other way round (an
/// escape pass run after wrapping would mangle the very tags this function
/// adds). The `<blockquote expandable>` / `</blockquote>` pair is placed by
/// INDEX (the first and last segment with `evidence == true`), so there is
/// always exactly one of each, never zero and never more, regardless of how
/// many lines the 佐證 section holds.
fn flatten_html(parts: &[Segment]) -> String {
    let mut texts: Vec<String> = parts.iter().map(|s| escape_html(&s.text)).collect();
    let evidence: Vec<usize> = parts
        .iter()
        .enumerate()
        .filter(|(_, s)| s.evidence)
        .map(|(i, _)| i)
        .collect();
    if let (Some(&first), Some(&last)) = (evidence.first(), evidence.last()) {
        texts[first] = format!("<blockquote expandable>{}", texts[first]);
        texts[last] = format!("{}</blockquote>", texts[last]);
    }
    texts.join("\n")
}

/// HTML-escape `&`, `<`, `>` -- the three symbols Telegram's HTML parse mode
/// requires escaped outside of a literal tag (Bot API docs: "All &lt;, &gt;
/// and & symbols that are not a part of a tag or an HTML entity must be
/// replaced with the corresponding HTML entities"). `&` MUST run first:
/// escaping `<`/`>` before `&` would re-escape the `&` this function itself
/// just introduced inside `&lt;`/`&gt;`.
///
/// Series labels come from the `cds_series` DB config, which the owner
/// edits -- an unescaped `<` or `&` in a label would make Telegram reject
/// the whole message once `parse_mode` is HTML (see `deliver_options`'s doc
/// comment), so this runs on every segment's full text before any tag is
/// placed around it. Nothing that reaches [`flatten_html`] is exempt:
/// labels, keys, dates, values and counts all arrive already concatenated
/// into one segment's text, so escaping the segment covers every field
/// inside it by construction -- there is no per-field call site to miss.
pub fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Build a title line of `{label}  {value_text}`, splitting into two lines
/// (label alone, then the value indented like a window row) when it would
/// exceed [`WIDTH_BOUND`]. The label comes from the `cds_series` config,
/// which no Rust test can see, so this bound is enforced here rather than
/// trusted to stay short. The renderer never truncates a configured label to
/// force a fit -- discarding real config data would be worse than one wrapped
/// line. See [`display_width`] and the `every_data_line_fits_the_width_bound`
/// test in `tests/cost_message.rs`.
fn title_lines(label: &str, value_text: &str) -> Vec<String> {
    let combined = format!("{label}  {value_text}");
    if display_width(&combined) <= WIDTH_BOUND {
        vec![combined]
    } else {
        vec![label.to_string(), format!("  {value_text}")]
    }
}

/// One line per trailing window the series can support, total named before
/// the part (`{n} 筆裡 {below} 筆比這一筆低`, not `{below}/{n}`). When
/// something sits below (`below > 0`), the truncated share rides alongside
/// in parentheses, computed from the very `below`/`n` printed on the same
/// line via [`truncated_pct_str`], so the two can never disagree. When
/// nothing sits below (`below == 0`), the parenthetical is dropped rather
/// than printing `(0.0%)`: a bare `0.0%` cannot tell a reader whether the
/// window's floor is EXACTLY zero or merely truncated down from something
/// small, but the count sitting right there (`0 筆比這一筆低`) can.
fn window_lines(line: &SeriesLine) -> Vec<String> {
    line.windows
        .iter()
        .map(|w| {
            if w.below == 0 {
                format!("  {} {} 筆裡 {} 筆比這一筆低", w.label, w.n, w.below)
            } else {
                format!(
                    "  {} {} 筆裡 {} 筆比這一筆低({}%)",
                    w.label,
                    w.n,
                    w.below,
                    truncated_pct_str(w.below, w.n)
                )
            }
        })
        .collect()
}

/// Both delivery representations of the message: the plain body (stdout and
/// the agent-readable fallback) and the HTML body (Telegram only). Built from
/// ONE [`render_cost_parts`] call by [`format_cost_variants`], so the two can
/// never disagree on content -- only on markup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageVariants {
    pub plain: String,
    pub html: String,
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

/// `{value}  {MM-DD}` when the series has a latest observation date, or the
/// bare value (including the `n/a` case) when it does not. The title line
/// states which date its own value is from, instead of the whole message
/// borrowing one shared date for the header. That borrowed date was a defect,
/// not a simplification: even two daily series from different providers can
/// post their latest observation a day apart -- a single header date silently
/// implied one snapshot that was not true. `MM-DD` (not the full year) matches
/// `format_freshness_line`'s monthly line, which already drops the year down
/// to `YYYY-MM`: the footer states the full date once, so the title line
/// only needs enough to distinguish "this series" from "that series",
/// not repeat the year on every line.
fn value_with_date(line: &SeriesLine) -> String {
    match latest_md(line) {
        Some(d) => format!("{}  {d}", value_str(line)),
        None => value_str(line),
    }
}

/// `MM-DD` slice of a series' own `latest` date (`YYYY-MM-DD`), or `None`
/// when the series has no observation to date. Deliberately independent of
/// [`year_str`]'s year slice -- this is the complementary suffix, not a
/// shared helper, because the two never need to change together.
fn latest_md(line: &SeriesLine) -> Option<String> {
    line.latest.as_ref().map(|d| {
        let m = &d[5..7.min(d.len())];
        let day = &d[8..10.min(d.len())];
        format!("{m}-{day}")
    })
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
