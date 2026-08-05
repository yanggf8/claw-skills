//! Nullclaw marker + status-contract goldens for cds-con.
//!
//! Plan decision 7: the status reports the **run**, not the **data**.
//! Stale data is still `ok` (and delivered). Zero usable observations, a
//! missing `kind`, or a store/read failure is `failed` (and does **not**
//! deliver). There is no `degraded` path for data quality.
//!
//! Marker shape follows chipcon: bare job id, deliver → status → trace,
//! markers only when a job id is set. Argument refusals follow oilcon's
//! argparse structure (exit 2 before any store work).

use cds_con::run::{deliver_options, run, Env, StoreAccess};
use credit_store::{ensure_schema, parse_series_list, upsert_credit_many};
use libsql::Connection;
use std::path::PathBuf;

const AS_OF: &str = "2026-07-31";
const JOB: &str = "job-77";

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

async fn mem() -> Connection {
    let db = libsql::Builder::new_local(":memory:").build().await.unwrap();
    let c = db.connect().unwrap();
    ensure_schema(&c).await.unwrap();
    c.execute(
        "CREATE TABLE IF NOT EXISTS config (
            key   TEXT NOT NULL PRIMARY KEY,
            value TEXT NOT NULL
        )",
        (),
    )
    .await
    .unwrap();
    c
}

async fn set_config(conn: &Connection, key: &str, value: &str) {
    conn.execute(
        "INSERT INTO config (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        libsql::params![key, value],
    )
    .await
    .unwrap();
}

async fn seed_rows(conn: &Connection, series: &str, rows: &[(&str, f64)]) {
    let owned: Vec<(String, f64)> = rows
        .iter()
        .map(|(d, v)| ((*d).to_string(), *v))
        .collect();
    upsert_credit_many(conn, series, &owned, &format!("fred:{series}"))
        .await
        .unwrap();
}

/// Two series with kinds and enough history to render.
const CDS_SERIES_OK: &str =
    "baa10y|BAA10Y|Moody's Baa-10y|spread;hy_oas|BAMLH0A0HYM2|ICE HY OAS|spread";

/// Same keys without kind — render must fail loudly (decision 6).
const CDS_SERIES_NO_KIND: &str = "baa10y|BAA10Y|Moody's Baa-10y;hy_oas|BAMLH0A0HYM2|ICE HY OAS";

/// Lead pair for `CDS_SERIES_OK`'s two series -- this crate does not
/// validate lead kinds against each other, so two spreads is fine for a
/// plumbing fixture. See `cds_message_lead` doc comments in `src/render.rs`.
const CDS_MESSAGE_LEAD_OK: &str = "baa10y|LEAD-BAA10Y;hy_oas|LEAD-HYOAS";

async fn seed_ok(conn: &Connection) {
    set_config(conn, "cds_series", CDS_SERIES_OK).await;
    set_config(conn, "cds_message_series", "baa10y,hy_oas").await;
    set_config(conn, "cds_message_lead", CDS_MESSAGE_LEAD_OK).await;
    // Daily spacing so frequency inference yields Daily.
    seed_rows(
        conn,
        "baa10y",
        &[
            ("2026-07-20", 1.50),
            ("2026-07-21", 1.55),
            ("2026-07-22", 1.52),
            ("2026-07-23", 1.58),
            ("2026-07-24", 1.59),
        ],
    )
    .await;
    seed_rows(
        conn,
        "hy_oas",
        &[
            ("2026-07-20", 2.70),
            ("2026-07-21", 2.75),
            ("2026-07-22", 2.72),
            ("2026-07-23", 2.80),
            ("2026-07-24", 2.79),
        ],
    )
    .await;
}

/// Stale data: latest is seven days before as_of. Still a successful run.
async fn seed_stale(conn: &Connection) {
    set_config(conn, "cds_series", CDS_SERIES_OK).await;
    set_config(conn, "cds_message_series", "baa10y,hy_oas").await;
    set_config(conn, "cds_message_lead", CDS_MESSAGE_LEAD_OK).await;
    seed_rows(
        conn,
        "baa10y",
        &[
            ("2026-07-20", 1.50),
            ("2026-07-21", 1.55),
            ("2026-07-22", 1.52),
            ("2026-07-23", 1.58),
            ("2026-07-24", 1.59),
        ],
    )
    .await;
    seed_rows(
        conn,
        "hy_oas",
        &[
            ("2026-07-20", 2.70),
            ("2026-07-21", 2.75),
            ("2026-07-22", 2.72),
            ("2026-07-23", 2.80),
            ("2026-07-24", 2.79),
        ],
    )
    .await;
}

