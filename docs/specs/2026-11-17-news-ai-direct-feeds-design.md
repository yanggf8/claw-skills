# news AI section: direct publisher feeds instead of Google News search — design

Operator decision 2026-11-17 (「換」), after a day that showed what Google News RSS
costs the AI section. This replaces the four Google News *search* feeds the AI
section reads (`ai_us`, `ai_labs`, `ai_cn`, `ai_tw` in `crates/news/src/feed.rs`)
with a list of publisher RSS/Atom feeds the operator owns. Tech and 重大新聞 are
**out of scope** and stay on Google News for now.

Revision 2 (same day): K3.1 Pro's design review (Muse unavailable, 402 billing)
returned *block* with nine findings — cache isolation, shadow alert and budget
containment, a self-contradicting "no Google" test, filter ordering, URL
validation, an AI-wide outage that would ship silently, two unmeasurable gate
criteria, and trace contamination. All nine were checked against the code and are
addressed below; each fix names the finding it answers as **[Rn]**.

Revision 3 (same day): the re-review accepted R1, R2, R4–R7 and kept R3, R8, R9
open — an in-process shadow cannot be contained (a post-marker overrun is still a
`timeout` and `retry_once` re-delivers), a time-scoped trace tag mislabels
abandoned worker threads, and rendered output carries no `#N` to count. The
shadow became a separate subcommand under its own cron job (§6) and the gate
counts story blocks from one event emitted in every mode (§7–§8).

## Why

Measured on 2026-11-17, not argued:

- **Readers cannot open the links.** Every Google item is a
  `news.google.com/rss/articles/…` redirect; the operator got Google's
  "automated queries" page on two phones on mobile networks.
- **Decoding is itself Google.** `f879be2` renders the publisher URL that Tier-2
  precheck decodes, but the decode is two Google requests per pick (article page
  + `batchexecute`). With the NUC's own address intermittently 429'd, a resend
  decoded 9 of 18 links; the other 9 still go through Google.
- **Volume invites the 429.** Each run makes 4 AI searches, 2 decode calls per
  checked pick and Google paywall searches, three runs within six minutes. Whether
  that volume earns the 429 is not established, but every request removed helps
  the sections that stay on Google.
- **Titles only.** Google RSS carries a title and a redirect. Techmeme carries an
  editor-written summary of ~1,000 characters; publisher feeds carry a description.
  Expected to help precision, not same-event recall (the 2026-08 finding: hard
  recall is an LLM-judgement limit, not an information limit).

## Probe (NUC, 2026-11-17, production UA `nullclaw-news/1.0`, gzip accepted)

| Feed | Result |
|---|---|
| `https://www.techmeme.com/feed.xml` | 15 items; link is a Techmeme permalink, original URL is the first non-Techmeme `<A HREF>` in the description; title ends `(Author/Publication)` |
| `https://openai.com/news/rss.xml` | ~1,400 items — the whole archive |
| `https://blog.google/technology/ai/rss/` | 20 items |
| `https://deepmind.google/blog/rss.xml` | 100 items; **always gzip-encoded**, even unrequested |
| `https://www.theverge.com/rss/ai-artificial-intelligence/index.xml` | 10 Atom entries |
| `https://techcrunch.com/category/artificial-intelligence/feed/` | 20 items |
| `https://feeds.arstechnica.com/arstechnica/technology-lab` | 20 items |
| `https://www.technologyreview.com/topic/artificial-intelligence/feed` | 10 items |
| `https://technews.tw/category/ai/feed/` | 40 items |
| `https://www.ithome.com.tw/rss/ai` | 30 items, distinct from the all-news `/rss` |
| `https://www.anthropic.com/news/rss.xml` | 404 — Anthropic publishes no feed; its news arrives via Techmeme and outlets |
| `https://www.bnext.com.tw/rss` | 404 |

Candidates for the operator's list, not defaults.

## Design

### 1. The feed list is operator config, not code

`~/.nullclaw/news-ai-feeds.json` (path via `config.rs`, like
`news-quality-sources.json`):

```json
{
  "feeds": [
    {"name": "Techmeme", "url": "https://www.techmeme.com/feed.xml", "kind": "techmeme", "max_items": 15},
    {"name": "OpenAI", "url": "https://openai.com/news/rss.xml", "max_items": 5},
    {"name": "iThome", "url": "https://www.ithome.com.tw/rss/ai", "max_items": 10, "naive_date_offset": "+08:00"}
  ],
  "max_age_hours": 30,
  "feeds_deadline_secs": 40
}
```

