//! Credit-spread message rendering.
//!
//! **No classification.** No `Status` enum, no `狀態：` line, no summary
//! adjective. The job is to lay out levels, percentiles and coverage honestly.
//!
//! **Lead-block redesign (2026-08-04).** The message now opens with a
//! guided PAIR — the same Baa bonds, one line with the risk-free rate
//! subtracted (a spread) and one without (a yield) — because that
//! side-by-side contrast is the one sentence the owner could read at a
//! glance ("利差在數十年低點、殖利率在一年高點"). Putting a spread and a
//! yield adjacent is normally forbidden (they are not commensurable and a
//! reader who does not know that will manufacture a signal that is not
//! there); it is safe here ONLY because the pair is the same underlying
//! bonds and an explanation line states how they relate
//! (`上面那條的算法,就是下面那條減掉十年期美債`) plus a guard against
//! decomposing the printed percentiles (`但兩排的百分比不能相減 ——
//! 排名不是水位`; see the 2026-08-04 lead-fix report for why the earlier
//! wording failed a reverse-day test and asserted an arithmetic identity the
//! layout does not support). Everything else `cds_message_series` names
//! renders below, under `──── 佐證 ────`, in the older per-series-block
//! shape. See `docs/specs/2026-08-04-cds-con-readability-v2-design.md`.

use credit_store::{
    baa_aaa_spread, below_and_total, window_counts, Observation, SeriesKind, SeriesSpec,
    WindowCounts,
};

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