fn env(job: Option<&str>) -> Env {
    Env {
        job_id: job.map(String::from),
        home: PathBuf::from("/tmp"),
    }
}

async fn go(
    argv: &[&str],
    job: Option<&str>,
    store: StoreAccess<'_>,
    as_of: &str,
) -> (i32, String, String) {
    let a: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    let (mut o, mut e) = (Vec::new(), Vec::new());
    let code = run(&a, &env(job), store, as_of, &mut o, &mut e).await;
    (
        code,
        String::from_utf8(o).unwrap(),
        String::from_utf8(e).unwrap(),
    )
}

// ---------------------------------------------------------------------------
// Decision 7 — status reports the run, not the data
// ---------------------------------------------------------------------------

#[tokio::test]
async fn store_unreachable_is_failed_and_does_not_deliver() {
    let (code, out, err) = go(
        &["cds-con"],
        Some(JOB),
        StoreAccess::Unreachable("turso unavailable - connection refused".into()),
        AS_OF,
    )
    .await;
    assert_eq!(code, 1, "hard failure exits non-zero");
    assert!(
        err.contains("CDS-CON failed:"),
        "stderr must name the skill: {err}"
    );
    assert!(
        err.contains("turso unavailable") || err.contains("unreachable") || err.contains("connection"),
        "stderr must carry the store error: {err}"
    );
    assert!(
        out.contains("[skill-status:failed]"),
        "unreachable store is failed: {out}"
    );
    assert!(out.contains("[trace:job-77]"), "{out}");
    assert!(
        !out.contains("💾 信用利差"),
        "failed path must not deliver a report body: {out}"
    );
    assert!(
        !out.contains("[skill-status:ok]"),
        "must not report ok: {out}"
    );
    assert!(
        !out.contains("[skill-status:degraded]"),
        "must not report degraded: {out}"
    );
}

#[tokio::test]
async fn store_read_failure_is_failed_and_does_not_deliver() {
    // Schema present but cds_series config missing → load fails.
    let conn = mem().await;
    let (code, out, err) = go(
        &["cds-con"],
        Some(JOB),
        StoreAccess::Ok(&conn),
        AS_OF,
    )
    .await;
    assert_eq!(code, 1);
    assert!(
        err.contains("CDS-CON failed:"),
        "stderr prefix required: {err}"
    );
    assert!(
        out.contains("[skill-status:failed]"),
        "a read/config failure is failed: {out}"
    );
    assert!(
        !out.contains("💾 信用利差"),
        "failed path must not deliver: {out}"
    );
}

#[tokio::test]
async fn empty_store_is_failed_and_does_not_deliver() {
    // Config lists series; store has zero observations for every one.
    let conn = mem().await;
    set_config(&conn, "cds_series", CDS_SERIES_OK).await;
    let (code, out, err) = go(
        &["cds-con"],
        Some(JOB),
        StoreAccess::Ok(&conn),
        AS_OF,
    )
    .await;
    assert_eq!(code, 1, "zero usable observations is a hard failure");
    assert!(
        err.contains("CDS-CON failed:"),
        "stderr must explain the failure: {err}"
    );
    assert!(
        out.contains("[skill-status:failed]"),
        "empty store must not leave the scheduler green: {out}"
    );
    assert!(
        !out.contains("💾 信用利差"),
        "must not deliver an all-n/a report: {out}"
    );
    assert!(
        !out.contains("[skill-status:ok]"),
        "never ok with nothing to say: {out}"
    );
}

#[tokio::test]
async fn every_configured_series_missing_is_failed() {
    // Config present; table has an unrelated series only — every configured
    // key has zero rows.
    let conn = mem().await;
    set_config(&conn, "cds_series", CDS_SERIES_OK).await;
    seed_rows(&conn, "unrelated", &[("2026-07-24", 1.0)]).await;
    let (code, out, _) = go(
        &["cds-con"],
        Some(JOB),
        StoreAccess::Ok(&conn),
        AS_OF,
    )
    .await;
    assert_eq!(code, 1);
    assert!(
        out.contains("[skill-status:failed]"),
        "all configured series missing is failed: {out}"
    );
    assert!(
        !out.contains("💾 信用利差"),
        "must not deliver: {out}"
    );
}

