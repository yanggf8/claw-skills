# cds-con readability v2 — design

Second readability pass. The first one (2026-08-02, recorded in
`2026-08-01-cds-con-intentional-differences.md` §「Readability pass」) fixed the
*words* — Chinese labels, `%` units, whole-number percentiles, a worked example.
The reader's verdict on the result was **improved but still not enough**, and
named the remaining problem as the nouns and the percentile itself.

Investigating that turned up a second, independent defect the first pass never
looked at: the message is laid out as a table on a transport that cannot render
one. Both are addressed here.

**Scope note.** These are two independent lines of work bundled into one
release. The layout fix is forced — the transport cannot render a table, full
stop. The percentile rewrite is a readability bet, and should be judged on its
own merits rather than carried by the layout argument.

**Success criterion, stated so it can be checked.** v2 aims to make the message
*self-explanatory as to what it measures*. It does **not** aim to let the reader
finish it knowing whether credit is tight. That second thing requires choosing a
window and a threshold, which is the verdict this skill does not emit. A design
that tried to satisfy it would fail by construction.

Status: approved 2026-08-04. Not yet implemented.

## What was measured, not assumed

**The column alignment never reaches the reader.** `run.rs:124` sets
`parse_mode: None`, so Telegram renders the body as plain text in a
**proportional** font. Space padding does not produce columns in a proportional
font. Every line of `display_width` / `pad_to` / `RowWidths` computes an
alignment that is invisible on the device it is delivered to.

The setting was inherited from chipcon and inflation-con, which also use
`parse_mode: None` — but those are prose messages with nothing to align. A
survey of the sibling skills makes the mismatch plain:

| skill | parse_mode | column machinery |
|---|---|---|
| chipcon | `None` | none |
| inflation-con | `None` | none |
| oilcon | default `Markdown` | none |
| **cds-con** | **`None`** | **`display_width`/`pad_to`/`RowWidths`** |

cds-con is the only skill that builds a table, and it is on the one transport
that cannot show one.

**The rows are also too wide.** Measured on the live 2026-08-04 message: the
widest data row is **85 display columns** and 14 of 18 lines exceed 40.

**`全庫` is three different rulers wearing one label:**

| series | what `全庫` actually is | n |
|---|---|---|
| `baa−aaa` | 1919–2026 | 1291 |
| `baa10y` | 1986–2026 | 10145 |
| `hy_oas` | 2023–2026 | 789 |

107 years, 40 years and 3 years all print as `全庫`. The label hides exactly the
difference the design elsewhere works hardest to expose.

**`percentile_rank` is strictly-below.** `credit-store/src/stats.rs:26` is
`values.iter().filter(|v| **v < x).count()`. This fixes the wording: the
correct Chinese is 「低於」, never 「不高於」.

## Decisions

### 1. Vertical layout; the column machinery goes

One line per window instead of one row per series. Alignment is abandoned rather
than fixed, because on this transport it was never happening.
`display_width`, `pad_to` and `RowWidths` are deleted — not disabled — since
their sole purpose was an effect the transport cannot produce.

| | current | v2 ordinary day | v2 days 1–7 |
|---|---:|---:|---:|
| lines | 18 | 41 | 58 |
| widest line | 85 | 60 | 60 |
| widest **data** line | 85 | **38** | **38** |

Length is the price paid for width, and it is why the daily set was cut (§4).

### 2. Percentiles become counts: `N/M 筆低於本次`

`近1年 p24` becomes `近1年 61/250 筆低於本次`.

Three things this fixes at once:

- **It explains rather than interprets.** The distinction the first review
  missed: a percentile cannot be *interpreted* without picking a window (that is
  a verdict and stays banned), but it can be *defined* in place. "61 of 250
  observations are lower than this one" is arithmetic, not a judgment.
- **It matches the implementation.** 「低於」 is strictly-below, as
  `stats.rs:26` is.
- **★ It removes the `p0` ambiguity.** Truncation maps the whole interval
  `[0,1)` onto `p0`, so `p0` conflates "this is the lowest value in the window"
  with "0.9% of the window is lower". A count cannot be ambiguous. Verified
  against live data — `baa−aaa`'s 1-year and 10-year percentiles are **exactly
  0.0000**, so today a reader inferring "nothing lower" is right; on another day,
  at a true 0.9, the same `p0` would make them wrong.

**Decided: counts only — no parenthetical share.** An earlier draft wrote
`61/250 筆低於本次(24.4%)`. That was rejected by the owner on 2026-08-04.

