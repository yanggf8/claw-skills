//! Argument parsing.
//!
//! Unknown flags are an error, not something to ignore. On 2026-07-31 an
//! argument parser in this repo that silently accepted anything let a typo run
//! the wrong mode while still reporting success; the install script now probes
//! for exit 2 on an unknown flag precisely to catch that.

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Deliver(DeliverArgs),
    ManageList { account: String, deliver_to: Option<String> },
    ManageAdd { account: String, topic: String, deliver_to: Option<String> },
    ManageRemove { account: String, topic: String, deliver_to: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DeliverArgs {
    pub lang: String,
    pub deliver_to: Option<String>,
    pub account: String,
    pub topics: Option<String>,
    pub account_topics: bool,
}

pub const USAGE: &str = "\
usage: news [deliver] [--lang zh] [--deliver-to CHAT_ID] [--account NAME]
            [--topics a,b,c] [--account-topics]
       news manage list   [--account NAME] [--deliver-to CHAT_ID]
       news manage add    --topic NAME [--account NAME] [--deliver-to CHAT_ID]
       news manage remove --topic NAME [--account NAME] [--deliver-to CHAT_ID]";

fn value(args: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
    *i += 1;
    args.get(*i)
        .cloned()
        .ok_or_else(|| format!("{flag} expects a value"))
}

pub fn parse_args(argv: &[String]) -> Result<Command, String> {
    let rest: &[String] = match argv.first().map(String::as_str) {
        Some("manage") => return parse_manage(&argv[1..]),
        Some("deliver") => &argv[1..],
        _ => argv,
    };

    let mut a = DeliverArgs {
        lang: "zh".into(),
        account: "main".into(),
        ..Default::default()
    };
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--lang" => a.lang = value(rest, &mut i, "--lang")?,
            "--deliver-to" => a.deliver_to = Some(value(rest, &mut i, "--deliver-to")?),
            "--account" => a.account = value(rest, &mut i, "--account")?,
            "--topics" => a.topics = Some(value(rest, &mut i, "--topics")?),
            "--account-topics" => a.account_topics = true,
            other => return Err(format!("unknown argument: {other}")),
        }
        i += 1;
    }
    Ok(Command::Deliver(a))
}

fn parse_manage(argv: &[String]) -> Result<Command, String> {
    let action = argv.first().cloned().unwrap_or_default();
    let mut account = "main".to_string();
    let mut deliver_to = None;
    let mut topic: Option<String> = None;

    let rest = if argv.is_empty() { argv } else { &argv[1..] };
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--account" => account = value(rest, &mut i, "--account")?,
            "--deliver-to" => deliver_to = Some(value(rest, &mut i, "--deliver-to")?),
            "--topic" => topic = Some(value(rest, &mut i, "--topic")?),
            other => return Err(format!("unknown argument: {other}")),
        }
        i += 1;
    }

    match action.as_str() {
        "list" => Ok(Command::ManageList {
            account,
            deliver_to,
        }),
        "add" => Ok(Command::ManageAdd {
            account,
            topic: topic.ok_or("manage add requires --topic")?,
            deliver_to,
        }),
        "remove" => Ok(Command::ManageRemove {
            account,
            topic: topic.ok_or("manage remove requires --topic")?,
            deliver_to,
        }),
        "" => Err("manage requires an action: list, add, or remove".into()),
        other => Err(format!("unknown manage action: {other}")),
    }
}

/// `--topics` wins over `--account-topics`; neither means the built-in feeds.
pub fn resolve_topics(args: &DeliverArgs) -> Option<Vec<String>> {
    if let Some(raw) = args.topics.as_deref() {
        let list: Vec<String> = raw
            .split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect();
        if !list.is_empty() {
            return Some(list);
        }
        return None;
    }
    if args.account_topics {
        let data = crate::topics::load();
        if let Some(topics) = data.get(&args.account).filter(|t| !t.is_empty()) {
            return Some(topics.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn no_arguments_means_deliver_with_defaults() {
        let Command::Deliver(a) = parse_args(&argv(&[])).unwrap() else {
            panic!("expected deliver");
        };
        assert_eq!(a.account, "main");
        assert_eq!(a.lang, "zh");
        assert_eq!(a.deliver_to, None);
    }

    #[test]
    fn the_deliver_subcommand_is_optional() {
        assert_eq!(
            parse_args(&argv(&["deliver", "--account", "nunu"])).unwrap(),
            parse_args(&argv(&["--account", "nunu"])).unwrap()
        );
    }

    #[test]
    fn an_unknown_flag_is_rejected_rather_than_ignored() {
        // Silently ignoring one lets a typo run a different mode while the run
        // still reports success.
        assert!(parse_args(&argv(&["--delivery-to", "123"])).is_err());
        assert!(parse_args(&argv(&["manage", "list", "--nope"])).is_err());
    }

    #[test]
    fn a_flag_missing_its_value_is_an_error() {
        assert!(parse_args(&argv(&["--deliver-to"])).is_err());
    }

    #[test]
    fn manage_add_requires_a_topic() {
        assert!(parse_args(&argv(&["manage", "add"])).is_err());
        assert!(parse_args(&argv(&["manage", "add", "--topic", "AI"])).is_ok());
    }

    #[test]
    fn manage_without_an_action_is_an_error() {
        assert!(parse_args(&argv(&["manage"])).is_err());
        assert!(parse_args(&argv(&["manage", "sync"])).is_err());
    }

    #[test]
    fn explicit_topics_win_over_the_account_list() {
        let a = DeliverArgs {
            topics: Some("AI, 半導體 ,".into()),
            account_topics: true,
            ..Default::default()
        };
        assert_eq!(
            resolve_topics(&a),
            Some(vec!["AI".to_string(), "半導體".to_string()])
        );
    }

    #[test]
    fn a_topics_flag_of_only_separators_falls_through_to_the_defaults() {
        // Not to the account list: naming --topics is an explicit choice, and
        // an empty one must not silently resurrect a stored subscription.
        let a = DeliverArgs {
            topics: Some(" , ".into()),
            account_topics: true,
            ..Default::default()
        };
        assert_eq!(resolve_topics(&a), None);
    }
}
