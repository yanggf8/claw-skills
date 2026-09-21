//! CLI parsing and DST-gate decision for doughcon.
//!
//! Extracted from `main.rs` so `cargo test` can reach arg parsing and the gate
//! without spawning the binary or reading the wall clock.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub mode: String,
    pub deliver_to: Option<String>,
    pub account: String,
    pub et_hour: Option<i32>,
}

/// Pure DST-gate decision. `main` supplies the current US-Eastern hour and
/// abbreviation; this function never reads a clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gate {
    Run,
    Skip { current_hour: i32, abbrev: String },
}

/// Parse CLI flags from an argv slice (no program name). Takes argv rather than
/// reading `std::env::args` so it is unit-testable.
pub fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut a = Args {
        mode: "deliver".into(),
        deliver_to: None,
        account: "main".into(),
        et_hour: None,
    };
    let mut i = 0;
    while i < argv.len() {
        let need = |i: usize| -> Result<String, String> {
            argv.get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{} requires a value", argv[i]))
        };
        match argv[i].as_str() {
            "--mode" => {
                a.mode = need(i)?;
                i += 2;
            }
            "--deliver-to" => {
                a.deliver_to = Some(need(i)?);
                i += 2;
            }
            "--account" => {
                a.account = need(i)?;
                i += 2;
            }
            // Deliberately NOT range-validated: the Python accepts -1 and 99,
            // which become permanent skips. Pinned by characterization test.
            "--et-hour" => {
                a.et_hour = Some(
                    need(i)?
                        .parse()
                        .map_err(|_| "--et-hour must be an integer".to_string())?,
                );
                i += 2;
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if a.mode != "deliver" && a.mode != "record" {
        return Err(format!("--mode must be deliver or record, got {}", a.mode));
    }
    Ok(a)
}

/// Decide whether the run should proceed given the current US-Eastern hour and
/// optional `--et-hour` target. `None` target always runs (gate not requested).
pub fn gate(now_hour: i32, abbrev: &str, target: Option<i32>) -> Gate {
    match target {
        None => Gate::Run,
        Some(t) if t == now_hour => Gate::Run,
        Some(_) => Gate::Skip {
            current_hour: now_hour,
            abbrev: abbrev.to_string(),
        },
    }
}