#[tokio::test]
async fn missing_kind_is_failed_and_does_not_deliver() {
    // Decision 6: three-field rows parse, but the family split cannot render.
    let conn = mem().await;
    set_config(&conn, "cds_series", CDS_SERIES_NO_KIND).await;
    // Set so this run reaches analyze()'s kind check (the thing under test)
    // rather than failing earlier on the now-mandatory message-series /
    // message-lead reads.
    set_config(&conn, "cds_message_series", "baa10y,hy_oas").await;
    set_config(&conn, "cds_message_lead", CDS_MESSAGE_LEAD_OK).await;
    seed_rows(
        &conn,
        "baa10y",
        &[
            ("2026-07-20", 1.50),
            ("2026-07-21", 1.55),
            ("2026-07-24", 1.59),
        ],
    )
    .await;
    // Confirm the fixture really has kind=None.
    let specs = parse_series_list(CDS_SERIES_NO_KIND).unwrap();
    assert!(specs.iter().all(|s| s.kind.is_none()));

    let (code, out, err) = go(
        &["cds-con"],
        Some(JOB),
        StoreAccess::Ok(&conn),
        AS_OF,
    )
    .await;
    assert_eq!(code, 1);
    assert!(
        err.contains("CDS-CON failed:"),
        "stderr must carry the render error: {err}"
    );
    assert!(
        err.contains("kind") || err.contains("no kind"),
        "error should mention missing kind: {err}"
    );
    assert!(
        out.contains("[skill-status:failed]"),
        "missing kind is failed: {out}"
    );
    assert!(
        !out.contains("💾 信用利差"),
        "must not deliver without a family split: {out}"
    );
}

#[tokio::test]
async fn missing_message_series_config_is_failed_and_does_not_deliver() {
    // cds_series present with enough history to render, but cds_message_series
    // is never set. This must fail loudly and by name -- v3 replaces the old
    // day-bound key with this one, and holds the same no-default standard.
    let conn = mem().await;
    set_config(&conn, "cds_series", CDS_SERIES_OK).await;
    // cds_message_series deliberately NOT set.
    seed_rows(
        &conn,
        "baa10y",
        &[
            ("2026-07-20", 1.50),
            ("2026-07-21", 1.55),
            ("2026-07-22", 1.52),
            ("2026-07-23", 1.58),
            ("2026-07-24", 1.59),
        ],
    )
    .await;
    seed_rows(
        &conn,
        "hy_oas",
        &[
            ("2026-07-20", 2.70),
            ("2026-07-21", 2.75),
            ("2026-07-22", 2.72),
            ("2026-07-23", 2.80),
            ("2026-07-24", 2.79),
        ],
    )
    .await;
    let (code, out, err) = go(
        &["cds-con"],
        Some(JOB),
        StoreAccess::Ok(&conn),
        AS_OF,
    )
    .await;
    assert_eq!(code, 1, "an absent cds_message_series key must fail the run");
    assert!(
        err.contains("CDS-CON failed:"),
        "stderr must carry the failure: {err}"
    );
    assert!(
        err.contains("cds_message_series"),
        "error must name the missing config key: {err}"
    );
    assert!(
        err.contains("missing config key"),
        "error must say the key is absent, not just 'not found': {err}"
    );
    assert!(
        out.contains("[skill-status:failed]"),
        "an absent message-series key is failed: {out}"
    );
    assert!(
        !out.contains("💾 信用利差"),
        "must not deliver without a resolvable message series list: {out}"
    );
}

