//! Mode dispatch, delivery, and nullclaw marker contract for cds-con.
//!
//! No Python original. The contract is plan decision 7 (status reports the
//! **run**, not the data) plus `tools/install-skill.sh`'s argparse refusals
//! (exit 2 before any store work). Marker shape follows chipcon: bare job id,
//! deliver → emit_skill_status → emit_trace, markers only when a job id is set.
//!
//! Writers / Env / store access / clock are injected so every branch is
//! testable without network, Telegram, or a live Turso.

use std::io::Write;
use std::path::PathBuf;

use claw_core::delivery::{deliver, DeliverOptions, DeliveryOutcome};
use credit_store::{parse_series_list, read_credit_history, Observation};
use libsql::Connection;

use crate::render::{
    escape_html, format_message_variants, parse_message_lead, parse_message_series, Frequency,
    LeadEntry, MessageVariants, SeriesInput,
};

/// Injected environment: job id (`NULLCLAW_JOB_ID`) and HOME for paths.
#[derive(Debug, Clone)]
pub struct Env {
    pub job_id: Option<String>,
    pub home: PathBuf,
}

/// How the run reaches the store. `Unreachable` is the connect-failure path
/// from `main`; `Ok` is a live (or in-memory) connection.
pub enum StoreAccess<'a> {
    Ok(&'a Connection),
    Unreachable(String),
}

struct Args {
    mode: String,
    deliver_to: Option<String>,
    account: String,
}

/// Parse argv the way argparse does, **including its refusals**.
///
/// Unknown flag, invalid `--mode`, and a flag missing its value all return
/// `Err` so the caller can exit 2 before any store work. There is no Python
/// original here; the contract is `tools/install-skill.sh`'s smoke probe
/// (requires exit 2) and the Phase ③ lesson of 2026-07-31.
///
/// cds-con is deliver-only: fetch lives in `price cds fetch`. The only valid
/// mode is `deliver` (also the default).
fn parse_args(argv: &[String]) -> Result<Args, String> {
    const MODES: [&str; 1] = ["deliver"];

    let mut mode = "deliver".to_string();
    let mut deliver_to: Option<String> = None;
    let mut account = "main".to_string();

    // Skip program name when present (first element that does not look like a flag).
    let mut args = argv;
    if let Some(first) = argv.first() {
        if !first.starts_with('-') {
            args = &argv[1..];
        }
    }

    // Consume the value belonging to `flag`, or refuse the way argparse does.
    fn value_for(args: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
        *i += 1;
        args.get(*i)
            .cloned()
            .ok_or_else(|| format!("cds-con: error: argument {flag}: expected one argument"))
    }

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--mode" => mode = value_for(args, &mut i, "--mode")?,
            "--deliver-to" => deliver_to = Some(value_for(args, &mut i, "--deliver-to")?),
            "--account" => account = value_for(args, &mut i, "--account")?,
            other => {
                return Err(format!("cds-con: error: unrecognized arguments: {other}"));
            }
        }
        i += 1;
    }

    if !MODES.contains(&mode.as_str()) {
        return Err(format!(
            "cds-con: error: argument --mode: invalid choice: '{mode}' (choose from 'deliver')"
        ));
    }

    Ok(Args {
        mode,
        deliver_to,
        account,
    })
}

/// Emit `[skill-status:<status>]` only when job_id is set (manual runs stay clean).
fn emit_skill_status(status: &str, env: &Env, out: &mut dyn Write) {
    if env.job_id.is_none() {
        return;
    }
    let _ = writeln!(out, "[skill-status:{status}]");
    let _ = out.flush();
}

/// Emit `[trace:<job_id>]` only when job_id is set.
fn emit_trace(env: &Env, out: &mut dyn Write) {
    let Some(ref id) = env.job_id else {
        return;
    };
    let _ = writeln!(out, "[trace:{id}]");
    let _ = out.flush();
}

/// Deliver options for cds-con.
///
/// `claw-core`'s default is `Some("Markdown")`. Until 2026-08-05 this crate
/// pinned `parse_mode: None` deliberately, because the body carried no
/// markup at all. That changed: the body now wraps its 佐證 (evidence)
/// section in a literal `<blockquote expandable>` tag (owner-approved
/// collapsible section, see `render.rs`'s module doc comment) — so `None`
/// would deliver those tags to the reader as visible text instead of
/// collapsing anything. `Some("HTML")` is the correct pin now, for the
/// opposite reason `None` was correct before: the decision follows the
/// body's actual markup, not a fixed per-skill default. (oilcon already
/// pins a non-`None` `parse_mode` for its own reason — this is a per-skill
/// choice, not a project-wide invariant.)
pub fn deliver_options(account: &str) -> DeliverOptions {
    DeliverOptions {
        account: account.to_string(),
        parse_mode: Some("HTML".to_string()),
        ..Default::default()
    }
}

