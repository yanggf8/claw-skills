//! One place to build an HTTP agent with a timeout that actually holds.
//!
//! `ureq::builder().timeout(d)` does not bound the connect phase. A host that
//! refuses a connection fails instantly either way, so this looks fine in every
//! normal test — but a host that *blackholes* packets hangs for ureq's own
//! 30-second connect default no matter what `timeout` says. Measured on
//! 2026-08-01: a 50 ms overall timeout against an unreachable address took
//! 30.03 s, while the same request with `timeout_connect` took 50 ms.
//!
//! That matters here because the Python this was ported from does not behave
//! that way: `urllib.request.urlopen(req, timeout=N)` applies N to the connect
//! too. Six feeds at a nominal 15 s could therefore cost three minutes rather
//! than ninety seconds, which is past a cron kill window — and no output
//! differential can see it, because the difference is time, not bytes.

use std::time::Duration;

/// An agent where every phase — connect, read, write, and the call overall —
/// is bounded by `timeout`.
pub fn agent(timeout: Duration) -> ureq::Agent {
    ureq::builder()
        .timeout_connect(timeout)
        .timeout_read(timeout)
        .timeout_write(timeout)
        .timeout(timeout)
        .build()
}

/// The status or transport class, never the rendered error.
///
/// `ureq`'s `Display` includes the full URL with its query string, which is
/// how an API key reached a delivered message once before in this repo.
pub fn error_class(e: &ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, _) => format!("HTTP {code}"),
        ureq::Error::Transport(_) => "transport error".to_string(),
    }
}
