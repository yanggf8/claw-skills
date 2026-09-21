//! cds-con binary — thin wrapper around `cds_con::run::run`.
//!
//! Supplies real argv, process environment, a price-registry connection,
//! the real CST calendar date, and stdout/stderr, then `process::exit`s the
//! code. Connect failure is `StoreAccess::Unreachable` so decision 7's
//! hard-failure path (failed, no deliver) runs after argparse.

use std::io::Write;
use std::path::PathBuf;

use cds_con::run::{run, Env, StoreAccess};
use turso_util::{connect, RegistryConfig, TokenEnvPolicy, TokenTier};

/// Same registry locator as price-cli / oilcon. Duplicated here because
/// `price-cli` is a `[[bin]]`-only crate with no `[lib]` (Task 4 may relocate
/// this; for now the values are the public env contract, not private logic).
fn price_registry() -> RegistryConfig {
    RegistryConfig {
        db_name: "price-registry".into(),
        db_name_envs: vec!["PRICE_TURSO_DB".into()],
        db_url_envs: vec!["PRICE_TURSO_URL".into()],
        operator_env: "PRICE_OPERATOR".into(),
        config_home_env: "GWEBCDB_CONFIG_HOME".into(),
        cache_namespace: "gwebcdb".into(),
        token_envs: TokenEnvPolicy {
            read: vec!["PRICE_TURSO_READ_TOKEN".into()],
            write: vec!["PRICE_TURSO_WRITE_TOKEN".into()],
            secrets: vec![],
            allow_generic_fallback: false,
        },
        supported_tiers: vec![TokenTier::Read, TokenTier::Write],
    }
}

/// CST calendar day for age-in-days on the freshness line.
fn cst_today() -> String {
    let tz = jiff::tz::TimeZone::fixed(jiff::tz::offset(8));
    jiff::Timestamp::now()
        .to_zoned(tz)
        .strftime("%Y-%m-%d")
        .to_string()
}

fn connect_warning(e: &turso_util::Error) -> String {
    let msg = e.message();
    let no_url = std::env::var("PRICE_TURSO_URL")
        .ok()
        .filter(|v| !v.is_empty())
        .is_none();
    let no_token = std::env::var("PRICE_TURSO_READ_TOKEN")
        .ok()
        .filter(|v| !v.is_empty())
        .is_none();
    if no_url || no_token {
        "turso credentials missing".into()
    } else {
        format!("turso unavailable - {msg}")
    }
}

#[tokio::main]
async fn main() {
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();

    let argv: Vec<String> = std::env::args().collect();
    let env = Env {
        job_id: std::env::var("NULLCLAW_JOB_ID")
            .ok()
            .filter(|v| !v.is_empty()),
        home: std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".")),
    };

    let as_of = cst_today();

    // Read tier: deliver only reads. Fetch writes live in `price cds fetch`.
    let code = match connect(&price_registry(), TokenTier::Read).await {
        Ok(db) => match db.connect() {
            Ok(conn) => {
                run(
                    &argv,
                    &env,
                    StoreAccess::Ok(&conn),
                    &as_of,
                    &mut out,
                    &mut err,
                )
                .await
            }
            Err(e) => {
                run(
                    &argv,
                    &env,
                    StoreAccess::Unreachable(format!("turso unavailable - {e}")),
                    &as_of,
                    &mut out,
                    &mut err,
                )
                .await
            }
        },
        Err(e) => {
            let warning = connect_warning(&e);
            run(
                &argv,
                &env,
                StoreAccess::Unreachable(warning),
                &as_of,
                &mut out,
                &mut err,
            )
            .await
        }
    };

    let _ = out.flush();
    let _ = err.flush();
    std::process::exit(code);
}