/// Width bound the renderer *enforces* on every title line (see
/// [`title_lines`]/[`display_width`]), and the gate
/// `every_rendered_line_fits_its_width_bound` in `tests/render.rs` checks the
/// rest of a block against via [`width_bound`]. The transport
/// (`parse_mode: None`) renders a proportional font, so this CJK-is-2-columns
/// model is a coarse proxy, not a guarantee against wrapping on a real phone
/// -- but it is an active guarantee for the title line, not a hope that a
/// config label stays short: a title line that would exceed this bound
/// splits (label alone, then `  value` indented like a window row) instead
/// of quietly growing past it.
///
/// The one line this cannot guarantee: a label that alone exceeds the bound
/// still overflows on its own line. The renderer never truncates a
/// configured label to force a fit -- discarding real config data would be
/// worse than one wrapped line. See
/// `overlong_ascii_label_splits_the_title_line` /
/// `overlong_cjk_label_splits_the_title_line` in `tests/render.rs`.
///
/// Unchanged at 48 for the 2026-08-04 lead-block redesign, and unchanged
/// again for the same-day lead-fix pass that put each series' own `MM-DD`
/// date on its title line: measured against the live 2026-07-30 data, the
/// widest `Data` line the new shape produces is a 佐證-block title line at
/// 42 display columns (e.g. `高收益債相對基準多出的殖利率  2.84%  07-30`) --
/// two columns wider than the lead-block redesign's own widest line (the
/// compressed windows line at 41), because the appended date pushed the
/// title line past it, but still well under the bound, so it was not
/// raised. See `.superpowers/sdd/2026-08-04-cds-con-readability-v2/`'s
/// reports for the full measurements.
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
/// `shown` is already the exact, ordered set the message shows -- selection
/// against `cds_message_series` happens in [`select_message_series`], before
/// this function ever runs. `lead` is the resolved `cds_message_lead` pair
/// (each entry's `SeriesLine`, also present in `shown`, paired with its
/// display label -- see [`resolve_lead`]). This function does no
/// validation itself: it may be called with zero, one, or an arbitrary
/// number of lead entries (tests exercising the 佐證 block in isolation do
/// this), but production always supplies exactly two -- the invariant
/// [`parse_message_lead`] enforces on the config string, and
/// [`require_single_kind`] enforces on what is left over.
///
/// `as_of` is an injected YYYY-MM-DD used only for age-in-days on the
/// freshness line. No clock.
pub fn render_parts(shown: &[SeriesLine], lead: &[(&SeriesLine, &str)], as_of: &str) -> Vec<Segment> {
    let mut out: Vec<Segment> = vec![
        // No shared date on the header: the series shown do not share one
        // observation date (a daily series and a monthly series never do,
        // and even two daily series can differ by a day when one provider
        // updates before another). Each series states its own `latest` date
        // on its own title line instead (see [`value_with_date`]) -- see
        // the 2026-08-04 lead-fix report for the defect this replaced (a
        // single borrowed date implied one snapshot that was not true).
        prose("💾 信用利差"),
        // No blank line here: the legend sits directly under the header
        // because it explains the `%` on the very next lines, not a new
        // section.
        prose("%＝該窗口內,比這一筆更低的觀測比例"),
        blank(),
        prose("同一批 Baa 公司債,一條扣掉利率、一條沒扣"),
        blank(),
    ];

    push_lead_blocks(&mut out, lead);

    out.push(blank());
    // Fixed, generic mechanism prose -- never today's specific numbers, so
    // neither line can become false on a day the market moves the other way
    // (the reverse-day test). It exists because a reviewer who had not been
    // told the no-verdict rule read a high yield alone as company-level
    // stress, without seeing the matching spread sit mid-range the same day:
    // the high number was the risk-free rate, not stress. These two lines
    // are the guard against exactly that misreading, and are why a spread
    // and a yield are allowed to sit adjacent here at all (see the module
    // doc comment).
    //
    // A 2026-08-04 review caught the ORIGINAL two-line version of this guard
    // failing on both counts it was supposed to hold to: `所以下面那條高,
    // 可能是利率,不是公司快倒閉` qualified the CAUSE ("可能") but asserted
    // the LEVEL outright ("下面那條高") -- false on a day the yield falls,
    // which is a verdict, not mechanism prose. And `兩條的差就是十年期美債`
    // is true of the series' definitions but not of the two printed numbers,
    // which carry different dates (`baa10y` daily, `baa` monthly); read as a
    // same-day identity it invites subtracting the two PERCENTILES too --
    // and a percentile is a rank within a window, not a level, so it cannot
    // be decomposed the way a level can. The replacement below speaks only
    // about how the series is computed (not today's numbers) and states the
    // non-decomposition rule explicitly.
    out.push(prose("上面那條的算法,就是下面那條減掉十年期美債"));
    out.push(prose("但兩排的百分比不能相減 —— 排名不是水位"));
    out.push(blank());
    out.push(prose("──── 佐證 ────"));

    let lead_keys: std::collections::HashSet<&str> =
        lead.iter().map(|(l, _)| l.key.as_str()).collect();
    let supporting: Vec<&SeriesLine> =
        shown.iter().filter(|l| !lead_keys.contains(l.key.as_str())).collect();
    // Only add the section's blank + blocks when there is something to show
    // -- an empty supporting set (every shown series is in the lead pair,
    // which real config never does but a narrow test fixture might) must
    // not leave two blank lines in a row before the freshness line.
    if !supporting.is_empty() {
        out.push(blank());
        push_blocks(&mut out, &supporting);
    }

    out.push(blank());
    out.push(prose(format_freshness_line(shown, as_of)));
    // Fixed prose, never a computed contrast between today's specific
    // rulers -- v2's dynamic footer sentence is gone along with the
    // daily/monthly split it depended on to know which rulers were on
    // screen. No verdict, no adjective about level: only what a window is
    // for.
    out.push(prose("SIGNAL-ONLY:窗口越短對當下越敏感,越長越穩定。"));

    out
}

