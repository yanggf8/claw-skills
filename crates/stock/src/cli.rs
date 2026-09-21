//! Argument parsing.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Market {
    Tw,
    Hk,
    All,
}

impl Market {
    pub fn wants_tw(&self) -> bool {
        matches!(self, Market::Tw | Market::All)
    }
    pub fn wants_hk(&self) -> bool {
        matches!(self, Market::Hk | Market::All)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub market: Market,
    pub symbol: Option<String>,
    pub deliver_to: Option<String>,
    pub account: String,
}

/// Parse argv (no program name).
///
/// `--market` is closed to tw / hk / all, as argparse's `choices` was: a typo
/// should be refused rather than silently treated as "all".
pub fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut a = Args {
        market: Market::All,
        symbol: None,
        deliver_to: None,
        account: "main".into(),
    };
    let mut i = 0;
    while i < argv.len() {
        let need = |i: usize| -> Result<String, String> {
            argv.get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{} requires a value", argv[i]))
        };
        match argv[i].as_str() {
            "--market" => {
                a.market = match need(i)?.as_str() {
                    "tw" => Market::Tw,
                    "hk" => Market::Hk,
                    "all" => Market::All,
                    other => {
                        return Err(format!(
                            "argument --market: invalid choice: '{other}' (choose from 'tw', 'hk', 'all')"
                        ))
                    }
                };
                i += 2;
            }
            "--symbol" => {
                a.symbol = Some(need(i)?);
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
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(a)
}