#[tokio::test]
async fn cds_message_series_with_an_empty_token_is_failed_and_does_not_deliver() {
    // Present, but malformed (a doubled comma) -- a distinct situation from
    // "absent", and it must produce a distinct, actionable error rather than
    // silently dropping the empty token.
    let conn = mem().await;
    set_config(&conn, "cds_series", CDS_SERIES_OK).await;
    set_config(&conn, "cds_message_series", "baa10y,,hy_oas").await;
    seed_rows(
        &conn,
        "baa10y",
        &[
            ("2026-07-20", 1.50),
            ("2026-07-21", 1.55),
            ("2026-07-22", 1.52),
            ("2026-07-23", 1.58),
            ("2026-07-24", 1.59),
        ],
    )
    .await;
    seed_rows(
        &conn,
        "hy_oas",
        &[
            ("2026-07-20", 2.70),
            ("2026-07-21", 2.75),
            ("2026-07-22", 2.72),
            ("2026-07-23", 2.80),
            ("2026-07-24", 2.79),
        ],
    )
    .await;
    let (code, out, err) = go(
        &["cds-con"],
        Some(JOB),
        StoreAccess::Ok(&conn),
        AS_OF,
    )
    .await;
    assert_eq!(code, 1, "an empty token in the list must fail the run");
    assert!(
        err.contains("CDS-CON failed:"),
        "stderr must carry the failure: {err}"
    );
    assert!(
        err.contains("cds_message_series") && err.contains("empty"),
        "error must name the key and say a token is empty: {err}"
    );
    assert!(
        out.contains("[skill-status:failed]"),
        "a malformed message-series value is failed: {out}"
    );
    assert!(
        !out.contains("💾 信用利差"),
        "must not deliver with an unparseable message series list: {out}"
    );
}

#[tokio::test]
async fn cds_message_series_naming_an_unknown_series_is_failed_and_does_not_deliver() {
    // Parses fine, but names a series that is not in cds_series at all -- must
    // fail loudly and name the offending key, never be silently skipped.
    let conn = mem().await;
    set_config(&conn, "cds_series", CDS_SERIES_OK).await;
    set_config(&conn, "cds_message_series", "baa10y,doesnotexist").await;
    // Set so this run reaches select_message_series's check (the thing under
    // test) rather than failing earlier on the now-mandatory message-lead
    // read.
    set_config(&conn, "cds_message_lead", CDS_MESSAGE_LEAD_OK).await;
    seed_rows(
        &conn,
        "baa10y",
        &[
            ("2026-07-20", 1.50),
            ("2026-07-21", 1.55),
            ("2026-07-22", 1.52),
            ("2026-07-23", 1.58),
            ("2026-07-24", 1.59),
        ],
    )
    .await;
    seed_rows(
        &conn,
        "hy_oas",
        &[
            ("2026-07-20", 2.70),
            ("2026-07-21", 2.75),
            ("2026-07-22", 2.72),
            ("2026-07-23", 2.80),
            ("2026-07-24", 2.79),
        ],
    )
    .await;
    let (code, out, err) = go(
        &["cds-con"],
        Some(JOB),
        StoreAccess::Ok(&conn),
        AS_OF,
    )
    .await;
    assert_eq!(code, 1, "a key naming an unknown series must fail the run");
    assert!(
        err.contains("CDS-CON failed:"),
        "stderr must carry the failure: {err}"
    );
    assert!(
        err.contains("doesnotexist"),
        "error must name the unknown key itself: {err}"
    );
    assert!(
        out.contains("[skill-status:failed]"),
        "an unknown series name is failed: {out}"
    );
    assert!(
        !out.contains("💾 信用利差"),
        "must not deliver when a configured message key does not resolve: {out}"
    );
}

// ---------------------------------------------------------------------------
// cds_message_lead — the opening pair (2026-08-04 lead-block redesign).
// Same no-default standard as cds_message_series: absent, unreadable, and
// unparseable are three distinct situations, and a lead key that does not
// resolve inside cds_message_series is a fourth. Each fails loudly and by
// name, never silently.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn missing_message_lead_config_is_failed_and_does_not_deliver() {
    // cds_series and cds_message_series both present and enough history to
    // render, but cds_message_lead is never set.
    let conn = mem().await;
    set_config(&conn, "cds_series", CDS_SERIES_OK).await;
    set_config(&conn, "cds_message_series", "baa10y,hy_oas").await;
    // cds_message_lead deliberately NOT set.
    seed_rows(
        &conn,
        "baa10y",
        &[
            ("2026-07-20", 1.50),
            ("2026-07-24", 1.59),
        ],
    )
    .await;
    seed_rows(
        &conn,
        "hy_oas",
        &[
            ("2026-07-20", 2.70),
            ("2026-07-24", 2.79),
        ],
    )
    .await;
    let (code, out, err) = go(&["cds-con"], Some(JOB), StoreAccess::Ok(&conn), AS_OF).await;
    assert_eq!(code, 1, "an absent cds_message_lead key must fail the run");
    assert!(
        err.contains("CDS-CON failed:"),
        "stderr must carry the failure: {err}"
    );
    assert!(
        err.contains("cds_message_lead"),
        "error must name the missing config key: {err}"
    );
    assert!(
        err.contains("missing config key"),
        "error must say the key is absent, not just 'not found': {err}"
    );
    assert!(
        out.contains("[skill-status:failed]"),
        "an absent message-lead key is failed: {out}"
    );
    assert!(
        !out.contains("💾 信用利差"),
        "must not deliver without a resolvable lead pair: {out}"
    );
}

