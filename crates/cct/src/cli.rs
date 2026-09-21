//! Argument parsing. `--mode` is required and closed to four values.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    PreMarket,
    Intraday,
    Eod,
    Weekly,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        Some(match s {
            "pre-market" => Mode::PreMarket,
            "intraday" => Mode::Intraday,
            "eod" => Mode::Eod,
            "weekly" => Mode::Weekly,
            _ => return None,
        })
    }

    /// The `--mode` spelling, for diagnostics.
    ///
    /// Four cron jobs share the skill name, and nullclaw's alert prints the
    /// name without the args, so a warning that omits the mode leaves the
    /// reader guessing which of the four degraded.
    pub fn slug(self) -> &'static str {
        match self {
            Mode::PreMarket => "pre-market",
            Mode::Intraday => "intraday",
            Mode::Eod => "eod",
            Mode::Weekly => "weekly",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub mode: Mode,
    pub deliver_to: Option<String>,
    pub account: String,
}

pub fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut mode = None;
    let mut deliver_to = None;
    let mut account = "main".to_string();
    let mut i = 0;
    while i < argv.len() {
        let need = |i: usize| -> Result<String, String> {
            argv.get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{} requires a value", argv[i]))
        };
        match argv[i].as_str() {
            "--mode" => {
                let v = need(i)?;
                mode = Some(Mode::parse(&v).ok_or_else(|| {
                    format!("argument --mode: invalid choice: '{v}' (choose from 'pre-market', 'intraday', 'eod', 'weekly')")
                })?);
                i += 2;
            }
            "--deliver-to" => { deliver_to = Some(need(i)?); i += 2; }
            "--account" => { account = need(i)?; i += 2; }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(Args {
        mode: mode.ok_or("the following arguments are required: --mode")?,
        deliver_to,
        account,
    })
}
