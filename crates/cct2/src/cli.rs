//! Argument parsing. `--mode` is required and closed to two values, as
//! argparse's `required=True, choices=[...]` was.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    PreMarket,
    Eod,
}

impl Mode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::PreMarket => "pre-market",
            Mode::Eod => "eod",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub mode: Mode,
    pub deliver_to: Option<String>,
    pub account: String,
    /// Run only when the market-time hour matches. The scheduler fires on a
    /// fixed UTC expression, so an ET-pinned job is scheduled at both UTC hours
    /// the year can put it at and the wrong one skips here.
    pub et_hour: Option<i32>,
}

pub fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut mode: Option<Mode> = None;
    let mut deliver_to = None;
    let mut account = "main".to_string();
    let mut et_hour: Option<i32> = None;
    let mut i = 0;
    while i < argv.len() {
        let need = |i: usize| -> Result<String, String> {
            argv.get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{} requires a value", argv[i]))
        };
        match argv[i].as_str() {
            "--mode" => {
                mode = Some(match need(i)?.as_str() {
                    "pre-market" => Mode::PreMarket,
                    "eod" => Mode::Eod,
                    o => {
                        return Err(format!(
                            "argument --mode: invalid choice: '{o}' (choose from 'pre-market', 'eod')"
                        ))
                    }
                });
                i += 2;
            }
            "--deliver-to" => { deliver_to = Some(need(i)?); i += 2; }
            "--account" => { account = need(i)?; i += 2; }
            "--et-hour" => {
                et_hour = Some(
                    need(i)?
                        .parse::<i32>()
                        .map_err(|_| "--et-hour must be an integer".to_string())?,
                );
                i += 2;
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(Args {
        mode: mode.ok_or("the following arguments are required: --mode")?,
        deliver_to,
        account,
        et_hour,
    })
}
