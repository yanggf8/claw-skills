use weather::routing::{is_hk, split, with_default};

fn v(a: &[&str]) -> Vec<String> { a.iter().map(|s| s.to_string()).collect() }

#[test]
fn hk_membership_is_the_closed_set_after_lowercase_trim() {
    for s in ["香港", "hong kong", "HK", " hk ", "九龍", "新界", "港島"] {
        assert!(is_hk(s), "{s} should be HK");
    }
    for s in ["臺北市", "台北", "Hong Kong Island", "hk1", "九龍城"] {
        assert!(!is_hk(s), "{s} should be TW");
    }
}

#[test]
fn split_preserves_order_and_duplicates() {
    // B4: repeated HK aliases each produce their own line later, so the split
    // must not deduplicate.
    let (hk, tw) = split(&v(&["香港", "臺北市", "hk", "香港", "新北市"]));
    assert_eq!(hk, v(&["香港", "hk", "香港"]));
    assert_eq!(tw, v(&["臺北市", "新北市"]));
}

#[test]
fn empty_input_defaults_to_taipei() {
    assert_eq!(with_default(vec![]), v(&["臺北市"]));
    assert_eq!(with_default(v(&["高雄市"])), v(&["高雄市"]));
}
