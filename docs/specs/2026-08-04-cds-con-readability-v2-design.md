# cds-con readability v2 — design

Second readability pass. The first one (2026-08-02, recorded in
`2026-08-01-cds-con-intentional-differences.md` §「Readability pass」) fixed the
*words* — Chinese labels, `%` units, whole-number percentiles, a worked example.
The reader's verdict on the result was **improved but still not enough**, and
named the remaining problem as the nouns and the percentile itself.

Investigating that turned up a second, independent defect the first pass never
looked at: the message is laid out as a table on a transport that cannot render
one. Both are addressed here.

Status: design approved 2026-08-04. Not yet implemented.

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
widest data row is **85 display columns** and 14 of 18 lines exceed 40. Even in a
monospace context those wrap on a phone.

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

Result, measured: **every data line is ≤ 38 columns.** The only lines over 40 are
prose (block headers, footer), where wrapping is harmless.

| | current | v2 ordinary day | v2 days 1–7 |
|---|---:|---:|---:|
| lines | 18 | 41 | 58 |
| widest line | 85 | 60 | 60 |
| widest **data** line | 85 | **38** | **38** |
| lines > 40 cols | 14 | 4 | 4 |

Length is the price paid for width, and it is why the daily set was cut (§4).

### 2. Percentiles become counts: `N/M 筆低於本次(XX.X%)`

`近1年 p24` becomes `近1年 61/250 筆低於本次(24.4%)`.

Three things this fixes at once:

- **It explains rather than interprets.** The distinction Grok missed: a
  percentile cannot be *interpreted* without picking a window (that is a verdict
  and stays banned), but it can be *defined* in place. "61 of 250 observations
  are lower than this one" is arithmetic, not a judgment.
- **It matches the implementation.** 「低於」 is strictly-below, as
  `stats.rs:26` is.
- **★ It removes the `p0` ambiguity.** Truncation maps the whole interval
  `[0,1)` onto `p0`, so `p0` currently conflates "this is the lowest value in
  the window" with "0.9% of the window is lower". A count cannot be ambiguous:
  `0/13` and `11/1291` are different statements. Verified against live data —
  `baa−aaa`'s 1-year and 10-year percentiles are **exactly 0.0000**, so today a
  reader inferring "nothing lower" is right; on another day, at a true 0.9, the
  same `p0` would make them wrong.

Percentages keep the **truncate-never-round** rule already in force: the display
may never claim a higher rank than the data supports. One decimal.

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

### 4. Six daily series every day; three monthly series on days 1–7

Daily: `baa10y`, `hy_oas`, `ig_oas`, `hy_yield`, `ig_yield`, `ccc_yield`.
Monthly: `baa−aaa`, `baa`, `aaa`.

The split is by **publication frequency**, not by value — the rule is identical
whichever way the market moves, so it is not a ladder. The monthly rows changing
once a month do not deserve a third of the daily message on the other 29 days.

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

**The safeguard makes the proxy safe to be wrong.** The footer always carries a
monthly status line — `月頻 3 列 資料至 2026-07,未展開(每月 1–7 日展開)` — so a
late FRED publication is visible on every ordinary day even though the block is
collapsed. Nothing is ever silently dropped. The `7` belongs in config, not in
code.

### 5. Deliberately unchanged

- **No change-since-last-observation column.** Proposed (a signed delta is
  arithmetic, not a verdict, provided every row uses the same rule and none is
  singled out). Declined: the stated problem was comprehension, not staleness,
  and it costs six more lines.
- **`SIGNAL-ONLY` stays, marker and prose both.** The argument for `DATA-ONLY` —
  that this message is telemetry and the marker implies a meaning that is not
  there — is sound, but `SIGNAL-ONLY` is a project-wide boundary marker shared
  with chipcon / inflation-con / oilcon and pinned by contract tests. Changing
  it in one skill would fracture it; changing it everywhere is separate work.
- **The two-block split, all windows side by side, per-line coverage, the
  freshness line.** Rejected for removal in the first pass; that still holds.
  This design *strengthens* them: distinct start years make the incomparability
  visible where `全庫` hid it.

