//! Tests for the deterministic text layer, written against the Python's
//! observable behaviour and the reasons behind it.

use news::text::{cluster, dedup, extract_source_name, is_cjk_headline, latin_tokens,
                 pick_representatives, replacement_query, title_without_source, topic_words, Item};

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

#[test]
fn latin_tokens_keep_every_alnum_run_but_pure_digits() {
    // Unlike topic_words: no stopword list and no three-char minimum — the
    // enrichment validator must account for every Latin token a rewrite
    // introduces, so even short tokens are tracked.
    let t = latin_tokens("Meta 發布 Muse Spark 1.2 開放權重");
    assert!(t.contains("meta") && t.contains("muse") && t.contains("spark"));
    assert!(!t.contains("1"));
    assert!(!t.contains("2"));
    assert!(latin_tokens("GPT-5").contains("gpt"));
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

// ── the paywall replacement query ────────────────────────────────────────────

#[test]
fn a_standing_lead_in_is_dropped_from_the_search_query() {
    // The defect this fixes, 2026-09-16: Google News splits a query on `：` and
    // `|` and ANDs the fragments, so `獨家：` stops being a label and becomes a
    // required term only the outlet that ran the exclusive uses. The WSJ pick
    // below scored 0 items queried whole and 12 queried without the lead-in.
    assert_eq!(
        replacement_query("獨家：美國施壓墨西哥阻擋中國 AI 硬體出口 - 華爾街日報中文網"),
        "美國施壓墨西哥阻擋中國 AI 硬體出口"
    );
    assert_eq!(
        replacement_query(
            "Exclusive | U.S. Pressures Mexico to Box Out China\u{2019}s AI Hardware Exports - WSJ"
        ),
        "U.S. Pressures Mexico to Box Out China\u{2019}s AI Hardware Exports"
    );
}

#[test]
fn stacked_lead_ins_are_all_dropped() {
    // Real feed headline. One pass would leave `Opinion | …` and the `|` would
    // keep ANDing a term the story does not contain.
    assert_eq!(
        replacement_query("Video: Opinion | How China Sees the A.I. Race - The New York Times"),
        "How China Sees the A.I. Race"
    );
}

#[test]
fn a_bracketed_lead_in_is_dropped_but_a_work_title_is_not() {
    // 【社論】 is furniture; 《Apex英雄》 is what the story is about. Stripping the
    // second would delete the only distinctive token in the headline.
    assert_eq!(
        replacement_query("【社論】面對AI競爭 牧師可以用什麼餵養會眾空虛的心靈？"),
        "面對AI競爭 牧師可以用什麼餵養會眾空虛的心靈？"
    );
    assert_eq!(
        replacement_query("《Apex英雄》9/22聯動《快打旋風6》 推出限定春麗、隆、豪鬼等角色造型"),
        "《Apex英雄》9/22聯動《快打旋風6》 推出限定春麗、隆、豪鬼等角色造型"
    );
    assert_eq!(
        replacement_query("《晶片戰爭》作者出新書 - 天下雜誌"),
        "《晶片戰爭》作者出新書"
    );
}

#[test]
fn a_dangling_close_drops_the_paper_name_but_not_a_mid_headline_colon() {
    // Google strips the opening 《 from these, leaving a close behind. The
    // second case is the guard: a bare colon with an ordinary fragment before
    // it (`Bessent:`) is part of the headline and must survive.
    assert_eq!(
        replacement_query("華爾街日報》中國也認為AI可能毀滅人類 - 華爾街日報"),
        "中國也認為AI可能毀滅人類"
    );
    assert_eq!(
        replacement_query("Bessent: AI companies ‘could stop any time they want to’"),
        "Bessent: AI companies ‘could stop any time they want to’"
    );
}

#[test]
fn a_headline_that_opens_with_its_story_is_left_alone() {
    for t in [
        "Nvidia reports record datacenter revenue - Reuters",
        "台積電法說會上修全年展望 - 工商時報",
        "AI 面試成求職新常態 八成求職者偏好 AI 語音篩選",
    ] {
        assert_eq!(replacement_query(t), title_without_source(t), "{t}");
    }
}

#[test]
fn the_query_never_strips_itself_empty() {
    // A headline that is nothing but furniture has no searchable part; returning
    // the label is better than returning "" and searching for everything.
    for t in ["獨家：", "獨家 - 自由時報", "Video:"] {
        assert!(!replacement_query(t).trim().is_empty(), "{t}");
    }
}

// ── script, which routes the search edition ──────────────────────────────────

#[test]
fn the_script_is_judged_after_the_source_suffix_goes() {
    // Google appends a Chinese outlet name to an English headline; counting the
    // suffix would send the search to the wrong edition and return nothing.
    assert!(is_cjk_headline("獨家：美國施壓墨西哥阻擋中國 AI 硬體出口"));
    assert!(!is_cjk_headline("Nvidia beats - 自由時報"));
    assert!(is_cjk_headline("台積電法說會上修全年展望 - 工商時報"));
}