/// Flatten [`render_parts`] into the plain-text message body actually sent.
pub fn render_lines(shown: &[SeriesLine], lead: &[(&SeriesLine, &str)], as_of: &str) -> String {
    render_parts(shown, lead, as_of)
        .into_iter()
        .map(|s| s.text)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Push each supporting-block series, with a blank line separating
/// consecutive blocks -- none before the first (the caller already put a
/// blank line under the `──── 佐證 ────` header). Every line a block
/// contributes is tagged `Data`: a series' title line (`Label  value`) wraps
/// just as badly as its window lines if it bloats back toward the old
/// table's column width.
fn push_blocks(out: &mut Vec<Segment>, series: &[&SeriesLine]) {
    for (i, line) in series.iter().enumerate() {
        if i > 0 {
            out.push(blank());
        }
        out.extend(series_block(line).into_iter().map(data));
    }
}

/// Build a title line of `{label}  {value_text}`, splitting into two lines
/// (label alone, then the value indented like a window row) when it would
/// exceed [`WIDTH_BOUND`]. Shared by the lead block and the 佐證 block: both
/// title lines come from config (`cds_message_lead`'s override label or
/// `cds_series`' own `Label`), which no Rust test can see, so this bound is
/// enforced here rather than trusted to stay short. The renderer never
/// truncates a configured label to force a fit -- discarding real config
/// data would be worse than one wrapped line. See [`display_width`] and the
/// `overlong_*_label_splits_the_title_line` tests in `tests/render.rs`.
fn title_lines(label: &str, value_text: &str) -> Vec<String> {
    let combined = format!("{label}  {value_text}");
    if display_width(&combined) <= WIDTH_BOUND {
        vec![combined]
    } else {
        vec![label.to_string(), format!("  {value_text}")]
    }
}

/// One 佐證-block series as a block of lines: title (label, value and the
/// series' own date), then one line per trailing window carrying the full
/// count. No padding across series -- `parse_mode: None` means Telegram
/// renders this in a proportional font, so column alignment across rows was
/// never reaching the reader. See the 2026-08-04 design notes (both the
/// original readability pass and the lead-block redesign).
fn series_block(line: &SeriesLine) -> Vec<String> {
    let mut out = title_lines(&line.label, &value_with_date(line));
    out.extend(window_lines(line));
    out
}

/// One line per trailing window the series can support, total named before
/// the part (`{n} 筆裡 {below} 筆比這一筆低`, not `{below}/{n}`). When
/// something sits below (`below > 0`), the truncated share rides alongside
/// in parentheses, computed from the very `below`/`n` printed on the same
/// line via [`truncated_pct_str`], so the two can never disagree. When
/// nothing sits below (`below == 0`), the parenthetical is dropped rather
/// than printing `(0.0%)`: a bare `0.0%` cannot tell a reader whether the
/// window's floor is EXACTLY zero or merely truncated down from something
/// small, but the count sitting right there (`0 筆比這一筆低`) can -- this is
/// the lead-block redesign's asymmetry, deliberate, not an oversight.
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

/// One `cds_message_lead` entry as a compressed block: title
/// `{override_label}  {value}  {date}` (no `[key]` -- the lead reader is
/// comparing two numbers, not looking up a series to query, so the
/// operator-lookup key that the 佐證 block used to carry has no job to do
/// here), then, when the series has data, ONE line carrying every window as
/// `{window} {share}%` (never the full count -- the lead block trades the
/// count's "magnitude without mental division" for fitting the whole pair's
/// windows on two lines total; the 佐證 block below keeps the full counts).
/// A missing series still needs the pairing intact, so it renders `n/a` (no
/// date -- there is no observation to date) on the title line and no
/// windows line, the same shape a missing series gets in the 佐證 block.
fn lead_block(line: &SeriesLine, label: &str) -> Vec<String> {
    let mut out = title_lines(label, &value_with_date(line));
    if let Some(w) = lead_windows_line(line) {
        out.push(w);
    }
    out
}

/// The lead block's single compressed windows line, or `None` when the
/// series has no windows to show (missing data). Every window always prints
/// its share, including `0.0%` -- unlike the 佐證 block, the lead has no
/// count to fall back on for the zero case, so suppressing the share there
/// would leave nothing at all.
fn lead_windows_line(line: &SeriesLine) -> Option<String> {
    if line.windows.is_empty() {
        return None;
    }
    let parts: Vec<String> = line
        .windows
        .iter()
        .map(|w| format!("{} {}%", w.label, truncated_pct_str(w.below, w.n)))
        .collect();
    Some(format!("  {}", parts.join("  ")))
}

/// Push the lead pair's blocks, blank-separated, with no blank before the
/// first (the caller already placed one under the pairing prose).
fn push_lead_blocks(out: &mut Vec<Segment>, lead: &[(&SeriesLine, &str)]) {
    for (i, (line, label)) in lead.iter().enumerate() {
        if i > 0 {
            out.push(blank());
        }
        out.extend(lead_block(line, label).into_iter().map(data));
    }
}

/// One `cds_message_lead` entry: a series key and the display label
/// `cds_message_lead` supplies for that position in the pair -- distinct
/// from `cds_series`' own `Label`. The pairing needs prose that names the
/// RELATIONSHIP between the two series (`扣掉利率(利差)` / `沒扣(總殖利率)`),
/// which is the wrong length and the wrong emphasis for the series' own
/// general-purpose label. See the 2026-08-04 lead-block design note.
#[derive(Debug, Clone, PartialEq)]
pub struct LeadEntry {
    pub key: String,
    pub label: String,
}

/// Parse `cds_message_lead`: `key|Label` records joined by `;`, in display
/// order, naming the opening pair, e.g.
/// `baa10y|扣掉利率(利差);baa|沒扣(總殖利率)`. Exactly two entries are
/// required -- it is a PAIR by construction, not an open-ended list.
/// Unparseable, wrong field count, an empty field, a duplicate key, or a
/// wrong entry count all fail the run loudly, the same standard
/// `cds_series`, `cds_message_series` and a missing `kind` already hold:
/// this crate does not silently default. Whether each key actually names a
/// series present in `cds_message_series` is checked later by
/// [`resolve_lead`], the same layering `parse_message_series` /
/// `select_message_series` already use.
pub fn parse_message_lead(raw: &str) -> Result<Vec<LeadEntry>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(
            "cds_message_lead is empty; expected key|Label;key|Label (exactly two entries)".into(),
        );
    }
    let mut out: Vec<LeadEntry> = Vec::new();
    for (i, record) in trimmed.split(';').enumerate() {
        let record = record.trim();
        if record.is_empty() {
            return Err(format!(
                "cds_message_lead has an empty record at position {} in '{trimmed}'",
                i + 1
            ));
        }
        let fields: Vec<&str> = record.split('|').map(str::trim).collect();
        if fields.len() != 2 || fields.iter().any(|f| f.is_empty()) {
            return Err(format!(
                "cds_message_lead record {} ('{record}') must be key|Label, got {} field(s)",
                i + 1,
                fields.len()
            ));
        }
        if out.iter().any(|e| e.key == fields[0]) {
            return Err(format!("cds_message_lead has duplicate key '{}'", fields[0]));
        }
        out.push(LeadEntry {
            key: fields[0].to_string(),
            label: fields[1].to_string(),
        });
    }
    if out.len() != 2 {
        return Err(format!(
            "cds_message_lead must name exactly two series (got {}); it is the opening pair, not a list",
            out.len()
        ));
    }
    Ok(out)
}