#[tokio::test]
async fn cds_message_lead_with_wrong_entry_count_is_failed_and_does_not_deliver() {
    // Present, but malformed -- only one entry, not the required pair.
    let conn = mem().await;
    set_config(&conn, "cds_series", CDS_SERIES_OK).await;
    set_config(&conn, "cds_message_series", "baa10y,hy_oas").await;
    set_config(&conn, "cds_message_lead", "baa10y|扣掉利率").await;
    seed_rows(&conn, "baa10y", &[("2026-07-20", 1.50), ("2026-07-24", 1.59)]).await;
    seed_rows(&conn, "hy_oas", &[("2026-07-20", 2.70), ("2026-07-24", 2.79)]).await;
    let (code, out, err) = go(&["cds-con"], Some(JOB), StoreAccess::Ok(&conn), AS_OF).await;
    assert_eq!(code, 1, "a lead value naming only one series must fail the run");
    assert!(
        err.contains("CDS-CON failed:"),
        "stderr must carry the failure: {err}"
    );
    assert!(
        err.contains("cds_message_lead") && err.contains("exactly two"),
        "error must name the key and say a pair is required: {err}"
    );
    assert!(
        out.contains("[skill-status:failed]"),
        "a malformed message-lead value is failed: {out}"
    );
    assert!(
        !out.contains("💾 信用利差"),
        "must not deliver with an unparseable lead value: {out}"
    );
}

#[tokio::test]
async fn cds_message_lead_naming_a_series_absent_from_message_series_is_failed_and_does_not_deliver()
{
    // Parses fine and names two real cds_series keys, but one of them is not
    // in cds_message_series -- resolve_lead must catch this, not just parse.
    let conn = mem().await;
    set_config(&conn, "cds_series", CDS_SERIES_OK).await;
    set_config(&conn, "cds_message_series", "baa10y").await;
    set_config(&conn, "cds_message_lead", "baa10y|扣掉利率;hy_oas|沒扣").await;
    seed_rows(&conn, "baa10y", &[("2026-07-20", 1.50), ("2026-07-24", 1.59)]).await;
    seed_rows(&conn, "hy_oas", &[("2026-07-20", 2.70), ("2026-07-24", 2.79)]).await;
    let (code, out, err) = go(&["cds-con"], Some(JOB), StoreAccess::Ok(&conn), AS_OF).await;
    assert_eq!(
        code, 1,
        "a lead key absent from cds_message_series must fail the run"
    );
    assert!(
        err.contains("CDS-CON failed:"),
        "stderr must carry the failure: {err}"
    );
    assert!(
        err.contains("hy_oas"),
        "error must name the unresolved lead key itself: {err}"
    );
    assert!(
        out.contains("[skill-status:failed]"),
        "a lead key outside cds_message_series is failed: {out}"
    );
    assert!(
        !out.contains("💾 信用利差"),
        "must not deliver when a lead key does not resolve: {out}"
    );
}

#[tokio::test]
async fn stale_data_is_ok_and_delivers() {
    // Seven days old is a fact about the data, not a run failure.
    let conn = mem().await;
    seed_stale(&conn).await;
    let (code, out, err) = go(
        &["cds-con"],
        Some(JOB),
        StoreAccess::Ok(&conn),
        AS_OF,
    )
    .await;
    assert_eq!(code, 0, "stale data is still a successful run");
    assert!(err.is_empty(), "stderr quiet on success: {err}");
    assert!(
        out.contains("[skill-status:ok]"),
        "stale data must report ok, never degraded: {out}"
    );
    assert!(
        !out.contains("[skill-status:degraded]"),
        "never degraded for data age: {out}"
    );
    assert!(
        out.contains("💾 信用利差"),
        "must deliver the report: {out}"
    );
    assert!(
        out.contains("7 天前") || out.contains("資料:"),
        "body must state how old the data is: {out}"
    );
}

