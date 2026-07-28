//! Canonical delivery helper.
//!
//! Centralises the "skill silently succeeds after a delivery failure" bug class:
//! on failure the body is preserved on stdout so the cron capture still has the
//! data, and the diagnostic goes to stderr.
//!
//! Unlike the Python original this does NOT exit the process. It returns an
//! outcome and the binary decides, because exit code and semantic status are
//! independent in nullclaw's classification.

use std::io::Write;
use std::path::PathBuf;

use crate::budget::resolve_delivery_deadline;
use crate::telegram::{send, SendOptions};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryOutcome {
    PrintedToStdout,
    Sent,
    FailedFatal,
    FailedSoft,
}

pub struct DeliverOptions {
    pub account: String,
    pub fail_on_delivery_error: bool,
    pub parse_mode: Option<String>,
    pub config_path: Option<PathBuf>,
    pub base_url: Option<String>,
}

impl Default for DeliverOptions {
    fn default() -> Self {
        Self {
            account: "main".into(),
            fail_on_delivery_error: true,
            parse_mode: Some("Markdown".into()),
            config_path: None,
            base_url: None,
        }
    }
}

pub fn deliver(
    chat_id: Option<&str>,
    body: &str,
    opts: &DeliverOptions,
    out: &mut impl Write,
    err: &mut impl Write,
) -> DeliveryOutcome {
    let chat = chat_id.filter(|c| !c.is_empty());
    let Some(chat) = chat else {
        let _ = writeln!(out, "{body}");
        let _ = out.flush();
        return DeliveryOutcome::PrintedToStdout;
    };

    let send_opts = SendOptions {
        account: opts.account.clone(),
        config_path: opts.config_path.clone(),
        deadline_s: resolve_delivery_deadline(),
        parse_mode: opts.parse_mode.clone(),
        base_url: opts.base_url.clone(),
    };

    if send(chat, body, &send_opts) {
        return DeliveryOutcome::Sent;
    }

    // stdout first — it is the data. stderr second — it is the diagnostic.
    let _ = writeln!(out, "{body}");
    let _ = out.flush();
    let _ = writeln!(
        err,
        "[delivery] telegram send failed for chat={chat} account={}",
        opts.account
    );
    let _ = err.flush();

    if opts.fail_on_delivery_error {
        DeliveryOutcome::FailedFatal
    } else {
        DeliveryOutcome::FailedSoft
    }
}
