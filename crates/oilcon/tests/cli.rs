//! The install probe's contract, as an offline test.
//!
//! `tools/install-skill.sh` requires an unknown flag to exit 2, and it runs in
//! whatever environment the operator happens to have. main() connects to the
//! price registry BEFORE run()'s strict `parse_args` gets a say, and the
//! no-registry path (`dispatch_warning`) re-parses argv leniently — so in a
//! shell without Turso credentials the probe used to get exit 0 and the skill
//! body instead of the refusal (2026-09-11). The refusal must live before the
//! connect, in every environment.

use std::process::Command;

fn probe_exit_code() -> i32 {
    let out = Command::new(env!("CARGO_BIN_EXE_oilcon"))
        .arg("--__install_smoke_probe__")
        // Deterministic: refuse regardless of what the invoking shell exports.
        // A credentialed shell reaches run()'s strict parser and exits 2 even
        // without this fix, which would make the test blind to the defect.
        .env_remove("PRICE_TURSO_URL")
        .env_remove("PRICE_TURSO_DB")
        .env_remove("PRICE_TURSO_WRITE_TOKEN")
        .env_remove("PRICE_TURSO_READ_TOKEN")
        .env_remove("PRICE_OPERATOR")
        .output()
        .expect("spawn oilcon");
    out.status.code().expect("oilcon must exit by code, not a signal")
}

#[test]
fn an_unknown_flag_exits_2_even_without_registry_credentials() {
    assert_eq!(probe_exit_code(), 2, "stderr was the parser's refusal, not a crash");
}