/// Append the job id to `body`, escaping it with `escape` -- bare
/// (`|s| s.to_string()`) for the plain representation, [`escape_html`] for
/// the HTML one. Pure and side-effect-free so the escaping decision is
/// directly unit-testable without a live delivery path (see this module's
/// `tests`).
fn append_job_id(body: &str, job_id: Option<&str>, escape: impl Fn(&str) -> String) -> String {
    match job_id {
        Some(id) => format!("{body}\n\n{}", escape(id)),
        None => body.to_string(),
    }
}

/// Deliver then markers. Job id is appended bare (not backticks — oilcon differs).
/// On hard delivery failure returns 1 without markers.
///
/// `message` is the [`MessageVariants`] pair [`format_message_variants`]
/// built from the SAME [`render_parts`](crate::render::render_parts) call —
/// `message.html` is what actually goes to Telegram (`deliver_options` now
/// pins `parse_mode: HTML`); `message.plain` is what this function
/// guarantees reaches stdout on every path that prints anything at all,
/// INCLUDING delivery failure. That guarantee is deliberate: `claw-core::deliver`
/// itself writes its `body` argument to stdout as the fallback on the
/// no-chat-id and delivery-failure paths, and if that argument were the
/// HTML body, a raw `<blockquote expandable>` tag would leak onto stdout
/// right when the point is to salvage readable content for the
/// human/agent watching the run. So the HTML body is handed to `deliver`
/// only to reach the wire (or be discarded); whatever `deliver` itself
/// wrote to its own output buffer is never forwarded — this function
/// writes its own plain copy instead. (Bundled into one struct rather than
/// two `&str` parameters to keep this function's arity sane.)
fn emit(
    message: &MessageVariants,
    status: &str,
    deliver_to: Option<&str>,
    account: &str,
    env: &Env,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let plain_output = append_job_id(&message.plain, env.job_id.as_deref(), |s| s.to_string());
    let html_output = append_job_id(&message.html, env.job_id.as_deref(), escape_html);

    let opts = deliver_options(account);
    // claw-core::deliver takes `impl Write` (Sized). Buffer through Vec so we
    // can still accept `&mut dyn Write` on the run seam. `o_buf` is a
    // throwaway: see this function's doc comment for why its content (a
    // copy of `html_output`, on the no-chat-id/failure paths) is discarded
    // rather than forwarded to `out`.
    let (mut o_buf, mut e_buf) = (Vec::new(), Vec::new());
    let outcome = deliver(deliver_to, &html_output, &opts, &mut o_buf, &mut e_buf);
    let _ = err.write_all(&e_buf);
    let _ = err.flush();
    match outcome {
        // Sent over the wire only — claw-core wrote nothing to o_buf, so
        // there is nothing to salvage or suppress.
        DeliveryOutcome::Sent => {}
        DeliveryOutcome::PrintedToStdout
        | DeliveryOutcome::FailedFatal
        | DeliveryOutcome::FailedSoft => {
            let _ = writeln!(out, "{plain_output}");
            let _ = out.flush();
        }
    }
    if outcome == DeliveryOutcome::FailedFatal {
        return 1;
    }
    // Order is load-bearing: deliver → status → trace.
    emit_skill_status(status, env, out);
    emit_trace(env, out);
    0
}

/// Hard-failure path: no delivery. Markers only when a job id is set.
/// Decision 7 / CLAUDE.md option A: when there is no usable result, emit
/// `failed` and send nothing so a retry is the only delivery.
fn finish_failed(message: &str, env: &Env, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let _ = writeln!(err, "CDS-CON failed: {message}");
    let _ = err.flush();
    emit_skill_status("failed", env, out);
    emit_trace(env, out);
    1
}