/// Resolve `cds_message_lead` against the already-selected message series
/// (`select_message_series`'s output). Every lead key must be present in
/// `shown` -- an absent one fails loudly by name, never silently dropped,
/// the same standard `select_message_series` already holds for
/// `cds_message_series` itself. Order follows `lead` (config order), not
/// `shown`'s order.
pub fn resolve_lead<'a, 'b>(
    shown: &'a [SeriesLine],
    lead: &'b [LeadEntry],
) -> Result<Vec<(&'a SeriesLine, &'b str)>, RenderError> {
    let mut out = Vec::with_capacity(lead.len());
    for entry in lead {
        match shown.iter().find(|l| l.key == entry.key) {
            Some(line) => out.push((line, entry.label.as_str())),
            None => {
                let available: Vec<&str> = shown.iter().map(|l| l.key.as_str()).collect();
                return Err(RenderError {
                    message: format!(
                        "cds_message_lead names series '{}' not present in cds_message_series \
                         (which currently names: {})",
                        entry.key,
                        available.join(", ")
                    ),
                });
            }
        }
    }
    Ok(out)
}

/// The 佐證 block (whatever `cds_message_series` names outside the lead
/// pair) must be a single [`SeriesKind`]. v4 dropped the per-kind block
/// headers v3 used to keep spreads and yields apart -- the whole argument
/// for letting a spread and a yield sit adjacent in the lead is that the
/// explanation line makes it safe there, and ONLY there (see the module doc
/// comment). Mixing kinds in 佐證 would silently reopen exactly the
/// unexplained adjacency the old split existed to prevent. An empty
/// supporting set trivially passes (nothing to mix).
fn require_single_kind(supporting: &[&SeriesLine]) -> Result<(), RenderError> {
    let mut kinds = supporting.iter().map(|l| l.kind);
    let Some(first) = kinds.next() else {
        return Ok(());
    };
    if kinds.all(|k| k == first) {
        Ok(())
    } else {
        Err(RenderError {
            message: format!(
                "cds_message_series names both spread and yield series outside cds_message_lead; \
                 the 佐證 block has no per-kind header, so they would sit adjacent with no \
                 explanation of why they differ -- move one into cds_message_lead or keep only \
                 one kind ({}) outside the lead pair",
                first.as_str()
            ),
        })
    }
}