The crate ships **no** feed list. `name` becomes the item's source; `kind`
defaults to plain RSS/Atom, `techmeme` turns on the unwrap in §2; `max_items` caps
one feed **after** filtering (§3).

`naive_date_offset` (optional, `±HH:MM`) is the UTC offset the operator declares
for a feed whose dates carry **no zone at all**. iThome emits
`<pubDate>2026-11-17 16:29</pubDate>` — neither RFC 2822 nor RFC 3339 — and on
2026-11-17 its newest item read 16:29 while Taipei time was 17:4x, so its clock
is +08:00 (read as UTC it would be seven hours in the future). The offset applies
only to such zoneless `YYYY-MM-DD HH:MM[:SS]` values; a date that carries a zone
always uses its own. Without the field a zoneless date stays `undated`, counted
in the trace, so a feed that needs one is visible rather than guessed. The code
assumes no zone for any feed.

### 2. Items must look like Google items to everything downstream

Every downstream step — `extract_source_name`, `title_without_source`, clustering,
paywall/deny rules keyed by source name, `replacement_query` — assumes the Google
invariant `"<headline> - <Source>"`, and `title_without_source` cuts at the
**last** ` - `. A bare direct title containing ` - ` would lose its tail. So a
direct item's `title` is built as `"{headline} - {name}"` at parse time and its
`source` is `name`. Nothing downstream changes.

`kind: techmeme`: headline = title minus its trailing `(Author/Publication)`;
source = that Publication; link = first `HREF` in the description whose host is
not `techmeme.com`. A Techmeme item for a WSJ story is therefore a WSJ item, and
WSJ's paywall/deny rules apply to it.