`(24.4%)` would sit one line under `1.63%`, which is a **rate**, while the
parenthetical is a **share of observations** — two meanings of one symbol,
adjacent. That is precisely what SKILL.md's still-binding rule guards against:
*"a percentile must never carry a `%` sign"*, pinned by
`percent_marks_values_but_never_percentiles`.

The decisive argument is that the parenthetical earns nothing. All three fixes
above are carried by `N/M` alone: the count explains the definition, 「低於」
matches strictly-below, and `0/13` kills the `p0` ambiguity. The share is the
same fact displayed twice, and keeping it would have required a new invariant of
its own (the percentage must be `truncate(100*N/M, 1)` and share a source with
the count, or the count stays right while the percentage drifts).

Cost accepted: `1397/10145` is not a proportion anyone reads at a glance. Large
denominators are a display cost, not a correctness cost, and cross-window
comparison within a series is carried by the three lines sitting together.

`percent_marks_values_but_never_percentiles` therefore stays **unchanged and
green**.

### 3. `全庫` → the actual start year; labels state what is measured

`全庫` becomes `自1919` / `自1986` / `自2023`. Series names become
measurement descriptions: `Baa 比 Aaa 多出的殖利率`, `高收益債相對基準多出的殖利率`.
These live in the `cds_series` `Label` field, so this is a config change and
touches no Rust — except `BAA_AAA_LABEL`, which is computed in `render.rs` and
so is named there.

The spreads block header drops **「這是信用風險本身的價格」** and becomes
「相對某個基準多出的殖利率」. The old claim was an overstatement in general (a
spread also carries liquidity and risk premia) and simply wrong for `baa−aaa`,
which is Baa yield minus Aaa yield and **touches no Treasury at all** — the
risk-free level cancels, nothing is subtracted.

This change carries the least risk of the three and is independent of the
others; it should not be held up by them.

### 4. Six daily series every day; three monthly series on days 1–7

Daily: `baa10y`, `hy_oas`, `ig_oas`, `hy_yield`, `ig_yield`, `ccc_yield`.
Monthly: `baa−aaa`, `baa`, `aaa`.

The split is by **publication frequency**, not by value — the rule is identical
whichever way the market moves, so it is not a ladder. It is nonetheless an
**information-priority decision, not pure layout**, and §6 records what it costs.

**Why a calendar proxy and not "when it updates":** it cannot be detected. A
monthly observation is always dated the 1st of the previous month and always
lands 30–35 days old, so the gap between observation date and `as_of` is
permanently ~1 month and carries no transition signal. cds-con writes nothing,
so it has no memory of what it showed last time. `credit_spreads` is
`(series, date, value, source)` with no insertion timestamp.

Two alternatives were considered and rejected:

