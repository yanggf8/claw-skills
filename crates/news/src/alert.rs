//! Telling the operator the skill could not produce news.
//!
//! Two channels, in this order: a durable on-disk block, then a best-effort
//! Telegram message. The file comes first so the record survives when Telegram
//! itself is the failure.

use crate::config::{failure_log, FAILURE_LOG_MAX_BYTES};
use crate::trace::log_trace;
use claw_core::delivery::{deliver, DeliverOptions};
use serde_json::json;
use std::fs::OpenOptions;
use std::io::Write;

/// Who to tell. Built once in `main` and threaded explicitly through the
/// summarize path, so a reader can see exactly which call sites can alert.
pub struct AlertContext {
    pub deliver_to: Option<String>,
    pub account: String,
    pub job_id: String,
}

impl AlertContext {
    pub fn new(deliver_to: Option<String>, account: String, job_id: Option<String>) -> Self {
        Self {
            deliver_to,
            account,
            job_id: job_id.filter(|s| !s.is_empty()).unwrap_or_else(|| "interactive".into()),
        }
    }
}

fn now_cst() -> String {
    jiff::Timestamp::now()
        .in_tz("Asia/Taipei")
        .map(|z| z.strftime("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}

/// How many times this same (reason, account) already fired in the trailing
/// window.
///
/// A single degraded run looks benign; the same alert twenty times over a month
/// is a chronic fault nobody notices. Folding the count into the alert text
/// makes the trend ride along with the alert — no metrics system, just the log
/// already being written. Best effort: any parse or IO trouble counts zero.
pub fn recent_failure_count(reason: &str, account: &str, days: i64) -> usize {
    let cutoff = match jiff::Timestamp::now()
        .in_tz("Asia/Taipei")
        .ok()
        .and_then(|z| z.checked_sub(jiff::Span::new().days(days)).ok())
    {
        Some(c) => c,
        None => return 0,
    };

    let mut count = 0;
    let primary = failure_log();
    let rotated = primary.with_extension(match primary.extension() {
        Some(e) => format!("{}.1", e.to_string_lossy()),
        None => "1".to_string(),
    });
    for path in [primary, rotated] {
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        // Blocks look like "=== <ts> CST ===\n...reason: <r>\naccount: <a>\n".
        for block in text.split("=== ").skip(1) {
            let (head, body) = match block.split_once(" CST ===") {
                Some(p) => p,
                None => continue,
            };
            if body.is_empty() {
                continue;
            }
            let when = jiff::civil::DateTime::strptime("%Y-%m-%d %H:%M:%S", head.trim())
                .ok()
                .and_then(|d| d.in_tz("Asia/Taipei").ok());
            let when = match when {
                Some(w) => w,
                None => continue,
            };
            if when < cutoff {
                continue;
            }
            if body.contains(&format!("reason: {reason}\n"))
                && body.contains(&format!("account: {account}\n"))
            {
                count += 1;
            }
        }
    }
    count
}

/// Never raises, by construction: every step is best effort.
pub fn alert_failure(ctx: &AlertContext, reason: &str, detail: &str) {
    // Counted before this block is written, so the number reads as "already
    // happened N times", excluding the alert being raised now.
    let prior_30d = recent_failure_count(reason, &ctx.account, 30);
    let detail = if prior_30d > 0 {
        format!("{detail} [此告警近30天已出現 {prior_30d} 次]")
    } else {
        detail.to_string()
    };

    let ts = now_cst();
    let block = format!(
        "=== {ts} CST ===\n\
         job_id: {job_id}\n\
         deliver_to: {to}\n\
         account: {account}\n\
         reason: {reason}\n\
         detail: {detail}\n\
         \n",
        job_id = ctx.job_id,
        to = ctx.deliver_to.as_deref().unwrap_or("(none)"),
        account = ctx.account,
    );

    let path = failure_log();
    let oversize = std::fs::metadata(&path).is_ok_and(|m| m.len() > FAILURE_LOG_MAX_BYTES);
    if oversize {
        let rotated = path.with_extension(match path.extension() {
            Some(e) => format!("{}.1", e.to_string_lossy()),
            None => "1".to_string(),
        });
        let _ = std::fs::rename(&path, rotated);
    }
    let written = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| f.write_all(block.as_bytes()));
    if let Err(e) = written {
        log_trace("news_failure_log_error", json!({"error": e.to_string()}));
    }

    log_trace(
        "news_failure_alert",
        json!({"reason": reason, "detail_chars": detail.chars().count()}),
    );

    let Some(chat_id) = ctx.deliver_to.as_deref().filter(|s| !s.is_empty()) else {
        return;
    };
    let msg = format!(
        "⚠️ 新聞無法送出 — {ts}\n原因：{reason}\n細節：{clipped}\njob_id: {job}",
        clipped = detail.chars().take(500).collect::<String>(),
        job = ctx.job_id,
    );
    let opts = DeliverOptions {
        account: ctx.account.clone(),
        // Already in the failure path — a send error here must not escalate.
        fail_on_delivery_error: false,
        ..Default::default()
    };
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();
    deliver(Some(chat_id), &msg, &opts, &mut out, &mut err);
}
