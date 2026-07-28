use doughcon::cli::{gate, parse_args, Gate};

fn argv(a: &[&str]) -> Vec<String> { a.iter().map(|s| s.to_string()).collect() }

#[test]
fn defaults_are_deliver_mode_and_main_account() {
    let a = parse_args(&argv(&[])).unwrap();
    assert_eq!(a.mode, "deliver");
    assert_eq!(a.account, "main");
    assert!(a.deliver_to.is_none());
    assert!(a.et_hour.is_none());
}

#[test]
fn parses_every_flag() {
    let a = parse_args(&argv(&["--mode", "record", "--deliver-to", "42", "--account", "nunu", "--et-hour", "20"])).unwrap();
    assert_eq!(a.mode, "record");
    assert_eq!(a.deliver_to.as_deref(), Some("42"));
    assert_eq!(a.account, "nunu");
    assert_eq!(a.et_hour, Some(20));
}

#[test]
fn et_hour_is_deliberately_not_range_checked() {
    // The Python's argparse does not validate 0-23; -1 and 99 are accepted and
    // become permanent skips. clap would "fix" this by default.
    assert_eq!(parse_args(&argv(&["--et-hour", "99"])).unwrap().et_hour, Some(99));
    assert_eq!(parse_args(&argv(&["--et-hour", "-1"])).unwrap().et_hour, Some(-1));
}

#[test]
fn rejects_unknown_flags_and_missing_values() {
    assert!(parse_args(&argv(&["--nope"])).is_err());
    assert!(parse_args(&argv(&["--mode"])).is_err());
    assert!(parse_args(&argv(&["--mode", "sideways"])).is_err());
}

#[test]
fn gate_runs_when_hour_matches_or_no_target() {
    assert!(matches!(gate(20, "EDT", Some(20)), Gate::Run));
    assert!(matches!(gate(4, "EDT", None), Gate::Run));
}

#[test]
fn gate_skip_carries_the_hour_and_abbreviation() {
    // The abbreviation is in the stderr line the Python emits; dropping it was
    // a real parity bug in Phase ①.
    match gate(4, "EDT", Some(20)) {
        Gate::Skip { current_hour, abbrev } => { assert_eq!(current_hour, 4); assert_eq!(abbrev, "EDT"); }
        Gate::Run => panic!("expected Skip"),
    }
}