#[tokio::test]
async fn partial_series_missing_is_ok_and_delivers() {
    // One series present, one missing → usable result; body names the gap.
    let conn = mem().await;
    set_config(&conn, "cds_series", CDS_SERIES_OK).await;
    set_config(&conn, "cds_message_series", "baa10y,hy_oas").await;
    set_config(&conn, "cds_message_lead", CDS_MESSAGE_LEAD_OK).await;
    seed_rows(
        &conn,
        "baa10y",
        &[
            ("2026-07-20", 1.50),
            ("2026-07-21", 1.55),
            ("2026-07-22", 1.52),
            ("2026-07-23", 1.58),
            ("2026-07-24", 1.59),
        ],
    )
    .await;
    // hy_oas deliberately not seeded.
    let (code, out, err) = go(
        &["cds-con"],
        Some(JOB),
        StoreAccess::Ok(&conn),
        AS_OF,
    )
    .await;
    assert_eq!(code, 0);
    assert!(err.is_empty(), "{err}");
    assert!(
        out.contains("[skill-status:ok]"),
        "partial data is ok: {out}"
    );
    assert!(out.contains("💾 信用利差"), "must deliver: {out}");
    assert!(
        out.contains("hy_oas") && out.contains("n/a"),
        "missing series named as n/a: {out}"
    );
    assert!(
        out.contains("缺") || out.contains("hy_oas"),
        "body must surface which series is missing: {out}"
    );
}

// ---------------------------------------------------------------------------
// Markers and delivery order
// ---------------------------------------------------------------------------

#[tokio::test]
async fn deliver_then_status_then_trace_in_that_order() {
    let conn = mem().await;
    seed_ok(&conn).await;
    let (code, out, err) = go(
        &["cds-con"],
        Some(JOB),
        StoreAccess::Ok(&conn),
        AS_OF,
    )
    .await;
    assert_eq!(code, 0);
    assert!(err.is_empty(), "{err}");
    let body = out.find("💾 信用利差").expect("body missing");
    let status = out
        .find("[skill-status:ok]")
        .expect("status marker missing");
    let trace = out.find("[trace:job-77]").expect("trace marker missing");
    assert!(body < status, "body must precede the markers: {out}");
    assert!(status < trace, "skill-status must precede trace: {out}");
}

#[tokio::test]
async fn the_job_id_is_appended_bare_not_backticked() {
    // oilcon wraps in backticks; chipcon/inflation-con/cds-con append bare.
    let conn = mem().await;
    seed_ok(&conn).await;
    let (_, out, _) = go(
        &["cds-con"],
        Some(JOB),
        StoreAccess::Ok(&conn),
        AS_OF,
    )
    .await;
    assert!(
        out.contains("\n\njob-77"),
        "job id must be appended bare: {out}"
    );
    assert!(
        !out.contains("`job-77`"),
        "cds-con must not wrap the job id in backticks: {out}"
    );
}

#[tokio::test]
async fn no_job_id_means_no_markers_at_all() {
    let conn = mem().await;
    seed_ok(&conn).await;
    let (code, out, _) = go(&["cds-con"], None, StoreAccess::Ok(&conn), AS_OF).await;
    assert_eq!(code, 0);
    assert!(out.contains("💾 信用利差"), "the report itself still prints");
    assert!(
        !out.contains("[skill-status:"),
        "no status marker without a job id: {out}"
    );
    assert!(
        !out.contains("[trace:"),
        "no trace marker without a job id: {out}"
    );
}

#[tokio::test]
async fn without_deliver_to_the_body_still_reaches_stdout() {
    // deliver_or_fail(None, …) echoes to stdout.
    let conn = mem().await;
    seed_ok(&conn).await;
    let (code, out, _) = go(
        &["cds-con"],
        Some(JOB),
        StoreAccess::Ok(&conn),
        AS_OF,
    )
    .await;
    assert_eq!(code, 0);
    assert!(
        out.contains("SIGNAL-ONLY"),
        "the full report must be on stdout: {out}"
    );
}