## Target output

Ordinary day (41 lines, data ≤ 38 cols):

```
💾 信用利差

利差 —— 相對某個基準多出的殖利率

Baa 比 10年期美債多出的殖利率 [baa10y]
  1.63%  日頻・自1986
  近1年     61/250 筆低於本次(24.4%)
  近10年   316/2495 筆低於本次(12.6%)
  自1986  1397/10145 筆低於本次(13.7%)

高收益債相對基準多出的殖利率 [hy_oas]
  2.84%  日頻・自2023
  近1年    117/265 筆低於本次(44.1%)
  自2023   194/789 筆低於本次(24.5%)

…（ig_oas 同型）

總殖利率 —— 含無風險利率在內的全部借款成本(與上一區不可互比)

…（hy_yield / ig_yield / ccc_yield 同型）

資料:日 至 2026-07-30(5 天前)
月頻 3 列 資料至 2026-07,未展開(每月 1–7 日展開)
SIGNAL-ONLY:每個窗口各自回答自己的問題,不可跨列比較——
自2023 的 789 筆和自1919 的 1291 筆不是同一把尺。
```

Days 1–7 additionally carry `baa−aaa` in the spreads block and `baa` / `aaa` in
the yields block, each in the same five-line form, and the footer's monthly
status line is replaced by `・月 至 2026-07` on the 資料 line.

Ordering is unchanged: within a block, earliest coverage first, config order on
ties.

## The footer sentence changes too

「換一把尺就換一個答案」 is replaced by 「每個窗口各自回答自己的問題,不可跨列比較」.

The old wording was subtly wrong and the correction matters: p24, p12 and p13
are **simultaneously true** and answer three different questions. Nothing flips.
What flips is a conclusion a reader adds on top — which is exactly why no
verdict is emitted, so the honest framing strengthens the rule rather than
weakening it.

The worked example is dropped: with counts in every row, the demonstration is
now on every line rather than needing a separate sentence.

## Review trail

Codex was asked whether Grok's "no good answer exists" verdict held, and
answered that it is half right — a percentile cannot be **interpreted** without
a verdict, but it can be **explained**. That distinction is the basis of §2.
Its findings were corroborated against source and data rather than accepted:

- ✅ `全庫` is a DB concept and a different ruler per row — confirmed by query.
- ✅ 「信用風險本身的價格」 overclaims — confirmed against the charter's own note
  that the quality spread touches no Treasuries.
- ✅ 「the window flips the conclusion」 is imprecise — accepted, see above.
- ❌ **Its proposed wording 「不高於」 is wrong.** The implementation is strictly
  below (`stats.rs:26`). At `p0` the phrasing is self-contradictory: a value is
  never higher than itself, so 「0% 不高於本次」 cannot hold. Shipping it would
  have put a definition in the message that contradicts the code.
- ⚠️ **Its `p0` diagnosis was wrong on today's data.** It supposed a displayed
  `p0` might hide a true 0.x; both live values are exactly 0.0000. The
  underlying ambiguity it pointed at is real, but for the opposite reason, and
  §2 fixes it.

## Testing implications

- The golden-message test is rewritten wholesale; the old byte-for-byte golden
  encodes the table.
- `cjk_labels_keep_columns_aligned` is deleted with the machinery it pinned.
- New pins needed: `低於` never becomes `不高於`; a count of `0` renders as `0/N`
  and never as an empty or omitted window; percentages truncate and never reach
  a rank the data does not support; the monthly status line is present on every
  ordinary-day message; the day-1–7 rule reads its bound from config.
- The `SIGNAL-ONLY` contract test is unaffected and must stay green.

## Known limits

- **The days-1–7 window is a proxy and can be wrong.** If FRED publishes a
  monthly value after the 7th, that month's block is never expanded. The footer
  status line is what keeps this visible instead of silent; it is not a fix.
- **The message gets longer**, 18 → 41 lines on an ordinary day. Width was
  traded for length deliberately, on the grounds that a phone scrolls easily and
  wraps badly.