/// Analyze, select the configured message series, resolve the lead pair,
/// validate what is left over, then render.
///
/// `message_keys` is `cds_message_series`, parsed by [`parse_message_series`]
/// and resolved by [`select_message_series`] -- filtered AFTER `analyze()`,
/// never before, so the derived `baa−aaa` row (only built once `baa`/`aaa`
/// have both been analyzed) can still be named in it. `lead_config` is
/// `cds_message_lead`, parsed by [`parse_message_lead`] and resolved by
/// [`resolve_lead`] against the already-selected series.
pub fn format_message(
    series: &[SeriesInput],
    as_of: &str,
    message_keys: &[String],
    lead_config: &[LeadEntry],
) -> Result<String, RenderError> {
    let lines = analyze(series)?;
    let shown = select_message_series(&lines, message_keys)?;
    let lead = resolve_lead(&shown, lead_config)?;
    let lead_keys: std::collections::HashSet<&str> =
        lead.iter().map(|(l, _)| l.key.as_str()).collect();
    let supporting: Vec<&SeriesLine> =
        shown.iter().filter(|l| !lead_keys.contains(l.key.as_str())).collect();
    require_single_kind(&supporting)?;
    Ok(render_lines(&shown, &lead, as_of))
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

/// `{value}  {MM-DD}` when the series has a latest observation date, or the
/// bare value (including the `n/a` case) when it does not. Shared by the
/// lead block and the 佐證 block so every title line states which date its
/// own value is from, instead of the whole message borrowing one shared date
/// for the header. That borrowed date was a defect, not a simplification:
/// `cds_message_lead`'s own pair is one daily series and one monthly series,
/// and even two daily series from different providers can post their latest
/// observation a day apart -- a single header date silently implied one
/// snapshot that was not true. `MM-DD` (not the full year) matches
/// `format_freshness_line`'s monthly line, which already drops the year down
/// to `YYYY-MM`: the footer states the full date once, so the title lines
/// only need enough to distinguish "this series" from "that series",
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