#[test]
fn parse_mode_is_html_because_the_body_now_carries_the_evidence_blockquote() {
    // Replaces `parse_mode_is_none_because_the_message_has_no_markdown`,
    // which pinned a real decision that has since changed for a stated
    // reason -- rewritten, not deleted (the old test's premise, "this
    // message has no Markdown markup", stopped being true).
    //
    // claw-core DeliverOptions::default() is Some("Markdown"). Until
    // 2026-08-05 cds-con pinned `None` because the body was plain text with
    // no markup of any kind. It now wraps the 佐證 (evidence) section in a
    // literal `<blockquote expandable>` tag (owner-approved, collapses the
    // section behind a tap on Telegram) — so `None` would deliver those
    // tags to the reader as visible text instead of collapsing anything.
    // `Some("HTML")` is required for the SAME reason `None` used to be:
    // the parse_mode must match what the body actually contains.
    let opts = deliver_options("main");
    assert_eq!(
        opts.parse_mode,
        Some("HTML".to_string()),
        "cds-con must set parse_mode: Some(\"HTML\") now that the body carries the evidence blockquote"
    );
}

// ---------------------------------------------------------------------------
// Delivery representations (2026-08-05 collapsible evidence): stdout must
// always carry the plain, unmarked-up body -- the html payload (with its
// literal <blockquote expandable> tag) is for Telegram only, never for a
// human/agent reading the run's own stdout.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn stdout_never_carries_html_markup() {
    // deliver_to is not set (matches every other test in this file), so
    // `emit` takes the no-chat-id path -- exactly the path whose whole job
    // is to salvage readable content onto stdout. It must salvage the PLAIN
    // body, never the html one.
    let conn = mem().await;
    seed_ok(&conn).await;
    let (code, out, err) = go(&["cds-con"], Some(JOB), StoreAccess::Ok(&conn), AS_OF).await;
    assert_eq!(code, 0, "{err}");
    for banned in ["<blockquote", "</blockquote>", "&lt;", "&gt;", "&amp;"] {
        assert!(
            !out.contains(banned),
            "stdout must carry no HTML markup or escape sequence (found '{banned}'): {out}"
        );
    }
    assert!(out.contains("💾 信用利差"), "the plain report must still print: {out}");
}

// ---------------------------------------------------------------------------
// Argument refusals — exit 2 before any store work
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_unknown_flag_exits_two_without_touching_the_store() {
    // StoreAccess::Unreachable would fire if we fell through past parse.
    // A bad arg must refuse first and never reach store logic.
    let (code, out, err) = go(
        &["cds-con", "--nosuchflag"],
        Some(JOB),
        StoreAccess::Unreachable("STORE WAS REACHED".into()),
        AS_OF,
    )
    .await;
    assert_eq!(code, 2, "argparse exits 2 on an unrecognized argument");
    assert!(
        err.contains("unrecognized arguments: --nosuchflag"),
        "the refusal must name the offending flag: {err}"
    );
    assert!(out.is_empty(), "nothing is delivered or emitted: {out}");
    assert!(
        !err.contains("STORE WAS REACHED"),
        "a bad argument must never reach the store: {err}"
    );
    assert!(
        !err.contains("CDS-CON failed"),
        "must not enter the failed-store path: {err}"
    );
}

#[tokio::test]
async fn a_mistyped_deliver_to_is_refused_rather_than_silently_printing() {
    let conn = mem().await;
    seed_ok(&conn).await;
    let (code, out, err) = go(
        &["cds-con", "--deliverto", "7972814626"],
        Some(JOB),
        StoreAccess::Ok(&conn),
        AS_OF,
    )
    .await;
    assert_eq!(code, 2);
    assert!(
        err.contains("unrecognized arguments: --deliverto"),
        "{err}"
    );
    assert!(
        !out.contains("💾 信用利差"),
        "a refused run must not render the report: {out}"
    );
}

#[tokio::test]
async fn an_invalid_mode_value_exits_two() {
    let (code, out, err) = go(
        &["cds-con", "--mode", "recrod"],
        Some(JOB),
        StoreAccess::Unreachable("STORE WAS REACHED".into()),
        AS_OF,
    )
    .await;
    assert_eq!(code, 2, "argparse exits 2 on an invalid choice");
    assert!(
        err.contains("invalid choice: 'recrod'"),
        "the refusal must name the rejected value: {err}"
    );
    assert!(
        !out.contains("[skill-status:"),
        "a rejected run must not report a status at all: {out}"
    );
    assert!(
        !err.contains("STORE WAS REACHED"),
        "invalid mode must not reach the store: {err}"
    );
}

