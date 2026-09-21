# Phase ① — intentional differences from the Python

Every entry is a deliberate decision. Anything not listed here is a bug.

1. **`emit_skill_status` cannot receive an invalid status.** Python raises
   `ValueError` before checking the env; Rust's typed `SkillStatus` makes it
   unrepresentable. The string boundary is still tested via `parse_status`.
2. **`deliver` returns an outcome instead of calling `sys.exit(1)`.** nullclaw
   classifies exit code and semantic status independently, so the binary owns
   the exit. Observable behaviour is identical.
3. **`SendOptions::base_url` and `DOUGHCON_BASE_URL` are test seams.** Both
   default to the production host, so scheduled runs are unaffected.
4. **A non-object PizzINT payload degrades instead of crashing.** Python would
   throw outside the fetch handler and exit hard; Rust rejects it at the adapter
   boundary and routes it to the same degraded path as a fetch failure. This is
   a compatibility fix, not a silent change.
5. **Telegram response bodies are still not parsed.** HTTP 200 is success. Rust
   must never start requiring `{"ok":true}`.
6. **Timestamps with a non-UTC offset** are accepted by both; a timezone-naive
   timestamp is rejected by `jiff` and falls back to run time, whereas Python
   would interpret it in the host's local zone. No PizzINT payload observed has
   ever been naive; recorded because it is unverified for all future payloads.
7. **Upstream error text never matches.** `[WARN: doughcon unavailable - {e}]`
   and `[ERROR: …]` embed the HTTP client's own message, and `urllib` and `ureq`
   word them differently. The failure *class*, exit code, and skill status all
   match; the string does not. This is why no differential case forces a fetch
   failure — it could only ever report a diff that means nothing.
8. **Transport-failure diagnostics say `transport error`** where Python names the
   exception type (`URLError`, `timeout`, `TimeoutError`). Retry behaviour is
   identical. Anything scraping those type names would break.
9. **`ureq` has no generic non-retryable arm.** Python's bare `except Exception`
   logs `non-retryable error …` and stops; `ureq::Error` is exhaustive over
   `Status` and `Transport`, so that path has no equivalent and cannot fire.
10. **Request JSON bytes differ.** `serde_json::Map` is a `BTreeMap`, so keys
    serialise alphabetically and without spaces, where Python emits insertion
    order with `json.dumps` default separators. Telegram semantics are identical;
    the wire bytes are not.
11. **Argument-parser surface differs.** The hand parser rejects `--mode=deliver`
    (space form only) and has no `--help`, where `argparse` accepts both. Error
    text differs. `--et-hour` remains deliberately un-range-checked in both.

---

# Phase ② (weather) — intentional differences from the Python

Every entry is a deliberate decision. Anything not listed here is a bug.
The end-to-end differential (`tools/differential/weather.sh`) applies exactly
two masks before comparing; residual diffs after those masks are findings.

12. **`HKO_BASE_URL` / `CWA_BASE_URL` / `OPEN_METEO_BASE_URL` are test seams.**
    Same role as `DOUGHCON_BASE_URL` (entry 3): production defaults when unset.
    The frozen Python has no equivalent env vars; the differential redirects it
    by monkeypatching `urllib.request.Request` and routing on URL substring
    (`hko` / `opendata.cwa` / `open-meteo`). Scheduled runs are unaffected.

13. **`elapsed_ms` digits differ** in `[skill-event] … and took Nms`. Both sides
    time only the Open-Meteo window (Python `run.py:301-303`; Rust
    `orchestrate.rs` Instant around `open_meteo_for_locations`). The quantity
    matches; wall-clock jitter does not. Differential mask:
    `and took \d+ms` → `and took <MS>ms`.

14. **Exception text inside CWA failure reasons and Open-Meteo WARN tails
    differs.** Python embeds `type(e).__name__: {e}` (e.g. `HTTPError: HTTP
    Error 500: …`); Rust embeds the `ureq` / adapter error string (e.g.
    `Error: http status: 500`). Failure *class*, exit code, skill status, and
    the non-exception reason strings (`CWA_API_KEY is not set…`, `CWA returned
    an empty record list`, `CWA did not return data for N of M locations`,
    `KeyError: 'elementName'`) match. Differential masks:
    - `CWA request failed with \w+: .*` → `CWA request failed with <EXC>`
    - `[WARN: Open-Meteo unavailable for … - .*]` tail → `<EXC>]`

15. **Agent binary is resolved through `$HOME` on both sides.** Python:
    `os.path.expanduser("~/nullclaw/zig-out/bin/nullclaw")`. Rust:
    `std::env::var_os("HOME")` + the same relative path. This is the shared
    injection seam the weather differential depends on — not a divergence.
    Do not replace the Rust path with a constant or a bespoke env var.

### Phase ② differential result (Task 10)

Seam verification: both sides emit the planted fixed advice string under
`HOME=$STAGE`. All 14 cases in `tools/differential/weather_cases.tsv` match
after the two masks above — zero residual findings.
