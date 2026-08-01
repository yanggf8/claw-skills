//! Sending the finished digest.
//!
//! Kept out of `main` so the chunking, the Markdown probe, and the numbering of
//! multi-part messages can be exercised against a stub endpoint. Wiring is
//! exactly where the earlier ports in this repo broke while every unit test
//! stayed green.

use claw_core::delivery::{deliver, DeliverOptions, DeliveryOutcome};
use serde_json::json;
use std::io::Write;

use crate::render::{markdown_chunk_is_safe, markdown_visible_text, telegram_chunks};
use crate::trace::log_trace;

/// Split, probe, then send.
///
/// A chunk that would trip Telegram's legacy Markdown parser sends the *whole*
/// message as plaintext. That expands every `[🔗](url)` into a bare URL, which
/// is ugly but readable — where a rejected message is nothing at all.
pub fn deliver_news(
    chat_id: Option<&str>,
    body: &str,
    account: &str,
    base_url: Option<String>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> DeliveryOutcome {
    let Some(chat) = chat_id.filter(|c| !c.is_empty()) else {
        let opts = DeliverOptions {
            account: account.to_string(),
            base_url,
            ..Default::default()
        };
        return deliver(None, body, &opts, out, err);
    };

    let chunks = telegram_chunks(body);
    let unsafe_chunks: Vec<(usize, &str)> = chunks
        .iter()
        .enumerate()
        .filter_map(|(i, c)| markdown_chunk_is_safe(c).err().map(|r| (i + 1, r)))
        .collect();

    let parse_mode = if unsafe_chunks.is_empty() {
        Some("Markdown".to_string())
    } else {
        log_trace(
            "digest_markdown_unsafe_fallback",
            json!({"total_chunks": chunks.len(),
                   "unsafe_chunks": unsafe_chunks.iter().map(|(i, _)| i).collect::<Vec<_>>(),
                   "reasons": unsafe_chunks.iter().take(3).map(|(_, r)| r).collect::<Vec<_>>()}),
        );
        None
    };

    if chunks.len() > 1 {
        log_trace(
            "digest_delivery_split",
            json!({"chunks": chunks.len(), "raw_chars": body.chars().count(),
                   "visible_chars": markdown_visible_text(body).chars().count()}),
        );
    }

    let opts = DeliverOptions {
        account: account.to_string(),
        parse_mode,
        base_url,
        ..Default::default()
    };
    let total = chunks.len();
    for (i, chunk) in chunks.iter().enumerate() {
        let text = if total == 1 {
            chunk.clone()
        } else {
            format!("({}/{total})\n{chunk}", i + 1)
        };
        let outcome = deliver(Some(chat), &text, &opts, out, err);
        if outcome == DeliveryOutcome::FailedFatal {
            return outcome;
        }
    }
    DeliveryOutcome::Sent
}
