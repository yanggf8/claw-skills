//! Tests for the deterministic text layer, written against the Python's
//! observable behaviour and the reasons behind it.

use news::text::{cluster, dedup, extract_source_name, pick_representatives, title_without_source,
                 topic_words, Item};

fn it(title: &str) -> Item {
    Item { title: title.into(), link: "http://x".into(), source: String::new(), ..Default::default() }
}

// ── source name ──────────────────────────────────────────────────────────────

#[test]
fn the_source_is_whatever_follows_the_last_dash() {
    assert_eq!(extract_source_name("Nvidia beats - Reuters"), "Reuters");
}

#[test]
fn a_dash_inside_the_headline_does_not_confuse_the_split() {
    // rsplit, not split: only the LAST " - " separates the source.
    assert_eq!(
        extract_source_name("AI chips - the next war - TechCrunch"),
        "TechCrunch"
    );
    assert_eq!(
        title_without_source("AI chips - the next war - TechCrunch"),
        "AI chips - the next war"
    );
}

#[test]
fn a_headline_with_no_source_keeps_all_of_itself() {
    assert_eq!(extract_source_name("Nvidia beats"), "");
    assert_eq!(title_without_source("Nvidia beats"), "Nvidia beats");
}

// ── tokenising ───────────────────────────────────────────────────────────────

#[test]
fn short_and_common_words_are_not_tokens() {
    let w = topic_words("The new AI is on a chip - Reuters");
    assert!(!w.contains("the"));
    assert!(!w.contains("new"));
    assert!(!w.contains("ai")); // too common in this feed to distinguish anything
    assert!(!w.contains("is"));
    assert!(w.contains("chip"));
}

#[test]
fn the_source_name_is_excluded_from_the_tokens() {
    // Otherwise every Reuters headline would cluster with every other one.
    assert!(!topic_words("Nvidia beats - Reuters").contains("reuters"));
}

#[test]
fn chinese_is_tokenised_as_bigrams_not_whole_runs() {
    // No spaces to split on, so the bigram is the smallest unit that carries
    // meaning. "台積電" yields 台積 and 積電.
    let w = topic_words("台積電法說會");
    assert!(w.contains("台積"), "{w:?}");
    assert!(w.contains("積電"), "{w:?}");
}

#[test]
fn business_filler_bigrams_are_dropped() {
    // "股價" and "上漲" appear in a large share of these headlines and would
    // cluster unrelated companies together.
    let w = topic_words("台積電股價上漲");
    assert!(!w.contains("股價"), "{w:?}");
    assert!(!w.contains("上漲"), "{w:?}");
    assert!(w.contains("台積"), "{w:?}");
}

#[test]
fn a_bigram_touching_a_stop_character_is_dropped() {
    // "的" is a stopword, so neither bigram containing it survives.
    let w = topic_words("蘋果的新機");
    assert!(!w.iter().any(|s| s.contains('的')), "{w:?}");
}

#[test]
fn a_version_number_survives_as_one_token() {
    // The pattern includes '.', so "gpt-4.5" yields "gpt" and "4.5" rather
    // than fragmenting the version.
    let w = topic_words("OpenAI ships GPT 4.5 today");
    assert!(w.contains("4.5"), "{w:?}");
}

// ── dedup ────────────────────────────────────────────────────────────────────

#[test]
fn identical_titles_collapse_case_insensitively() {
    let out = dedup(&[it("Nvidia Beats"), it("nvidia beats"), it("AMD misses")]);
    assert_eq!(out.len(), 2);
    // First occurrence wins, so the earlier feed's casing is what ships.
    assert_eq!(out[0].title, "Nvidia Beats");
}

// ── clustering ───────────────────────────────────────────────────────────────

#[test]
fn two_headlines_sharing_enough_tokens_are_one_event() {
    let out = cluster(&[
        it("Nvidia earnings beat expectations - Reuters"),
        it("Nvidia earnings top forecasts - AP"),
    ]);
    assert_eq!(out.len(), 1, "{out:?}");
    assert_eq!(out[0].len(), 2);
}

#[test]
fn one_shared_token_is_not_enough() {
    // The threshold is 2. A single shared word is a coincidence, not an event.
    let out = cluster(&[
        it("Nvidia earnings beat expectations"),
        it("Nvidia hires chief scientist"),
    ]);
    assert_eq!(out.len(), 2, "{out:?}");
}

#[test]
fn membership_is_judged_against_the_seed_not_the_growing_group() {
    // If the group accumulated its members' tokens, each addition would widen
    // what counts as the same event and unrelated headlines would drift in.
    // A joins B on shared tokens; C shares tokens only with B, not the seed.
    let out = cluster(&[
        it("alpha beta gamma"),
        it("alpha beta delta"),
        it("delta epsilon zeta"),
    ]);
    assert_eq!(out.len(), 2, "{out:?}");
    assert_eq!(out[0].len(), 2);
}

#[test]
fn a_headline_too_short_to_tokenise_stands_alone() {
    // Fewer than CLUSTER_OVERLAP tokens: it can never match, and nothing can
    // ever match it.
    let out = cluster(&[it("Nvidia"), it("Nvidia")]);
    assert_eq!(out.len(), 2, "{out:?}");
}

#[test]
fn the_largest_cluster_is_reported_first() {
    let out = cluster(&[
        it("solo headline about weather patterns"),
        it("chip export controls tighten again"),
        it("chip export controls widen further"),
        it("chip export controls expand once more"),
    ]);
    assert_eq!(out[0].len(), 3, "{out:?}");
}

#[test]
fn equal_sized_clusters_keep_the_order_they_were_found_in() {
    // An unstable sort here would let the digest reorder between runs on
    // identical input.
    let out = cluster(&[
        it("apple silicon roadmap leaked online"),
        it("apple silicon roadmap details emerge"),
        it("google tensor roadmap leaked online"),
        it("google tensor roadmap details emerge"),
    ]);
    assert_eq!(out.len(), 2);
    assert!(out[0][0].title.starts_with("apple"), "{out:?}");
}

// ── representatives ──────────────────────────────────────────────────────────

#[test]
fn one_representative_per_cluster_by_default() {
    let clusters = vec![vec![it("a"), it("b")], vec![it("c")]];
    assert_eq!(pick_representatives(&clusters, 1).len(), 2);
}

#[test]
fn asking_for_more_than_a_cluster_holds_takes_what_there_is() {
    let clusters = vec![vec![it("a"), it("b")], vec![it("c")]];
    assert_eq!(pick_representatives(&clusters, 5).len(), 3);
}
