# traffic — Rust port, and the deletion of commute

Live 2026-08-01. `crates/traffic` replaces both `traffic/scripts/run.py` and
`commute/scripts/run.py`; the commute skill no longer exists.

This file records what was decided, not what was copied. That distinction is
the point of the whole exercise and is stated first.

## Python was evidence, not specification

Earlier ports in this repo treated the Python as an oracle and measured success
as byte parity. That is right for anything an external system parses and wrong
for everything else, because it promotes the Python's accidents into this
program's requirements.

Three things here are contracts, reproduced exactly:

| | Parsed by |
|---|---|
| `[skill-status:ok\|degraded]` and `[trace:<id>]` on stdout | `cron.zig`, byte for byte |
| exit 0 / 1 / 2 | the scheduler's classifier |
| the delivered message format | a person, every weekday |

Everything else was re-decided. The clearest case:

**Rounding now goes half up. It used to go half to even.** Python's `round()`
rounds `2.5` to `2` and `3.5` to `4`, so the old skill reported 150 seconds as
2 minutes and 210 seconds as 4 — down then up on adjacent half-minutes. Nobody
chose that for travel time; it is a property of `round()` that leaked into the
output. A first port faithfully reproduced it with `round_ties_even()`. That
was the wrong instinct and the code was changed back.

    150s   was 2 分鐘   now 3 分鐘
    270s   was 4 分鐘   now 5 分鐘

Two other Python behaviours were deliberately not carried over: `float("1_0")`
parses as 10.0 in Python and is rejected here (underscore digit groups in a
lat/lon are not a feature anyone wants), and argparse's exact stderr wording is
not reproduced. Exit codes are.

## commute is gone

Its whole job was to add the scheduler markers, and it was only necessary
because traffic omitted them — which it did only because commute stripped
`NULLCLAW_JOB_ID` out of the child environment before invoking it
(`commute/scripts/run.py:24`). Each existed to serve the other.

traffic now emits its own markers when `NULLCLAW_JOB_ID` is set. That removes:

- one skill (74 lines)
- one subprocess per run, and its 30-second timeout
- the status heuristic `stdout.startswith("🚗 ")` — status is read from the
  outcome now, not guessed from the rendered text
- `~/.nullclaw/skill-errors.log` as a dependency. It was 0 bytes and four
  months stale, and it could only ever have caught an LLM-advice exception:
  every failure a user cares about (no API key, unknown place, TomTom down)
  printed to **stdout**, not stderr, so the log was wired to the least
  important failure class in the skill.

The 7 cron jobs were migrated from `commute` to `traffic` with the same
expressions, arguments, timeout (120s), timezone (+8) and
`verification_mode = skill_contract`.

## repair_policy changed from retry_once to none

Deliberate, and the one change a reader is most likely to question.

Every failure traffic can have returns the same answer on a second attempt — a
missing key is still missing, an unknown place name is still unknown, a TomTom
outage is still an outage. A retry cannot repair any of them. Meanwhile the
skill delivers on the degraded path (the `[WARN: traffic unavailable - ...]`
line *is* the message), and `degraded` is `verified != 1`, which is what
triggers the retry. So `retry_once` bought nothing and risked exactly the
duplicate-delivery defect CLAUDE.md §3 documents.

## Fixed during review: the API key reached the user

`RouteError::Http(e.to_string())` on a ureq error rendered the whole request
URL, and the request URL carries `key=<the API key>`. Printed through the WARN
line — which under the old commute *was* the delivered Telegram message — a 401
would have mailed the key to the reader.

    before   [WARN: traffic unavailable - https://api.tomtom.com/…?key=REAL_KEY&traffic=true: status code 401]
    after    [WARN: traffic unavailable - HTTP Error 401: Unauthorized]

`route::status_message` renders statuses directly and `e.to_string()` is never
used on a ureq error. The test asserts the *absence* of `key=` and
`api.tomtom.com` rather than an exact string, so the leak cannot return under
different wording. This was a regression introduced by the port — the Python
never had it.

## Tests: 23 unit, 9 binary

The unit tests came first, before any implementation, and were derived by
reading the Python rather than by observing the Rust.

The binary tests exist because an adversarial review found a breakage the unit
suite could not see: replacing `minutes_from_seconds(seconds)` with
`seconds / 60` in `main.rs` alone leaves all 23 green while every live route
reports the wrong number. That is lessons §1, "assertions that never see the
composition". `tests/binary.rs` spawns the real executable against a local
stub, and the stub **fails closed** — an unscripted request gets a 418 — so an
unexpected request surfaces as a failure rather than a silent pass.

Verified by breaking things on purpose and checking the right test went red:

| breakage | result |
|---|---|
| `round()` → truncating `/60` in main.rs | 1 binary test red, 23 unit green |
| `resolve` trimming its input | 1 red |
| full-width `：` → half-width | 4 red |
| missing `summary` → 0 instead of an error | 1 red |

## Known gaps

- `--from=A` is accepted by argparse and rejected here; `--help` exits 0 in
  Python and 2 here. Neither is used by any cron job.
- `claw-core`'s agent helper prints `[WARN] LLM clothing advice failed` — text
  hardcoded during the weather port, now emitted by every skill that calls it.
  Not fixed here because claw-core is in another repo.
- A float `travelTimeInSeconds` is rejected where Python accepted it. TomTom
  returns integers; unverified against a payload that does otherwise.

## Recorded, not acted on: where claw-core lives

`claw-core` sits in `~/b/gwebcdb/crates/claw-core` and every skill reaches it
through `path = "../../../../b/gwebcdb/crates/claw-core"`. The question of
whether it belongs there was reviewed on 2026-08-01 and the answer is **it
should move into this repo, but not yet** — recorded here so the reasoning is
not re-derived.

The case for moving, all verified:

- gwebcdb's own README lists its shared crates as `turso-util`, `price-cli` and
  `worker-github-oauth`. `claw-core` is not among them.
- `cargo tree -i claw-core` inside gwebcdb returns nothing. No crate in the
  repo that hosts it consumes it.
- `git log -- crates/claw-core` is 1 commit — the one that moved it in.
  `turso-util` has 8.
- Its stated justification is that `cct` and `autocli` will need it once they
  port. Neither has any Rust yet, and the Python they run today resolves its
  shared lib to `~/a/claw-skills/lib` by an explicit rule in cct's own
  `run.py`, which says the contract is "maintained in claw-skills". Placing the
  Rust twin elsewhere splits the ownership of one contract across two repos.
- The commit that moved it says of the path: "Ugly, and worth remembering if
  either repo ever moves."

And the correction that keeps it from being urgent — this is the part an
outside review understated:

**Moving claw-core does not free claw-skills from gwebcdb.** Five crates cross
that boundary, not one:

| crate | consumers here |
|---|---|
| `claw-core` | all seven |
| `market-fetch` | inflation-con, chipcon, oilcon |
| `turso-util` | cds-con, oilcon |
| `price-store` | oilcon |
| `credit-store` | cds-con |

Four of the seven skills would still need gwebcdb checked out beside this repo.
So the strongest practical argument — "you cannot build without a second
checkout" — mostly does not survive; only doughcon, weather and traffic would
be freed. The remaining case is conceptual: claw-core is claw-specific, while
the other four are genuinely shared domain crates that belong where they are.

Worth doing before the crate count grows (7 `Cargo.toml` files today, 14 once
every skill is ported), and not worth interrupting the port for.
