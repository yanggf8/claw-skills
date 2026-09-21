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

#[test]
fn an_unreachable_upstream_is_bounded_by_the_fetch_timeout() {
    // Not a doughcon rule so much as a check that this crate goes through
    // claw_core::http::agent. Building a ureq agent by hand leaves the connect
    // phase on ureq's own 30-second default however short the stated timeout
    // is, so a twenty-second budget silently becomes thirty — and with a
    // handful of upstreams that is the difference between finishing and being
    // killed by cron. tools/lint-http.sh keeps the other crates honest; this
    // measures that the wrapper actually does what it claims.
    //
    // Costs ~20s, the crate's own budget, because there is no seam to shorten
    // it and a refused connection would prove nothing — it returns fast with
    // or without the bug. The static check cannot see composition and the
    // claw-core unit test cannot see this crate, so something has to pay.
    let started = std::time::Instant::now();
    // TEST-NET-3: routed nowhere, so the connect hangs rather than refusing.
    let out = doughcon::pizzint::fetch(Some("http://203.0.113.1"));
    assert!(out.is_err());
    let took = started.elapsed();
    if took < std::time::Duration::from_millis(100) {
        eprintln!("note: this host refuses TEST-NET-3; the connect bound was not exercised");
        return;
    }
    assert!(
        took < std::time::Duration::from_secs(25),
        "a 20s budget took {took:?}; the connect phase is unbounded again"
    );
}
