//! The excerpt gate that decides whether a fetched body may top up a headline.
//!
//! A Cloudflare challenge page passes a naive word floor and reads like prose
//! after `strip_html`, so the title-token overlap is the real pairing gate;
//! the marker list only short-circuits the obvious chrome. These fixtures pin
//! that ordering.

use news::quality::{usable_excerpt, Article};

fn body(text: &str) -> Article {
    Article {
        text: text.to_string(),
        error: None,
        ..Default::default()
    }
}

fn meta_title() -> &'static str {
    "Meta unveils Muse Glimmer open-weight model"
}

fn real_body() -> String {
    "Meta launched Muse Glimmer on Monday and said the weights will follow. \
     Engineers described the 30-billion-parameter build as small enough to run \
     on a laptop, and the company published benchmarks alongside the weights."
        .repeat(3)
}

#[test]
fn clean_body_yields_a_leading_window() {
    let ex = usable_excerpt(meta_title(), &body(&real_body())).unwrap();
    assert!(ex.contains("Muse"));
    assert!(ex.chars().count() <= 600);
}

#[test]
fn window_cut_is_char_boundary_safe_over_cjk() {
    let text = format!("{}{}", real_body(), "模型權重基準測試資料中心訓練運算。".repeat(120));
    let ex = usable_excerpt(meta_title(), &body(&text)).unwrap();
    assert!(ex.chars().count() <= 600);
    assert!(ex.contains("Muse"));
}

#[test]
fn errored_body_is_never_usable() {
    let a = Article {
        error: Some("boom".to_string()),
        ..Default::default()
    };
    assert!(usable_excerpt(meta_title(), &a).is_none());
}

#[test]
fn body_below_the_word_floor_is_never_usable() {
    assert!(usable_excerpt(meta_title(), &body("too short to be an article")).is_none());
}

#[test]
fn challenge_page_is_never_usable_even_past_the_word_floor() {
    let cf = "Just a moment... Enable JavaScript and cookies to continue. Please stand by, \
              while we are checking your browser before accessing the site. Please wait \
              until the verification process is complete, because this security review \
              helps protect the service from malicious traffic and automated abuse, and \
              then press the button below to continue to the requested page. The delay is \
              normally a few seconds and rarely longer during busy periods today.";
    assert!(usable_excerpt(meta_title(), &body(cf)).is_none());
}

#[test]
fn body_sharing_no_significant_token_with_the_title_is_never_usable() {
    let unrelated = "The committee published its annual report on railway scheduling across \
                     several regional lines with detailed timetables, annexes and consultation \
                     responses from local authorities about seasonal timetable adjustments \
                     affecting rural stations during the winter months, alongside funding notes.";
    assert!(usable_excerpt(meta_title(), &body(unrelated)).is_none());
}

#[test]
fn title_without_any_latin_token_cannot_be_paired() {
    // The overlap test is the pairing evidence; with nothing to overlap, there
    // is no evidence the body belongs to this headline, so no excerpt.
    assert!(usable_excerpt("祖克柏發布開放權重模型", &body(&real_body())).is_none());
}