#[tokio::test]
async fn a_flag_missing_its_value_exits_two() {
    let (code, out, err) = go(
        &["cds-con", "--mode"],
        Some(JOB),
        StoreAccess::Unreachable("STORE WAS REACHED".into()),
        AS_OF,
    )
    .await;
    assert_eq!(code, 2);
    assert!(
        err.contains("argument --mode: expected one argument"),
        "{err}"
    );
    assert!(out.is_empty(), "{out}");
    assert!(
        !err.contains("STORE WAS REACHED"),
        "missing value must not reach the store: {err}"
    );
}

#[tokio::test]
async fn deliver_mode_is_accepted() {
    let conn = mem().await;
    seed_ok(&conn).await;
    let (code, out, err) = go(
        &["cds-con", "--mode", "deliver"],
        Some(JOB),
        StoreAccess::Ok(&conn),
        AS_OF,
    )
    .await;
    assert_eq!(code, 0, "--mode deliver must be accepted: {err}");
    assert!(out.contains("[skill-status:ok]"), "{out}");
}

// ---------------------------------------------------------------------------
// Monthly-inferred frequency, end to end — closes the gap where `run.rs`'s
// `infer_frequency` (the only production code that ever returns
// `Frequency::Monthly`) went untested. v3 removed the daily/monthly display
// split entirely (design doc §「v3 decisions」2): a series inferred Monthly
// still renders every day like any other, as long as `cds_message_series`
// names it -- what `infer_frequency` still controls is only which bucket
// (`日 至`/`月 至`) it lands in on the freshness line.
// ---------------------------------------------------------------------------

/// Two daily series plus a third ("aaa") whose observations are spaced
/// ~30 days apart, so `infer_frequency`'s median-gap-≥-20-days rule must
/// classify it Monthly by itself -- nothing in this fixture declares its
/// frequency.
const CDS_SERIES_WITH_MONTHLY: &str =
    "baa10y|BAA10Y|Moody's Baa-10y|spread;hy_oas|BAMLH0A0HYM2|ICE HY OAS|spread;aaa|AAA|Moody's Aaa|yield";

async fn seed_daily_and_monthly(conn: &Connection) {
    set_config(conn, "cds_series", CDS_SERIES_WITH_MONTHLY).await;
    set_config(conn, "cds_message_series", "baa10y,hy_oas,aaa").await;
    // aaa (the sole yield here) leads, so the two daily spreads left in 佐證
    // are single-kind.
    set_config(conn, "cds_message_lead", "baa10y|LEAD-BAA10Y;aaa|LEAD-AAA").await;
    seed_rows(
        conn,
        "baa10y",
        &[
            ("2026-07-20", 1.50),
            ("2026-07-21", 1.55),
            ("2026-07-22", 1.52),
            ("2026-07-23", 1.58),
            ("2026-07-24", 1.59),
        ],
    )
    .await;
    seed_rows(
        conn,
        "hy_oas",
        &[
            ("2026-07-20", 2.70),
            ("2026-07-21", 2.75),
            ("2026-07-22", 2.72),
            ("2026-07-23", 2.80),
            ("2026-07-24", 2.79),
        ],
    )
    .await;
    // ~30-day gaps: 04-01→05-01 is 30 days, 05-01→06-01 is 31 -- both well
    // clear of the 20-day median threshold in `infer_frequency`.
    seed_rows(
        conn,
        "aaa",
        &[
            ("2026-04-01", 5.30),
            ("2026-05-01", 5.35),
            ("2026-06-01", 5.40),
        ],
    )
    .await;
}

#[tokio::test]
async fn monthly_series_inferred_from_gaps_reaches_the_month_freshness_line() {
    let conn = mem().await;
    seed_daily_and_monthly(&conn).await;
    let (code, out, err) = go(
        &["cds-con"],
        Some(JOB),
        StoreAccess::Ok(&conn),
        AS_OF,
    )
    .await;
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains("LEAD-AAA"),
        "a series inferred Monthly purely from its ~30-day gaps renders like \
         any other configured series -- there is no day-of-month gate: {out}"
    );
    assert!(
        out.contains("月 至 2026-06"),
        "infer_frequency's Monthly classification must still land it in the \
         monthly freshness bucket, not the daily one: {out}"
    );
    // The daily series render alongside it, unaffected: baa10y (lead
    // override label) and hy_oas (its own cds_series label "ICE HY OAS", in
    // 佐證).
    assert!(out.contains("LEAD-BAA10Y") && out.contains("ICE HY OAS"), "{out}");
}
