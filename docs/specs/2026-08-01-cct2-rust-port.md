# cct2 — Rust port

Live 2026-08-01. `crates/cct2` replaces `cct2/scripts/run.py` (531 lines).
Two cron jobs: pre-market and end-of-day.

The port found four defects that had been hiding one another. They are the
substance of this document; the translation itself was routine.

## The primary model had never once answered

`run.py` called it as

```text
nullclaw agent --provider anthropic-custom:minimax --model MiniMax-M2.7
```

and nullclaw refuses that:

```text
Config error: models.providers.<name>.base_url and custom: provider URLs must
be absolute http(s) URLs …
```

`anthropic-custom:` requires an absolute URL after the colon. The provider
registered in nullclaw's config is
`anthropic-custom:https://api.minimax.io/anthropic`; the string in
cct2/config.json was `anthropic-custom:minimax`. Every run reached the merge
step with `primary = None`.

## …and the report called that a consensus

`merge_results` wrote `"consensus": not both_present` on the non-agreeing
branch. For a ticker only one model answered, `both_present` is false, so
`consensus` came out **true** and the report filed it under 🎯 共識訊號. The
📊 單一參考 section, whose filter was `not consensus and not diverged`, could
never match: that combination is unreachable.

So every report for months read

```text
🎯 共識訊號
  • AAPL 看漲 🟢 90% — …
分析標的：5 支｜雙模型對照
```

on the strength of one model, while the other had not been contacted. The two
faults concealed each other — nobody notices a dead model when the output
claims agreement.

`Agreement` is now a three-variant enum. Consensus, Diverged and Solo cannot
overlap, the solo section is reachable, and it names which model spoke. The
footer counts: `雙模型對照`, `單一模型回應`, or `N 支雙模型對照，M 支僅單一模型`.

## Two more that would have surfaced the moment the first was fixed

**`content[0]["text"]`.** MiniMax-M2.7 returns a `thinking` block first and the
answer second. Reading index 0 raises. It never did, because the call never
completed.

**`max_tokens: 512`.** Measured on the real five-ticker prompt, MiniMax spends
**1775 output tokens** reaching an answer. At 512 and again at 2048 the reply
came back `stop_reason: max_tokens` with a thinking block and no text —
indistinguishable from the model declining to answer. Now 4096. Both endpoints
bill tokens produced, not the ceiling.

## No subprocess

Both upstreams speak the Anthropic messages API, so one HTTP client serves
both:

| | endpoint |
|---|---|
| primary | `https://api.minimax.io/anthropic/v1/messages` |
| backup | `https://open.bigmodel.cn/api/anthropic/v1/messages` |

Keys still come from nullclaw's provider table, so there is one place to rotate
them and this skill does not introduce a second. `~/.secrets` and the
environment still take precedence for BigModel, preserving the Python's order.
`nullclaw agent` is no longer invoked at runtime; `nullclaw memory get` is,
for the ticker list.

Concurrency is `std::thread::scope` with two threads, matching
`ThreadPoolExecutor(max_workers=2)`.

## Kept

- Confidence truncates: `int(c * 100)`, so 0.789 shows 78%. A model that said
  0.789 did not claim 79.
- A ticker neither model mentions is dropped, not reported as neutral. A silent
  model is not a reading.
- Divergences sort first, then by confidence. The ones asking the reader to
  decide come first.
- Option A on total failure: no rows means `[skill-status:failed]`, the chat id
  is suppressed and nothing reaches Telegram, so the scheduler's retry is the
  only path that can deliver and a rescued run produces one message rather than
  an error followed by a report. `lib/test_cct2_run.py` covered this in Python
  and its cases are carried into the Rust suite.
- The prompt, byte for byte.

## Tests: 31

Reason strings are clipped by character, not by byte — `reason[:80]` counts
characters in Python and byte-slicing a `&str` mid-codepoint panics, which
every one of these Chinese reasons would trigger.

A live run after the fixes produced a real divergence: MiniMax read NVDA
bullish at 78% while GLM read it neutral at 55%. That output shape was
unreachable before, because a divergence needs two models.
