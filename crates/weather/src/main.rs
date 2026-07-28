//! weather — fetch forecast for Taiwan (CWA) and Hong Kong (HKO).
//!
//! Thin binary: parse argv, load env, run orchestration, append advice +
//! job-id footer, deliver, finish. Exit ownership lives here: a hard delivery
//! failure must exit BEFORE markers (same rule as doughcon).

use std::io::Write;
use std::time::Duration;

use claw_core::agent::call_agent;
use claw_core::delivery::{deliver, DeliverOptions, DeliveryOutcome};
use claw_core::env::load_env;
use claw_core::marker::emit_fallback;
use claw_core::outcome::{finish, Finish};
use weather::cli::{self, assemble_body, format_advice_line};
use weather::orchestrate::{self, status_of, Sources};
use weather::routing;
use weather::sources::hko::{self, HkoData};
use weather::sources::open_meteo::{self, OmData};
use weather::sources::cwa;

/// Live HTTP sources. Production defaults apply when the env seams are unset.
/// `HKO_BASE_URL` / `CWA_BASE_URL` / `OPEN_METEO_BASE_URL` redirect each source
/// independently for the differential harness (same role as `DOUGHCON_BASE_URL`).
struct LiveSources {
    api_key: String,
    hko_base: Option<String>,
    cwa_base: Option<String>,
    om_base: Option<String>,
}

impl Sources for LiveSources {
    fn hko(&self) -> Result<HkoData, String> {
        hko::fetch(self.hko_base.as_deref())
    }

    fn cwa(&self, locs: &[String]) -> Result<String, String> {
        cwa::fetch(self.cwa_base.as_deref(), locs, &self.api_key)
    }

    fn open_meteo(&self, loc: &str) -> Result<OmData, String> {
        // orchestrate only calls this after confirming coords exist.
        let &(lat, lon) = open_meteo::tw_coords()
            .get(loc)
            .ok_or_else(|| format!("no fallback coordinates for '{loc}'"))?;
        open_meteo::fetch(self.om_base.as_deref(), lat, lon)
    }
}

fn env_url(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.is_empty())
}

fn main() {
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();

    load_env(None);

    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match cli::parse_args(&argv) {
        Ok(a) => a,
        Err(e) => {
            let _ = writeln!(err, "[ERROR: {e}]");
            std::process::exit(2);
        }
    };

    let locations = routing::with_default(args.locations);
    let (hk, tw) = routing::split(&locations);

    let api_key = std::env::var("CWA_API_KEY").unwrap_or_default();
    let src = LiveSources {
        api_key: api_key.clone(),
        hko_base: env_url("HKO_BASE_URL"),
        cwa_base: env_url("CWA_BASE_URL"),
        om_base: env_url("OPEN_METEO_BASE_URL"),
    };

    let outcome = orchestrate::run(&hk, &tw, &api_key, &src);

    if let Some(ev) = &outcome.fallback_event {
        let _ = emit_fallback(
            "Weather",
            "CWA",
            "Open-Meteo",
            &ev.reason,
            &ev.scope,
            Some(ev.elapsed_ms),
            &mut err,
        );
    }

    // Advice only if rows is non-empty; line only if sanitized advice non-empty (B13).
    let advice_line = if !outcome.rows.is_empty() {
        let prompt = cli::advice_prompt(&outcome.rows);
        let advice = call_agent(&prompt, Duration::from_secs(30));
        format_advice_line(&advice)
    } else {
        None
    };

    let job_id = std::env::var("NULLCLAW_JOB_ID")
        .ok()
        .filter(|v| !v.is_empty());
    let body = assemble_body(
        &outcome.lines,
        advice_line.as_deref(),
        job_id.as_deref(),
    );

    let opts = DeliverOptions {
        account: args.account.clone(),
        ..Default::default()
    };
    let delivery = deliver(
        args.deliver_to.as_deref(),
        &body,
        &opts,
        &mut out,
        &mut err,
    );
    // FailedFatal exits 1 BEFORE markers — nullclaw's exit_code != 0 branch
    // overrides marker parsing anyway.
    if delivery == DeliveryOutcome::FailedFatal {
        std::process::exit(finish(Finish::Unmarked { exit: 1 }, &mut out));
    }

    let status = status_of(&outcome);
    std::process::exit(finish(
        Finish::Marked { status, exit: 0 },
        &mut out,
    ));
}
