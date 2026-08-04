//! Paths, tunables, and the shape of the default digest.
//!
//! Everything an operator can change without a code edit reads from the
//! environment here, so the knobs are in one place rather than scattered
//! through the call sites that use them.

use std::path::PathBuf;

pub fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

pub fn topics_file() -> PathBuf {
    home().join(".nullclaw/news-topics.json")
}
pub fn trace_file() -> PathBuf {
    home().join(".nullclaw/skill-traces.jsonl")
}
pub fn cache_dir() -> PathBuf {
    home().join(".nullclaw/.news-cache")
}
pub fn failure_log() -> PathBuf {
    home().join(".nullclaw/news-failures.log")
}

pub const CACHE_TTL_DAYS: u64 = 7;
/// 1 MiB, then rotate to `.1` and start again.
pub const FAILURE_LOG_MAX_BYTES: u64 = 1_048_576;

pub const LLM_CUSTOM_TOPIC_LIMIT: usize = 8;
pub const LLM_DEFAULT_TIMEOUT_SECS: u64 = 180;
pub const LLM_CUSTOM_TIMEOUT_SECS: u64 = 180;
pub const LLM_SECTION_TIMEOUT_SECS: u64 = 90;
pub const LLM_TRANSLATION_TIMEOUT_SECS: u64 = 60;
pub const TELEGRAM_RAW_CHUNK_LIMIT: usize = 3800;

/// Bumped when the AI substage prompt or its cached-output semantics change;
/// day-caches written under an older variant are simply never looked up.
pub const AI_SUBSTAGE_CACHE_VARIANT: &str = "default_ai_clustered_v5_post_dedup";

/// Each Level-2 half (or Level-3 quarter) gets less than the 90s the monolithic
/// call had — a half-size prompt should not need it.
pub const AI_SUBSTAGE_TIMEOUT_SECS: u64 = 60;

/// Independent pair-hint and post-dedup threshold. Four, not "3 plus a shared
/// entity" — that lower bar false-merges same-theme different events.
pub const LLM_DEDUP_HINT_OVERLAP: usize = 4;
pub const LLM_POST_DEDUP_OVERLAP: usize = 4;

// ── env-tunable knobs ────────────────────────────────────────────────────────