/// Read `cds_series` from the registry `config` table and load each series'
/// history into [`SeriesInput`]s. Frequency is inferred from observation gaps
/// (median ≥ 20 days → monthly); empty series default to daily.
async fn load_series(conn: &Connection) -> Result<Vec<SeriesInput>, String> {
    let raw = read_config(conn, "cds_series")
        .await?
        .ok_or_else(|| {
            "missing config key 'cds_series' in price-registry; set it with: price config set cds_series <value>"
                .to_string()
        })?;
    let specs = parse_series_list(&raw).map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(specs.len());
    for spec in specs {
        let history = read_credit_history(conn, &spec.key)
            .await
            .map_err(|e| e.message().to_string())?;
        let frequency = infer_frequency(&history);
        let rows: Vec<Observation> = history
            .into_iter()
            .map(|(date, value)| Observation { date, value })
            .collect();
        out.push(SeriesInput {
            spec,
            rows,
            frequency,
        });
    }
    Ok(out)
}

/// Which series the daily message shows, and in what order. Config, not code
/// -- same pattern as a missing `cds_series` or a missing `kind` (decision
/// 6): an absent key, a failed read, and an unparseable value are three
/// distinct situations, and each fails the run loudly and by name. No
/// default value, and no hardcoded series list, anywhere in Rust. Whether
/// each named key actually exists among the configured series is checked
/// later, by `select_message_series` once the series are loaded and derived
/// rows (like `baa−aaa`) exist to be named.
async fn message_series_keys(conn: &Connection) -> Result<Vec<String>, String> {
    let raw = read_config(conn, "cds_message_series")
        .await
        .map_err(|e| {
            format!(
                "could not read config key 'cds_message_series' from price-registry: {e}; set it with: price config set cds_message_series <comma-separated keys>"
            )
        })?
        .ok_or_else(|| {
            "missing config key 'cds_message_series' in price-registry; set it with: price config set cds_message_series <comma-separated keys>"
                .to_string()
        })?;
    parse_message_series(&raw).map_err(|e| {
        format!(
            "{e}; set it with: price config set cds_message_series <comma-separated keys>"
        )
    })
}

/// `cds_message_lead`: the opening pair (the lead-block redesign, item 1/3
/// of `docs/specs/2026-08-04-cds-con-readability-v2-design.md`). Same
/// no-default standard as `cds_message_series`: an absent key, a failed
/// read, and an unparseable value are three distinct situations, each named
/// and each failing the run loudly. Whether each lead key actually resolves
/// inside `cds_message_series` is checked later, by `resolve_lead` once the
/// message series are selected.
async fn message_lead_keys(conn: &Connection) -> Result<Vec<LeadEntry>, String> {
    let raw = read_config(conn, "cds_message_lead")
        .await
        .map_err(|e| {
            format!(
                "could not read config key 'cds_message_lead' from price-registry: {e}; set it with: price config set cds_message_lead <key|Label;key|Label>"
            )
        })?
        .ok_or_else(|| {
            "missing config key 'cds_message_lead' in price-registry; set it with: price config set cds_message_lead <key|Label;key|Label>"
                .to_string()
        })?;
    parse_message_lead(&raw).map_err(|e| {
        format!("{e}; set it with: price config set cds_message_lead <key|Label;key|Label>")
    })
}

async fn read_config(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    let mut rows = conn
        .query(
            "SELECT value FROM config WHERE key = ?",
            libsql::params![key],
        )
        .await
        .map_err(|e| e.to_string())?;
    match rows.next().await.map_err(|e| e.to_string())? {
        Some(r) => Ok(Some(r.get::<String>(0).map_err(|e| e.to_string())?)),
        None => Ok(None),
    }
}

/// Infer publication frequency from consecutive observation gaps.
/// Median gap ≥ 20 calendar days → monthly; otherwise daily. Empty or
/// single-point series default to daily (no gap to measure).
fn infer_frequency(rows: &[(String, f64)]) -> Frequency {
    if rows.len() < 2 {
        return Frequency::Daily;
    }
    let mut gaps: Vec<i64> = Vec::new();
    for w in rows.windows(2).take(32) {
        if let (Some(a), Some(b)) = (parse_ymd(&w[0].0), parse_ymd(&w[1].0)) {
            let g = (date_to_rata_die(b.0, b.1, b.2) - date_to_rata_die(a.0, a.1, a.2)).abs();
            if g > 0 {
                gaps.push(g);
            }
        }
    }
    if gaps.is_empty() {
        return Frequency::Daily;
    }
    gaps.sort_unstable();
    let median = gaps[gaps.len() / 2];
    if median >= 20 {
        Frequency::Monthly
    } else {
        Frequency::Daily
    }
}

