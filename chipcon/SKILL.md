---
name: chipcon
description: Monitor semiconductor momentum for the SMH tactical position using trend and relative-strength signals, then deliver a signal-only exit-review report.
always: true
---

# chipcon

Monitor the SMH semiconductor satellite position using trend signals, not
entry-price stop levels. This skill is signal-only: it never trades and never
edits portfolio state.

Data input comes through the shared `price` CLI. Each daily run calls
`price fetch` for the configured tickers, then calls `price history` to read
accumulated closes from the shared `price-registry` for the trend calculation.
No broker state is read or changed.

## Script

```
~/.nullclaw/skills/chipcon/scripts/run.py
```

## Usage

```
python3 ~/.nullclaw/skills/chipcon/scripts/run.py
python3 ~/.nullclaw/skills/chipcon/scripts/run.py --mode record
python3 ~/.nullclaw/skills/chipcon/scripts/run.py --deliver-to 7972814626
```

## Trend Signals

The report watches:

- SMH close vs 20DMA and 50DMA
- 20DMA direction over a 5-trading-day lookback
- SMH 5-day relative strength vs QQQ
- SMH 5-day confirmation vs SOXX
- consecutive SMH down days
- overextended SMH above 20DMA

Status ladder:

| Status | Meaning |
|---|---|
| `OK` | Trend intact. |
| `YELLOW` | Momentum weakening; do not add automatically. |
| `ORANGE` | Trend deterioration; reduce review. |
| `RED` | Exit-bias review. |
| `PROFIT_PROTECT` | Trend still high but overextended; protect gains review. |
| `INSUFFICIENT_HISTORY` | Need more local history before trend can be trusted. |

Manual event checks remain outside the algorithm:

- NVIDIA / Broadcom / AMD / Micron guidance
- TSMC monthly revenue
- Microsoft / Amazon / Google / Meta capex guidance
- Export-control escalation
- SpaceX IPO / index-flow liquidity drain

## Data Store

- CLI substrate: `price fetch <ticker...>` and `price history <ticker...>`
- Source and storage are owned by `price`: Stooq CSV latest-close quotes into
  Turso `price-registry`, table `prices(ticker,date,close,source)`
- Output contract consumed by this skill: TSV `ticker date close source`
- CLI resolution order: `CHIPCON_PRICE_CLI`, `price_cli_path` in config,
  `price` from `PATH`, then local-dev fallback `~/b/gwebcdb/target/debug/price`

## Cron

Daily after the US close, Tuesday-Saturday Taiwan time:

```
nullclaw cron add-skill "30 5 * * 2-6" chipcon --deliver-to 7972814626 --timeout 180 --tz +08:00 --verify skill_contract --repair retry_once
```

The skill emits `[skill-status:ok|degraded|failed]` and `[trace:<job_id>]`
for `skill_contract` verification.

## Delivery

The Telegram report is sent as **plain text** (`parse_mode=None`). The body
is not Markdown — the status string `INSUFFICIENT_HISTORY` contains an
underscore, and the `price fetch` WARN can carry a raw Stooq anti-scraping
payload with unbalanced backticks/brackets. Under Telegram legacy Markdown
these break entity parsing (`can't parse entities`, HTTP 400) and the delivery
fails. Plain text is both correct and immune to any content.

## Degraded vs failed

- `degraded` (`skill-status:degraded`): a `price fetch` WARN was present but the
  report still built and delivered. Most common cause today is Stooq serving
  its **anti-scraping page** instead of CSV — an upstream data-source issue.
  When that blocks the tickers, history never accumulates and the status is
  `INSUFFICIENT_HISTORY`. The run still delivers; it self-reports `degraded` to
  flag the fetch warning. `ok` returns automatically once Stooq serves clean
  CSV again (no code change needed).
- `failed` (`skill-status:failed`): a hard error — registry unreachable,
  price CLI non-partial failure, or delivery failure.
