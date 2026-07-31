# stock — Rust port

Live 2026-08-01. `crates/stock` replaces `stock/scripts/run.py` (184 lines, no
tests). No cron job exists for this skill; it is invoked by hand or by the
agent.

Same rule as the traffic port: the Python is evidence of current behaviour, not
a specification. What follows is what was decided.

## Two output changes, both deliberate

**The date is now readable.** TWSE returns `d` as `20260731` and the Python
interpolated it straight into the message, so a person read
`，20260731 13:33:00`. That is the wire format reaching the page — nobody chose
it. Now `2026-07-31 13:33:00`.

**The Hang Seng line gained 高/低 and a timestamp.** The TWSE line always had
them; the HSI line did not, and the asymmetry was not a decision. The original
HKEX path did read `high`, `low` and `updatetime`; when a Yahoo fallback was
added ahead of it, the replacement simply never read the equivalent fields.
Yahoo supplies `regularMarketDayHigh`, `regularMarketDayLow` and
`regularMarketTime`, so the two lines now match.

    before   📈 恒生指數：25884.43 +921.20 (+3.69%)
    after    📈 恒生指數：25884.43 +921.20 (+3.69%)
                高 25917.2 / 低 25622.92，2026-07-31 16:09:04

The timestamp is rendered in `exchangeTimezoneName`, not the host's clock. At
16:09 Hong Kong time the host timezone would move it to the wrong day in much
of the world.

## Kept, on purpose

- **The sign convention.** `+3186.45 (+7.98%)` on a rise; on a fall the numbers
  carry their own minus and no extra sign is added. At exactly zero the plus
  applies.
- **`-` as a price.** TWSE sends it when nothing has traded. Rendering it
  verbatim, with no change suffix, says "no trade yet"; substituting a number
  would not.
- **The source's own price string.** TWSE sends `2425.0000` for a stock and
  `43119.75` for the index — that is the exchange's precision, and reformatting
  it would be this program restating someone else's figures.
- **Two decimals.** Python's `.2f` and Rust's `{:.2}` agree, including on
  2.675, 1.005 and 2.345, because both round the underlying double. Checked
  rather than assumed, since the same class of difference in `round()` did bite
  the traffic port.

## Markers, which the Python did not emit

`[skill-status:...]` and `[trace:...]` are emitted when `NULLCLAW_JOB_ID` is
set. stock has no cron job today, and "no markers because nothing schedules it"
is exactly the reasoning that made traffic and commute unable to exist without
each other. A skill should be able to report to a scheduler whether or not one
is listening. A manual run stays clean.

Status is counted, not guessed from the text: `ok` when every requested market
answered, `degraded` when some did, `failed` when none did. On `failed` the
chat id is suppressed so nothing reaches Telegram and the scheduler's retry is
the only thing that can deliver — CLAUDE.md option A.

## Tests: 18, written before the implementation

Payloads are real, captured from TWSE and Yahoo on 2026-08-01. Two of the
expectations were wrong on the first run and the implementation was right:

- a fall of the same absolute size is −7.39%, not −7.98%. The test had been
  written by negating the rise's percentage, which ignores that the base
  changed. It now says so.
- the Hong Kong timestamp was guessed. Computed independently it is
  2026-07-31 16:09:04.

Both are recorded because they are the failure mode this file exists to
prevent: an expectation copied from intuition rather than derived.

Breaking things on purpose, to check the suite is load-bearing:

| breakage | result |
|---|---|
| drop the `+` sign | 5 red |
| stop separating the date | 1 red |
| render the timestamp in UTC | 1 red |

## Known gaps

- The HKEX endpoint the Python kept as a second fallback is gone. It carried a
  hardcoded `token=` in the URL, was only reachable when Yahoo raised, and was
  never exercised in the runs observed here. If Yahoo becomes unreliable this
  needs a real second source, not that one.
- `--market` rejects an unknown value with exit 2, as argparse's `choices` did.
  The message wording differs.