**URL validation at ingestion [R6].** Every direct item's link — plain or
Techmeme — must pass the same rule as `publisher_link` (absolute `http`/`https`
with a host; no whitespace, control characters, `(`, `)` or `\`). An item that
fails is dropped and counted (`invalid_link`), never rendered: for a direct item
there is no safe fallback link, because the fallback *is* the item's own URL.
Tests: empty, relative, `ftp:`, `javascript:`, parenthesised, backslashed.

### 3. Recency replaces `when:1d` — filter, then cap [R5]

Google filtered to one day server-side; direct feeds return archives. Per feed,
in this order:

1. parse **every** item (no early stop — the existing `parse_rss(max_items)`
   stops at the first N, which on the OpenAI archive could be N stale items
   while fresh ones sit later);
2. drop items whose `pubDate` (RFC 2822) or Atom `published`/`updated`
   (RFC 3339) is older than `max_age_hours` (`stale`) or unparseable (`undated`);
3. drop invalid links (§2);
4. sort the survivors newest first and keep `max_items`.

Parsing whole archives is bounded by the response size; a body over 5 MiB is
rejected as `oversize` before parsing. Test: stale and undated items occupying the
first N positions ahead of a fresh one — the fresh one must survive.

### 4. Fetching: Atom, gzip, and a total deadline

- **Atom** — `<entry>`, `<link href="…"/>` as an attribute (prefer
  `rel="alternate"` or no `rel`), `<published>`/`<updated>`. Plain RSS parsing
  stays byte-identical; the existing parser tests guard it.
- **gzip** — DeepMind always gzips. ureq 2's default features decode it, but this
  is asserted by a test against a gzip-encoded local stub, not assumed.
- **Deadline [R3]** — feeds are fetched concurrently (one thread per feed, each
  through `claw_core::http::agent(FEED_TIMEOUT)`), and the whole direct fetch
  stops at `feeds_deadline_secs`; a feed still pending is recorded as `timeout`.
  The per-feed outcome is traced (§7).

### 5. Precheck never touches Google for a direct item [R4]

`precheck_action` calls `decode_google_news_url` on whatever link it is given.
Whenever the link's host is not `news.google.com`, `precheck_apply` passes the
link itself as `decoded_url_hint` (the parameter exists; it passes `None` today).
Paywall, deny, media and body checks then run on the real publisher URL.

The **paywall replacement** search (Google + Bing) is kept, so the invariant is
stated precisely: *direct mode makes no Google request to ingest, decode or check
AI items*. A paywalled pick may still trigger a replacement search, exactly as in
the other sections; its volume is traced (`paywall_replacement_*`) and revisited
if it matters. The test asserts zero requests to `news.google.com/rss/search` (AI
queries), `/rss/articles` and `batchexecute` for AI items, with replacement
stubbed out.

### 6. Rollout: shadow is a separate process; delivery switches by `NEWS_AI_SOURCES`

Revision 3. Revision 2 ran the shadow inside the delivering process after the
markers. Review showed why that cannot be contained: `finish` prints the markers
but the process only exits after `run_deliver` returns, so a shadow that overruns
is killed by the scheduler, the run is classified `timeout` regardless of the
markers already printed, and this job's `retry_once` re-delivers the digest. And
no single deadline can be threaded through the existing stages — the agent's
timeout retry, precheck, paywall replacement and cross-dedup each own an
independent deadline and abandon worker threads. Containment by deadline is
therefore not achievable inside the delivering process; it is achieved by not
being in it.

**Cache isolation [R1].** The AI substage cache variant includes the source:
Google keeps today's `AI_SUBSTAGE_CACHE_VARIANT` unchanged (this release does not
invalidate today's cache); direct uses a distinct variant. Neither can replay the
other's entry. Test: pre-seed a Google entry at the identical date and range; a
direct build must ignore it and fetch.

**Delivery: `NEWS_AI_SOURCES=google|direct`**, read at call time. Only an
**unset** (or empty) variable means `google`. Any other value — a typo like
`direc` — must not silently become Google: the run raises the alert
`news_ai_sources_invalid` (detail: the value seen) and builds the AI section from
Google so the reader still gets a digest. The `[skill-status:]` marker is **not**
downgraded for it: `degraded` is `verified != 1`, which under `retry_once` would
re-deliver the whole digest (CLAUDE.md, scheduler contract §3); the alert is the
channel. Test: `NEWS_AI_SOURCES=direc` → one alert, Google section, marker `ok`.

The direct cache variant is used everywhere the AI substage touches its cache —
the read, both writes, and the Level-3 diagnostic lookup in
`summarize_default_ai_substaged` — not only on the read path.

- **`google`** — today's behaviour, byte-identical.
- **`direct`** — the delivered AI section reads only the direct feeds.
  **AI-wide outage [R7]:** if every configured feed fails, times out or filters to
  zero, or the config is missing or empty, the run raises the alert
  `ai_direct_feeds_empty` (detail: per-feed outcome) and the digest ships with
  `今日無相關新聞` in the AI section — the same contract as a tech/general section
  fallback: the reader still gets tech and 重大新聞, the operator is told. Never a
  silent fallback to Google: that would hide a dead feed list behind a working
  digest.

**Shadow: a new subcommand `news shadow-ai`, run by its own cron job.**

- It builds only the direct AI section, through the same substage / precheck /
  cross-dedup / theme code, writes the rendered section to
  `~/.nullclaw/.news-shadow/<YYYY-MM-DD>.md`, and never delivers. It takes no
  `--deliver-to`; there is no code path from this subcommand to Telegram delivery.
- **Containment [R3]** is structural. It is a different process under a different
  cron job, so its timeout, crash or exit code is classified against *its* job
  only. The delivering job's process, markers, retry and exit code are untouched
  because nothing of the shadow runs inside it. The shadow job is scheduled
  after the operator's job has finished (its last runs ended within ~70 s of
  08:30), on `repair_policy = none` — a retry buys nothing for a shadow — with its
  own timeout.
- **No alerts [R2].** The subcommand constructs its alert context in `silent`
  mode: `alert_failure` appends to the trace and returns; it never writes
  `news-failures.log` and never calls Telegram. Tests: forced Level-3 exhaustion
  under `shadow-ai` produces no Telegram request and no failure-log line.
- **Markers.** It exits through `claw_core::outcome::finish`, `ok` when a section
  was written, `degraded` when the section is empty or fell back, so the shadow
  job's own run record in `cron.db` says how each day went.
- **Traces [R9].** The whole process is shadow, so `log_trace` stamps
  `"ai_sources": "shadow"` on every event the process writes, worker threads and
  late abandoned work included — a process-wide flag set once at startup, before
  any thread exists, needs no propagation. Events from delivering runs carry no
  such field; their shape is unchanged. Each run also has its own
  `NULLCLAW_JOB_ID`, so `job_id` separates the two as well. Test: under
  `shadow-ai`, every emitted event — including one written from a spawned thread
  after the section returns — has the field.

### 7. What gets traced

`ai_direct_feeds` once per direct or shadow build:
`{feeds: [{name, outcome: ok|http_error|timeout|oversize|parse_error,
fetched, stale, undated, invalid_link, kept}], total_kept, elapsed_ms}`. A feed
that returned zero for a reason is distinguishable from one that is empty.

`ai_section_rendered` once per AI section in **every** mode — Google delivery,
direct delivery and shadow — so the gate compares like with like:
`{stories, links_total, links_google}`, computed on the final rendered AI section
(after cross-dedup and theming), not on `#N` markers, which
`attach_numbered_links` consumes.

`stories` is the number of lines that begin with `- ` or are exactly `-` (the
renderer's bodyless bullet, `render.rs::linked`) — one per story, by
construction of the renderer: theme headings (`▸ …`) and paywall continuation
lines (`　↳ …`, of which a paywall summary renders several) never begin with
`- `. A new helper `count_ai_stories`, **not** `theme::parse_ai_blocks`: that
parser is a pre-theming tool that rejects headings and consumes only one
continuation, so it returns `None` on exactly the themed and paywall-summary
sections the gate must count. Tests: plain section; themed section with two
headings; a paywall replacement (head + one `↳`); a paywall summary with three
`↳` lines; `今日無相關新聞` → 0. `links_total` / `links_google` count `[🔗](…)`
targets and those whose host is `news.google.com`.

### 8. Gate for switching delivery to `direct` [R8]

Five weekdays. Every quantity is computed from named records, not from the
shadow file alone:

- **Section filled.** Shadow `ai_section_rendered.stories ≥ 5` on each day. Over
  the same days the delivered Google run's `ai_section_rendered.stories` is read
  from the same event; if Google itself falls under 5, the operator decides
  whether 5 is the right floor.
- **Links.** Shadow `links_google = 0` on each day.
- **Feeds.** No configured feed has `kept = 0` with `outcome ≠ ok` on two
  consecutive days (`ai_direct_feeds`). A feed that is `ok` but empty is reported,
  not failed — a quiet lab blog is not an outage.
- **Shadow health.** The shadow job's own `cron_runs` rows: `exit_code = 0` and a
  `[skill-status:ok]` marker each day; any `timeout` or `exec_error` is
  investigated before the switch.
- **Containment.** Over the same days, the operator's delivery job's
  `cron_runs` show no `repair_action = retried*` and exactly one delivery per day;
  `news-failures.log` has no line written by a `shadow-ai` process (the silent
  sink makes this structural; the check confirms it).

Above the floor the decision is the operator's: is the direct section at least as
good to read as the Google one. No numeric quality score is invented for this.

Above the floor the decision is the operator's: is the direct section at least as
good to read as the Google one. No numeric quality score is invented for this.

## Not in this change

- Tech and 重大新聞 stay on Google News (decoded links from `f879be2` still help
  them). Candidates later: TechNews / iThome categories for tech, 中央社 RSS for
  重大新聞 — each its own decision.
- Custom topics (`--account-topics`) are free-text Google searches by nature.
- The paywall replacement search (§5).
- `~/b/ainews` (the weekly) also reads Google News RSS; the operator's repo, not
  touched here.

## Tests (written first)

Listed with their sections above; in summary: Atom + RSS parity; gzip stub;
Techmeme unwrap incl. malformed; title invariant with an inner ` - `; filter-then-cap
ordering; URL validation; concurrent fetch deadline; precheck makes zero decode
calls for direct links; cache variants isolated per mode; `direct` AI-wide empty
alerts and still ships the other sections; `google` mode byte-identical to today;
`shadow-ai` writes the section file, has no delivery path, never alerts (forced
Level-3 exhaustion → no Telegram request, no failure-log line), exits through
`finish` with the right marker, and stamps `ai_sources: shadow` on every event
including one written from a thread after the section returns;
`ai_section_rendered.stories` counts story blocks (paywall continuation = one
story) in every mode. All HOME-isolated; the real
`~/.nullclaw/skill-traces.jsonl` line count must not change across
`cargo test --workspace`.
