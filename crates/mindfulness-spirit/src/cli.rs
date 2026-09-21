//! Argument parsing.
//!
//! `--account` and `--deliver-to` are deliberately absent: where an
//! installment goes is the column's `delivery_target`, and a second routing
//! source of truth is a way for the two to disagree.

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Write { dry_run: bool },
    FixSignature { devto_id: i64, dry_run: bool },
}

pub const USAGE: &str = "\
usage: mindfulness-spirit [write] [--dry-run]
       mindfulness-spirit fix-signature <DEVTO_ID> [--dry-run]";

pub fn parse_args(argv: &[String]) -> Result<Command, String> {
    let (rest, fixing) = match argv.first().map(String::as_str) {
        Some("fix-signature") => (&argv[1..], true),
        Some("write") => (&argv[1..], false),
        _ => (argv, false),
    };

    let mut dry_run = false;
    let mut positional: Vec<&String> = Vec::new();
    for arg in rest {
        match arg.as_str() {
            "--dry-run" => dry_run = true,
            other if other.starts_with('-') => {
                return Err(format!("unknown argument: {other}"))
            }
            _ => positional.push(arg),
        }
    }

    if !fixing {
        if let Some(extra) = positional.first() {
            return Err(format!("unexpected argument: {extra}"));
        }
        return Ok(Command::Write { dry_run });
    }

    let id = positional
        .first()
        .ok_or("fix-signature requires a dev.to article id")?;
    let devto_id: i64 = id
        .parse()
        .map_err(|_| format!("dev.to article id must be a number, got {id}"))?;
    if positional.len() > 1 {
        return Err(format!("unexpected argument: {}", positional[1]));
    }
    Ok(Command::FixSignature { devto_id, dry_run })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn no_arguments_means_write() {
        assert_eq!(
            parse_args(&argv(&[])).unwrap(),
            Command::Write { dry_run: false }
        );
    }

    #[test]
    fn the_write_subcommand_is_optional() {
        assert_eq!(
            parse_args(&argv(&["write", "--dry-run"])).unwrap(),
            parse_args(&argv(&["--dry-run"])).unwrap()
        );
    }

    #[test]
    fn fix_signature_needs_a_numeric_id() {
        assert_eq!(
            parse_args(&argv(&["fix-signature", "12345"])).unwrap(),
            Command::FixSignature {
                devto_id: 12345,
                dry_run: false
            }
        );
        assert!(parse_args(&argv(&["fix-signature"])).is_err());
        assert!(parse_args(&argv(&["fix-signature", "abc"])).is_err());
    }

    #[test]
    fn an_unknown_flag_is_rejected_rather_than_ignored() {
        // The install probe requires exit 2 on one of these; more to the point,
        // a silently-ignored flag lets a typo publish when the operator asked
        // for a dry run.
        assert!(parse_args(&argv(&["--dryrun"])).is_err());
        assert!(parse_args(&argv(&["--deliver-to", "123"])).is_err());
    }

    #[test]
    fn a_stray_positional_is_rejected() {
        assert!(parse_args(&argv(&["write", "extra"])).is_err());
        assert!(parse_args(&argv(&["fix-signature", "1", "2"])).is_err());
    }
}