fn env_str(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

/// Python reads `float(os.environ.get(name, default))` and lets a bad value
/// raise at import. Rust falls back to the default instead: a typo in a cron
/// env should not stop the digest, and the trace records what was used.
fn env_f64(name: &str, default: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

/// `"0"` disables; anything else, including unset, enables.
fn env_flag(name: &str) -> bool {
    env_str(name, "1") != "0"
}

/// Deterministic, no network, before the LLM sees anything.
pub fn precheck_enabled() -> bool {
    env_flag("NEWS_PRECHECK")
}
pub fn precheck_decode_timeout() -> f64 {
    env_f64("NEWS_PRECHECK_DECODE_TIMEOUT", 5.0)
}
pub fn precheck_fetch_timeout() -> f64 {
    env_f64("NEWS_PRECHECK_FETCH_TIMEOUT", 5.0)
}
pub fn precheck_total_deadline() -> f64 {
    env_f64("NEWS_PRECHECK_DEADLINE", 25.0)
}
pub fn precheck_max_workers() -> usize {
    env_usize("NEWS_PRECHECK_WORKERS", 6)
}

/// Looking up a free replacement for a paywalled pick. Bounded so it can never
/// push a cron run past its kill window; disabled, the reader gets the
/// single-bullet 付費牆 note instead.
pub fn paywall_replace_enabled() -> bool {
    env_flag("NEWS_PAYWALL_REPLACE")
}
pub fn paywall_replace_deadline() -> f64 {
    env_f64("NEWS_PAYWALL_REPLACE_DEADLINE", 20.0)
}
pub fn paywall_replace_max() -> usize {
    env_usize("NEWS_PAYWALL_REPLACE_MAX", 4)
}
pub fn paywall_replace_sources() -> Vec<String> {
    env_str("NEWS_PAYWALL_REPLACE_SOURCES", "google,bing")
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}
pub fn paywall_replace_bing_mkt() -> String {
    env_str("NEWS_PAYWALL_REPLACE_BING_MKT", "en-US")
}

/// Soft pair hints injected into the selection prompt — never a hard drop.
pub fn llm_dedup_hints_enabled() -> bool {
    env_flag("NEWS_LLM_DEDUP_HINTS")
}
/// Hard dedup of the selected set, after marker validation and before precheck.
pub fn llm_post_dedup_enabled() -> bool {
    env_flag("NEWS_LLM_POST_DEDUP")
}

/// A synthetic timeout means the provider stalled after stream-start; one retry
/// usually lands in seconds. The retry gets a shorter budget than the original
/// so a wedged provider cannot spend the stall twice.
pub fn llm_retry_timeout_secs() -> u64 {
    env_usize("NEWS_LLM_RETRY_TIMEOUT", 30) as u64
}

// ── the default digest ───────────────────────────────────────────────────────

pub struct SectionSpec {
    pub key: &'static str,
    pub header: &'static str,
    pub limit: usize,
    pub fallback_limit: usize,
    pub pick: &'static str,
    pub focus: &'static str,
}

pub const SECTIONS: [SectionSpec; 3] = [
    SectionSpec {
        key: "ai",
        header: "**🤖 AI 人工智慧**",
        limit: 30,
        fallback_limit: 8,
        pick: "5-8",
        focus: "重大研究突破、政策變化、產品發布、產業併購、國安與監管等真正有影響力的 AI 新聞",
    },
    SectionSpec {
        key: "tech",
        header: "**💻 科技 & 半導體**",
        limit: 12,
        fallback_limit: 5,
        pick: "3-5",
        focus: "半導體、晶片、消費電子、太空科技與重要非 AI 科技新聞",
    },
    SectionSpec {
        key: "general",
        header: "**🌏 重大新聞**",
        limit: 8,
        fallback_limit: 3,
        pick: "2-3",
        focus: "最重大的非科技一般新聞",
    },
];

pub fn section(key: &str) -> Option<&'static SectionSpec> {
    SECTIONS.iter().find(|s| s.key == key)
}

/// How many items of each section reach the LLM prompt.
pub fn llm_item_limit(key: &str) -> usize {
    section(key).map(|s| s.limit).unwrap_or(0)
}

/// Told to the model in both the default-section and AI-substage prompts.
/// Same *event* collapses; same *theme* does not.
pub const DEDUP_RULES: &str = concat!(
    "- 同一則新聞如果有多個來源（標題講同一件事，例如同一財報／同一政策公告／",
    "同一研究報告／同一產品發布的不同改寫），只挑一則：\n",
    "  優先選免費來源（cnyes、TechNews、Yahoo新聞、MoneyDJ、工商時報、",
    "Reuters、AP、ScienceDaily、TechCrunch 等）\n",
    "  避開付費牆來源（WSJ、Bloomberg、FT、Nikkei、Barron's 等）\n",
    "  只有付費來源報導時才保留付費來源\n",
    "- 重複判斷以「事件本身」為準，不是「同產業主題」：\n",
    "  同事件（應合併）：同一份報告／同一公告／同一季財報／同一產品發布的多出口改寫，",
    "即使標題 hook 不同（例如「晶片回流夢碎」與「彭博爆晶片業拉警報」講同一危機）\n",
    "  不同事件（應保留）：主體或焦點不同（例如記憶體股技術性熊市 vs 輝達選擇權 vs ",
    "泛板塊震盪綜述可同時保留）"
);

/// Company and product names that read as ordinary English, and so get
/// translated when a model is not told otherwise.
///
/// Membership test: the name is made of ordinary words *and* has no settled
/// Chinese rendering. Apple and Microsoft fail the second half — 蘋果 and 微軟
/// are what a Taiwanese reader expects — so translating those is right and
/// they are deliberately absent. 「雪花」 for Snowflake is not.
///
/// One list, two uses: the prompt quotes from it and
/// [`crate::validate::dropped_protected_names`] checks the delivered digest
/// against it, so the instruction and the detector cannot drift apart.
pub const PROTECTED_NAMES: &[&str] = &[
    "Hugging Face",
    "Stability AI",
    "Character.AI",
    "Perplexity",
    "Anthropic",
    "Snowflake",
    "Databricks",
    "Palantir",
    "Salesforce",
    "Notion",
    "Discord",
    "Slack",
    "Stripe",
    "Figma",
];

/// How many of [`PROTECTED_NAMES`] the prompt quotes. Enough to establish the
/// pattern; the whole list would cost tokens the model does not need, since
/// the rule it has to generalise is stated outright.
const PROMPT_NAME_EXAMPLES: usize = 5;

/// Injected into every prompt that produces reader-facing headlines. Half
/// translated prose reads worse than either language alone, so the rule names
/// the four categories that may stay in English and forbids the rest.
///
/// The proper-noun clause is emphatic because the earlier wording ("可以保留
/// 英文原文") read as permission, and its examples were all names no reader
/// could mistake for common nouns — OpenAI, Google, Microsoft. A name built
/// out of ordinary English words falls off that cliff: on 2026-08-04 the AI
/// section shipped 「擁抱臉書執行長」 for Hugging Face's CEO. Keeping a name
/// in English is not a stylistic option, it is the only correct rendering.
pub static TRANSLATION_RULES_STRICT: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| {
        let examples = PROTECTED_NAMES
            .iter()
            .take(PROMPT_NAME_EXAMPLES)
            .copied()
            .collect::<Vec<_>>()
            .join("、");
        format!(
            "英文標題必須完整翻譯成繁體中文。\
             只有以下類別可以保留英文原文：公司名、人名、產品名、\
             既定技術術語（例如 AGI、GPU、API、LLM）。\
             專有名詞（公司名、人名、產品名）一律保留英文原文，嚴禁逐字意譯，\
             即使該名稱是由普通英文單字組成也一樣 —— \
             Hugging Face 必須維持 Hugging Face，不可寫成「擁抱臉書」或「抱抱臉」，\
             {examples} 等同理。\
             判斷方式：若該詞在句中指的是一家公司、一個人或一項產品，就保留英文，\
             不要管它字面上是什麼意思。\
             原標題點名的公司必須逐一留在譯文裡，不可併成籠統說法：\
             「Meta, Anthropic, Google, OpenAI to meet Trump officials」\
             要譯成「Meta、Anthropic、Google、OpenAI 將與川普政府官員會談」，\
             不可寫成「美國多家科技巨頭將與川普政府會談」——被點名的是誰本身就是新聞。\
             所有普通英文詞彙必須翻譯，包括但不限於副詞（increasingly、significantly、rapidly、notably、effectively）、\
             動詞、形容詞、連接詞。\
             輸出中不得保留任何非上述四類的英文單字。"
        )
    });