fn parse_ymd(s: &str) -> Option<(i32, u32, u32)> {
    if s.len() < 10 {
        return None;
    }
    let y: i32 = s[0..4].parse().ok()?;
    let m: u32 = s[5..7].parse().ok()?;
    let d: u32 = s[8..10].parse().ok()?;
    Some((y, m, d))
}

fn date_to_rata_die(y: i32, m: u32, d: u32) -> i64 {
    let y = y as i64;
    let m = m as i64;
    let d = d as i64;
    let (y, m) = if m <= 2 {
        (y - 1, m + 9)
    } else {
        (y, m - 3)
    };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * m + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 306
}

/// Core entry: parse, load, render, deliver, markers.
///
/// Returns the process exit code. Does **not** call `process::exit`.
///
/// - `store` — live connection or an unreachable diagnostic
/// - `as_of` — injected YYYY-MM-DD for age-in-days (no wall clock inside run)
pub async fn run(
    argv: &[String],
    env: &Env,
    store: StoreAccess<'_>,
    as_of: &str,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    // argparse refuses before any work: a bad argument must not reach the store.
    let args = match parse_args(argv) {
        Ok(a) => a,
        Err(msg) => {
            let _ = writeln!(err, "{msg}");
            let _ = err.flush();
            return 2;
        }
    };

    // Currently deliver-only; parse_args already rejected other modes.
    let _ = args.mode;

    let conn = match store {
        StoreAccess::Ok(c) => c,
        StoreAccess::Unreachable(msg) => {
            return finish_failed(&msg, env, out, err);
        }
    };

    let series = match load_series(conn).await {
        Ok(s) => s,
        Err(e) => {
            return finish_failed(&e, env, out, err);
        }
    };

    // Decision 7: zero usable observations is a run failure, not an all-n/a ok.
    if !series.iter().any(|s| !s.rows.is_empty()) {
        return finish_failed(
            "no usable observations — empty store or every configured series missing",
            env,
            out,
            err,
        );
    }

    let message_keys = match message_series_keys(conn).await {
        Ok(k) => k,
        Err(e) => {
            return finish_failed(&e, env, out, err);
        }
    };

    let lead = match message_lead_keys(conn).await {
        Ok(l) => l,
        Err(e) => {
            return finish_failed(&e, env, out, err);
        }
    };

    // Missing kind, a cds_message_series key naming an unknown series, a
    // cds_message_lead key absent from cds_message_series, or a spread/yield
    // mix left outside the lead pair all fail here (RenderError) — failed,
    // no deliver. Both representations come from ONE call so they can never
    // disagree on content (see `format_message_variants`'s doc comment).
    let message = match format_message_variants(&series, as_of, &message_keys, &lead) {
        Ok(m) => m,
        Err(e) => {
            return finish_failed(&e.message, env, out, err);
        }
    };

    // Data present (however stale; partial missing OK) → ok and deliver.
    emit(
        &message,
        "ok",
        args.deliver_to.as_deref(),
        &args.account,
        env,
        out,
        err,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_job_id_plain_stays_bare() {
        // Matches the pre-existing marker contract: the plain representation
        // never gains markup, escaped or otherwise.
        let out = append_job_id("body", Some("job-77"), |s| s.to_string());
        assert_eq!(out, "body\n\njob-77");
    }

    #[test]
    fn append_job_id_html_escapes_the_id() {
        // The job id is DB-independent (it comes from NULLCLAW_JOB_ID, not
        // `cds_series`), but it still lands in the same HTML payload as
        // every escaped field, and the escaping discipline draws no
        // exception for it — see render.rs's `escape_html` doc comment.
        let out = append_job_id("body", Some("<XJOB>&\"'"), escape_html);
        assert!(
            !out.contains("<XJOB>"),
            "job id must not leak raw markup into the HTML payload: {out}"
        );
        assert!(
            out.contains("&lt;XJOB&gt;&amp;\"'"),
            "job id must be escaped (only &, <, > need it — \" and ' are left \
             alone, same as render.rs's escape_html): {out}"
        );
    }

    #[test]
    fn append_job_id_none_is_a_noop_for_either_representation() {
        assert_eq!(append_job_id("body", None, |s| s.to_string()), "body");
        assert_eq!(append_job_id("body", None, escape_html), "body");
    }
}
