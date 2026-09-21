use doughcon::pizzint::{parse, RawIndex};

fn present(rendered: &str, is_zero: bool) -> RawIndex {
    RawIndex::Present { rendered: rendered.to_string(), is_zero }
}
use doughcon::report::{derive_index, format_body, NO_DATA};

#[test]
fn normal_index_passes_through() {
    assert_eq!(derive_index(&present("42", false), &[false, false]), "42");
}

#[test]
fn zero_with_all_null_is_no_data() {
    assert_eq!(derive_index(&present("0", true), &[true, true]), NO_DATA);
}

#[test]
fn zero_with_real_data_stays_zero() {
    // A genuine zero is not no-data.
    assert_eq!(derive_index(&present("0", true), &[false, false]), "0");
}

#[test]
fn missing_index_is_no_data() {
    assert_eq!(derive_index(&RawIndex::Missing, &[false]), NO_DATA);
}

#[test]
fn empty_places_counts_as_all_null() {
    assert_eq!(derive_index(&present("0", true), &[]), NO_DATA);
}

#[test]
fn mixed_null_and_present_popularity_is_not_all_null() {
    // THE mutation guard: with `.all()` this is "not all null" so a zero index
    // stays 0; with `.any()` it would collapse to the -1 sentinel. Every other
    // fixture is uniformly null or uniformly present, so this is the only input
    // that can tell the two apart — a prior verification pass proved that
    // swapping .all() for .any() survived the entire suite without it.
    assert_eq!(derive_index(&present("0", true), &[true, false]), "0");
    assert_eq!(derive_index(&present("0", true), &[false, true]), "0");
    assert_eq!(derive_index(&present("0", true), &[true, true]), NO_DATA);
}

#[test]
fn python_zero_semantics_cover_float_and_false() {
    // Python's `raw_index == 0` is true for 0.0, -0.0 and False. Treating only
    // the integer 0 as zero flips the no-data path — and therefore the skill
    // status — on exactly the payload this code exists to detect.
    for body in [
        r#"{"overall_index":0.0,"data":[{"current_popularity":null}]}"#,
        r#"{"overall_index":-0.0,"data":[{"current_popularity":null}]}"#,
        r#"{"overall_index":false,"data":[{"current_popularity":null}]}"#,
    ] {
        let s = parse(body).unwrap();
        assert_eq!(
            derive_index(&s.raw_index, &s.popularity_is_null),
            NO_DATA,
            "payload {body} must be no-data"
        );
    }
    // true == 0 is FALSE in Python, so it renders instead of collapsing.
    let t = parse(r#"{"overall_index":true,"data":[{"current_popularity":null}]}"#).unwrap();
    assert_eq!(derive_index(&t.raw_index, &t.popularity_is_null), "True");
}

#[test]
fn defcon_level_defaults_only_when_key_is_absent() {
    // Python is data.get("defcon_level", "?"): a null level is None, not "?".
    assert_eq!(parse(r#"{"overall_index":1}"#).unwrap().level, "?");
    assert_eq!(parse(r#"{"defcon_level":null,"overall_index":1}"#).unwrap().level, "None");
    assert_eq!(parse(r#"{"defcon_level":true,"overall_index":1}"#).unwrap().level, "True");
    assert_eq!(parse(r#"{"defcon_level":3,"overall_index":1}"#).unwrap().level, "3");
    assert_eq!(parse(r#"{"defcon_level":"3","overall_index":1}"#).unwrap().level, "3");
}

#[test]
fn non_numeric_popularity_is_not_null() {
    // Python tests `is None` only. The string "x" is NOT None there, so
    // all_null is false and a zero index stays 0 with status ok. Parsing to a
    // number and treating failure as null would flip this to degraded, which
    // nullclaw escalates to last_status=error plus a retry.
    let s = parse(r#"{"overall_index":0,"data":[{"current_popularity":"x"}]}"#).unwrap();
    assert_eq!(s.popularity_is_null, vec![false]);
    assert_eq!(derive_index(&s.raw_index, &s.popularity_is_null), "0");
}

#[test]
fn explicit_json_null_popularity_is_null() {
    let s = parse(r#"{"overall_index":0,"data":[{"current_popularity":null}]}"#).unwrap();
    assert_eq!(s.popularity_is_null, vec![true]);
    assert_eq!(derive_index(&s.raw_index, &s.popularity_is_null), NO_DATA);
}

#[test]
fn absent_popularity_key_is_null() {
    let s = parse(r#"{"overall_index":0,"data":[{}]}"#).unwrap();
    assert_eq!(s.popularity_is_null, vec![true]);
}

#[test]
fn non_integer_index_passes_through_verbatim() {
    // Python prints whatever it got and `== 0` is false, so it is never the
    // sentinel. A float and a string both survive.
    let f = parse(r#"{"overall_index":3.5,"data":[{"current_popularity":null}]}"#).unwrap();
    assert_eq!(derive_index(&f.raw_index, &f.popularity_is_null), "3.5");
    let s = parse(r#"{"overall_index":"12","data":[{"current_popularity":null}]}"#).unwrap();
    assert_eq!(derive_index(&s.raw_index, &s.popularity_is_null), "12");
}

#[test]
fn body_matches_python_layout() {
    let b = format_body("3", "42", "2026-06-03 11:23 CST（美東 06-03 23:23 EDT）", None);
    assert_eq!(
        b,
        "🍕 DOUGHCON 情報\n目前等級：DOUGHCON 3\n指數：42\n更新：2026-06-03 11:23 CST（美東 06-03 23:23 EDT）"
    );
}

#[test]
fn body_appends_job_id_when_present() {
    let b = format_body("3", "42", "U", Some("t-1"));
    assert!(b.ends_with("\n\n`t-1`"));
}