// ── theme classification ─────────────────────────────────────────────────────

pub const THEME_PRODUCT: &str = "產品發布";
pub const THEME_RESEARCH: &str = "研究突破";
pub const THEME_CAPITAL: &str = "產業資本";
pub const THEME_POLICY: &str = "政策監管";
pub const THEME_OTHER: &str = "其他";

/// Render order. `其他` is last because it is the residue, not a topic.
pub const THEME_RENDER_ORDER: [&str; 5] = [
    THEME_PRODUCT,
    THEME_RESEARCH,
    THEME_CAPITAL,
    THEME_POLICY,
    THEME_OTHER,
];

pub fn theme_heading(theme: &str) -> String {
    format!("▸ {theme}")
}

pub const CLASSIFIER_TIMEOUT_SECS: u64 = 10;
pub const THEME_MAX_BLOCKS: usize = 20;
/// Reserve the whole delivery deadline, not one attempt: telegram's is 30s and
/// covers a retry, and a long digest is delivered as several chunks in
/// sequence. Under-reserving would let the classifier push a slow delivery past
/// the cron kill. Four seconds of exit margin on top.
pub const THEME_DELIVERY_RESERVE_SECS: f64 = 34.0;
pub const THEME_TRIM_THRESHOLD: usize = 4000;

// ── cross-dedup ensemble ─────────────────────────────────────────────────────

/// Ceiling on how much of a section one cross-dedup pass may remove. A pass
/// that wants to drop more than this is disbelieved wholesale — refinement
/// should not be able to gut a section.
pub const CROSS_DEDUP_MAX_DROP_RATIO: f64 = 0.40;
/// Short on purpose: this is refinement, not a gate. Up to two attempts each.
pub const CROSS_DEDUP_TIMEOUT_SECS: u64 = 45;
pub const CROSS_DEDUP_MAX_SAMPLES: usize = 12;
/// Beyond this, concurrent agent calls just contend with each other.
pub const CROSS_DEDUP_MAX_INFLIGHT: usize = 3;
/// De-correlates sample start times, and with them the retry storms.
pub const CROSS_DEDUP_STAGGER_SECS: f64 = 0.35;
pub const CROSS_DEDUP_TOTAL_TIMEOUT_SECS: f64 = 120.0;

/// The real rollback lever for the whole layer.
pub fn cross_dedup_enabled() -> bool {
    std::env::var("NEWS_CROSS_DEDUP").unwrap_or_else(|_| "1".into()) != "0"
}

/// N: independent samples of the same prompt, run concurrently.
///
/// One sample is unreliable — measured per-sample recall on an obvious
/// duplicate pair is 40-60%, a fifth to a half of samples return no groups at
/// all, and a fifth to a third contain a false pair. Bootstrapping over
/// recorded runs puts N=7 with K=3 at recall 46%→68% and false pairs 20%→3%.
/// `NEWS_CROSS_DEDUP_N=1` collapses the ensemble but is not a bit-exact revert:
/// the survivor is still chosen by policy rather than by the model's "keep".
pub fn cross_dedup_samples() -> usize {
    cross_dedup_env_int("NEWS_CROSS_DEDUP_N", 7, Some(CROSS_DEDUP_MAX_SAMPLES))
}
/// K: votes a pair needs across those samples before it is believed.
pub fn cross_dedup_vote_k() -> usize {
    cross_dedup_env_int("NEWS_CROSS_DEDUP_K", 3, Some(cross_dedup_samples()))
}

fn cross_dedup_env_int(name: &str, default: usize, maximum: Option<usize>) -> usize {
    let v = std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v >= 1)
        .unwrap_or(default);
    match maximum {
        Some(m) => v.min(m),
        None => v,
    }
}