- **rowid freshness.** Technically viable — the writer is
  `ON CONFLICT(series,date) DO UPDATE`, an in-place upsert, so rowid preserves
  first-insertion order. Rejected because the threshold ("how recent is
  recent") would be a hardcoded magic number, and because binding display logic
  to rowid means a future table rebuild silently breaks it.
- **Letting cds-con persist state.** The only exact answer, and rejected: it
  would turn a read-only skill into a writing one and add a failure path, for a
  cosmetic gain.

**What the safeguard does and does not buy.** The footer always carries a
monthly status line — `月頻 3 列 資料至 2026-07,未展開(每月 1–7 日展開)`.

It guarantees the monthly series remain **visible as a group** and that **how far
behind the data is can always be read**. It does **not** guarantee the reader
sees the numbers: if FRED publishes after the 7th, that month's values stay
collapsed for the rest of the month and only the date stamp moves. So the honest
statement is *"a wrong proxy stays auditable, but a full monthly reading may be
absent for that month"* — not "nothing is ever dropped". This is an accepted
engineering trade under the no-state constraint, and must not be written up as
solved.

The safeguard also assumes the footer gets read, which sits awkwardly beside the
first pass's own finding that abstract footers get skipped on a phone.

**★ The status line must also name missing monthly series.**
`format_freshness_line` finds missing series by scanning the rendered lines for
`value.is_none()`. If the monthly rows are filtered out before rendering, a
missing monthly series stops being named for the 29 days the block is collapsed
— re-opening, through the back door, the exact silent-miss the safeguard exists
to close. The collapsed form must read
`月頻 3 列 資料至 2026-07,未展開(每月 1–7 日展開)・缺 aaa` when one is absent.

The `7` belongs in config, not in code. The day is evaluated against the CST
calendar date `main.rs` already computes (`cst_today()`, fixed offset +8), never
against UTC.

**Implementation note.** The derived `baa−aaa` is built inside `analyze()` from
the `baa` and `aaa` inputs, so the daily/monthly filter must be applied to the
*rendered* set, after `analyze`, never to the inputs — filtering earlier would
silently stop the quality spread from being derived at all.

### 5. Footer: the ruler contrast is computed, never written

The footer sentence becomes
「每個窗口各自回答自己的問題,不可跨列比較」, replacing
「換一把尺就換一個答案」. The old wording was subtly wrong: p24, p12 and p13 are
**simultaneously true** and answer three different questions. Nothing flips.
What flips is a conclusion a reader adds on top — which is why no verdict is
emitted, so the honest framing strengthens the rule rather than weakening it.

**The illustration must be drawn from the series actually rendered that day.**
A draft footer read 「自2023 的 789 筆和自1919 的 1291 筆不是同一把尺」 on an
ordinary day — but `自1919` is a monthly series, which is collapsed on ordinary
days, so the sentence pointed at a ruler not on screen. This is fixed as a rule,
not as a corrected sentence: take the **earliest-start and latest-start** series
among the rendered lines. Ordinary day → `自1986` (10145) vs `自2023` (789);
days 1–7 → `自1919` (1291) vs `自2023` (789).

The old worked example is dropped: with counts in every row, the demonstration
is on every line rather than needing a separate sentence. §6 records what that
loses.

### 6. What v2 destroys, stated plainly

- **Cross-series scanning.** The table let the eye run down a column and compare
  `hy_oas` against `ig_oas` at the same window. Vertical form makes that serial
  reading. This is the real cost of table→vertical, beyond line count.
- **The monthly series' daily presence.** `baa−aaa` is the longest ruler
  available and frequently sits at an extreme rank; it is absent on ~29 days a
  month. Justifying this by update frequency is sound but it is still a
  re-prioritisation of information.
- **The same-day two-window demonstration.** The old footer used that day's
  `baa10y` disagreeing with itself across two windows. Counts make the
  *definition* ubiquitous, but the one-sentence demonstration that a single
  instrument has two true answers is weaker.
- **The yields header's causal hint.** 「高低多半是利率在動,不是信用在動」 had
  interpretive flavour and also teaching value. Removing it is cleaner and
  blunter.
- **Attention gradient.** A longer message scrolls easily and is also abandoned
  more often; series at the bottom (`ccc_yield`) get read less. Scroll is not
  free.

Preserved, and to stay preserved: the two-block split, all windows side by side,
per-line coverage, a freshness line that does not judge, `SIGNAL-ONLY`,
strictly-below, and never rounding up to a rank the data does not support.

### 7. Deliberately unchanged

- **No change-since-last-observation column.** Proposed (a signed delta is
  arithmetic, not a verdict, provided every row uses the same rule and none is
  singled out). Declined: the stated problem was comprehension, not staleness,
  and it costs six more lines.
- **`SIGNAL-ONLY` stays, marker and prose both.** The argument for `DATA-ONLY` —
  that this message is telemetry and the marker implies a meaning that is not
  there — is sound, but `SIGNAL-ONLY` is a project-wide boundary marker shared
  with chipcon / inflation-con / oilcon and pinned by contract tests. Changing
  it in one skill would fracture it; changing it everywhere is separate work.
- **`parse_mode: None`.** v2 does not switch to HTML/`<pre>`. Choosing the
  vertical layout is what makes the proportional-font transport acceptable.

## Target output — ordinary day (41 lines, data ≤ 38 cols)

```
💾 信用利差

利差 —— 相對某個基準多出的殖利率

Baa 比 10年期美債多出的殖利率 [baa10y]
  1.63%  日頻・自1986
  近1年     61/250 筆低於本次
  近10年   316/2495 筆低於本次
  自1986  1397/10145 筆低於本次

高收益債相對基準多出的殖利率 [hy_oas]
  2.84%  日頻・自2023
  近1年    117/265 筆低於本次
  自2023   194/789 筆低於本次

投資級債相對基準多出的殖利率 [ig_oas]
  0.80%  日頻・自2023
  近1年    155/265 筆低於本次
  自2023   173/788 筆低於本次

總殖利率 —— 含無風險利率在內的全部借款成本(與上一區不可互比)

高收益債總殖利率 [hy_yield]
  7.16%  日頻・自2023
  近1年    250/265 筆低於本次
  自2023   409/789 筆低於本次

投資級債總殖利率 [ig_yield]
  5.39%  日頻・自2023
  近1年    259/265 筆低於本次
  自2023   580/788 筆低於本次

CCC 及以下總殖利率 [ccc_yield]
  14.29%  日頻・自2023
  近1年    261/265 筆低於本次
  自2023   735/789 筆低於本次

資料:日 至 2026-07-30(5 天前)
月頻 3 列 資料至 2026-07,未展開(每月 1–7 日展開)
SIGNAL-ONLY:每個窗口各自回答自己的問題,不可跨列比較——
自2023 的 789 筆和自1986 的 10145 筆不是同一把尺。
```

Days 1–7 additionally carry `baa−aaa` in the spreads block and `baa` / `aaa` in
the yields block, in the same form; the monthly status line is replaced by
`・月 至 2026-07` on the 資料 line, and the footer contrast becomes
`自1919 的 1291 筆` vs `自2023 的 789 筆`.

Ordering is unchanged: within a block, earliest coverage first, config order on
ties.

## Test plan

Current suite: `tests/render.rs` 15 tests, `tests/contract.rs` 17 tests.

### Deleted

| test | why |
|---|---|
| `cjk_labels_keep_columns_aligned` | pins the column machinery being removed; asserting that byte offsets differ while display columns match is meaningless once nothing is padded |
| `golden_message_matches_plan_exactly` | replaced by two goldens; the old constant encodes the table |

### Rewritten, same intent

| test | change |
|---|---|
| `precision_value_two_decimals_percentile_whole_number` | no more `pN`; becomes "value 2dp, counts are integers, no share is printed" |
| `percentile_never_displays_p100_by_rounding_up` | the guarantee survives in a stronger form: with counts there is no rounding step at all, so the test asserts the count is the raw `filter(< x).count()` and nothing derived |
| `missing_series_renders_na_and_is_named_in_freshness` | `n/a` in the vertical form |
| `spread_and_yield_blocks_are_separate_with_meaning_labels` | header text changed; the two-block assertion is unchanged |

### Unchanged, must stay green

`order_spreads_before_yields_longest_coverage_first`,
`unreachable_window_is_omitted_not_printed_as_insufficient`,
`long_coverage_series_shows_all_three_windows`,
`provenance_is_coverage_and_frequency_not_fred_id`,
`freshness_line_shows_age_without_judgment`,
`monthly_freshness_uses_minimum_latest_not_maximum`,
`missing_kind_fails_loudly_not_defaulted_to_yield`,
`closes_with_signal_only_and_has_no_status_line`,
**`percent_marks_values_but_never_percentiles`** (unchanged because the share was
dropped — see §2).

**All 17 contract tests are unaffected**, including
`parse_mode_is_none_because_the_message_has_no_markdown`, which asserts only
`opts.parse_mode == None` and never inspects the body (verified at
`tests/contract.rs:475`).

### New

| test | pins |
|---|---|
| `wording_is_strictly_below_never_at_most` | the message contains 「低於」 and never 「不高於」, and the printed count equals `values.iter().filter(\|v\| **v < x).count()` recomputed independently in the test |
| `zero_below_renders_as_zero_over_n` | a window with nothing below prints `0/13`, never an omitted, blank or `—` window. This is the `p0` fix; without it the ambiguity returns |
| `no_share_percentage_is_printed_beside_a_count` | the §2 decision; a `%` may only ever follow a value, never a count |
| `window_label_is_the_actual_start_year` | `自1919`/`自1986`/`自2023`; the literal `全庫` appears nowhere |
| `three_series_with_different_coverage_get_three_different_labels` | the fixture that made `全庫` misleading now shows three distinct rulers — the defect is pinned, not just fixed |
| `footer_contrast_uses_only_series_rendered_today` | **the footer defect, generalised.** On an ordinary-day render the sentence must not name a collapsed series' start year. Asserted by rendering an ordinary day and checking the two years named both appear on a rendered line |
| `monthly_block_expands_only_within_the_configured_day_bound` | days 1–7 expand, day 8 does not; **the bound is read from config**, and the test moves it to prove nothing is hardcoded |
| `day_bound_is_evaluated_in_cst_not_utc` | a timestamp that is day 8 in UTC but day 7 in Taipei must expand. Without this the cron misfires for a whole month and every fixed-date test still passes |
| `monthly_status_line_present_whenever_the_block_is_collapsed` | the safeguard against a late FRED publication going silent |
| `monthly_status_line_names_missing_monthly_series` | the back door found while corroborating: a missing monthly series must still be named while collapsed |
| `monthly_status_line_absent_when_expanded` | and `・月 至` appears on the 資料 line instead — mutually exclusive |
| `ordinary_day_carries_exactly_the_six_daily_series` | and day 1–7 carries nine |
| `spreads_header_makes_no_claim_about_the_price_of_credit_risk` | the removed overclaim cannot creep back |
| `every_rendered_line_fits_its_width_bound` | covers every rendered **`Data`-kind** line — a series' title, value, and window lines, tagged structurally via `Segment::kind`, not guessed from text — so a series title line at 80 columns is caught just as a window line would be. It deliberately **skips `Prose`** segments (headers, freshness line, footer): §1's own table budgets those at up to 60 columns against this 40-column bound, so holding them to it would fail by design, not catch a regression. See the honest scope note below |
| `golden_ordinary_day` / `golden_first_seven_days` | two byte-for-byte goldens replacing the one that encoded the table |

### What the width test is and is not

`every_rendered_line_fits_its_width_bound` measures display width under a
CJK-is-2 model. The transport is a **proportional** font, where that model does
not describe the real wrap point — the same fact that made the old alignment
machinery useless. So the test is a **proxy that prevents lines bloating back to
desktop-monospace-breaking size**; it is *not* a guarantee against wrapping on
the reader's phone. Only the device check below is that.

### Regressions this suite would still pass

Named so they are handled by review or by hand, not assumed away:

1. **A soft verdict re-enters the footer** (e.g. 「近1年較能代表近況」). It carries
   no status-ladder keyword, so the contract test stays green. The verdict
   boundary is held by human review, not by tests.
2. **`cds_series.Label` is wrong in the live config** — an overclaim returns, or
   `CCC` loses 「及以下」. Labels are data; no Rust test can see them.
3. **`baa−aaa`'s label drifts from the config labels' style**, since it alone
   lives in Rust (`BAA_AAA_LABEL`).

### Verified by hand at cutover, not by tests

- **Read the delivered message on the actual phone before enabling the cron.**
  The 2026-08-02 pass shipped alignment that was never visible; only a device
  check would have caught it.
- **Diff the live `cds_series` Label fields against this spec**, since they are
  data.

## Review trail

**Codex** was asked whether the first review's "no good answer exists" verdict
held, and answered that it is half right: a percentile cannot be **interpreted**
without a verdict, but it can be **explained**. That distinction is the basis of
§2. Its findings were corroborated against source and data rather than accepted:

- ✅ `全庫` is a DB concept and a different ruler per row — confirmed by query.
- ✅ 「信用風險本身的價格」 overclaims — confirmed against the charter's own note
  that the quality spread touches no Treasuries.
- ✅ 「the window flips the conclusion」 is imprecise — accepted, see §5.
- ❌ **Its proposed wording 「不高於」 is wrong.** The implementation is strictly
  below (`stats.rs:26`). At `p0` the phrasing is self-contradictory: a value is
  never higher than itself, so 「0% 不高於本次」 cannot hold. Shipping it would
  have put a definition in the message that contradicts the code.
- ⚠️ **Its `p0` diagnosis was wrong on today's data.** It supposed a displayed
  `p0` might hide a true 0.x; both live values are exactly 0.0000. The
  underlying ambiguity it pointed at is real, but for the opposite reason.

**Grok** then reviewed the written spec adversarially, having been told its own
earlier verdict was under challenge. It withdrew the over-broad half of that
verdict and accepted Codex's distinction. Its findings, each corroborated:

- ✅ **The draft footer pointed at a ruler not on screen.** Confirmed by reading
  the ordinary-day mock: it named `自1919` while the monthly block was collapsed.
  Fixed as a rule in §5, not as a corrected sentence.
- ✅ **The spec contradicted itself about the width test**, calling it
  load-bearing while also admitting it was a proxy. Confirmed; resolved above.
- ✅ **"Nothing is ever silently dropped" was too strong.** Confirmed: what is
  dropped is the values, what remains is the date stamp. §4 restated.
- ✅ **Losses were under-admitted**, principally cross-series scanning.
  Confirmed and written into §6.
- ✅ **It chose (a) on the `%` conflict** with the argument that the
  parenthetical carries none of §2's three fixes. Corroborated by inspection —
  each fix traces to the count or the wording, none to the share. The owner
  decided (a) on 2026-08-04.
- ✅ **Timezone and Label-in-DB are untestable gaps.** Confirmed: `main.rs`
  computes `cst_today()` at offset +8, so a UTC-keyed day bound would misfire;
  labels are config and invisible to Rust tests. Both now have entries above.
