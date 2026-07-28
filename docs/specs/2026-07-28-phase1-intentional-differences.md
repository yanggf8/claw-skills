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
