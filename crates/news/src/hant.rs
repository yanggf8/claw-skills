//! Simplified characters out of the text a reader gets.
//!
//! The model sometimes writes Simplified although every source is Traditional
//! (2026-11-24 重大新聞: 光電布局争议延烧，蔣万安质疑…; 2026-11-23 a single
//! stray 现 in a Traditional line). This repairs the finished text, with no
//! model call, at the two points where text leaves for a reader: `run.rs` and
//! `shadow.rs`. Model replies, caches and traces keep what the model wrote.
//!
//! Running OpenCC over whole lines is not an option: on 1972 delivered lines
//! it damaged about 250 correct ones (台積電→臺積電, 公布→公佈, 蘇姿丰→蘇姿豐,
//! 獲准→獲準). So a line is judged character by character first, and OpenCC's
//! phrase context is used only where the evidence says Simplified.

use crate::render::{is_kana, markdown_visible_text, md_link_re, strip_links_keep_spacing};
use crate::trace::{clip_subprocess_text, log_trace};
use ferrous_opencc::{config::BuiltinConfig, OpenCC};
use serde_json::json;
use std::collections::HashSet;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::OnceLock;

/// Characters OpenCC maps that are also ordinary Traditional, so one of them
/// on its own says nothing (里 in 公里, 准 in 獲准, 丰 in 蘇姿丰).
///
/// Generated, not chosen: every key of OpenCC's `STCharacters.txt`
/// (ferrous-opencc 0.4.0) whose first value differs from the key and which is
/// Big5 level 1 — 0xA440..=0xC67E under Python's `big5` codec, the 5401 common
/// Traditional characters. Level-1 keys whose first value is the key itself
/// (只 面 系 周 克 布 …) are not mapped alone and need no entry. The test
/// `no_common_traditional_character_changes_on_its_own` runs all 5401.
const AMBIGUOUS: &str = "丑丰么于云仆伙余佣儿准几凶划占台吁吃后听咨咸唇坏夸尸岩岳峰帘干并床征愿扑托挂斗晒朴杠杰涌游灶痴皂秘筑粽群肴腌苧范荐虫虱辟郁采里雇霉";

/// Not Big5 level 1, yet written this way in Traditional text: 恒指/恒生, the
/// surname 涂 (涂權吉), 客庄, and 沄 in given names.
const PROPER: &str = "恒涂庄沄";

/// What S2tw writes where Taiwan keeps the original. Pairs, not characters:
/// 台 stays 台 in 台湾, yet 台风 must still become 颱風.
const TAIWAN_USAGE: [(char, char); 7] = [
    ('台', '臺'),
    ('布', '佈'),
    ('周', '週'),
    ('占', '佔'),
    ('雇', '僱'),
    ('污', '汙'),
    ('恒', '恆'),
];

const MAX_SAMPLES: usize = 5;
const SAMPLE_CHARS: usize = 240;

/// `text` with Simplified characters converted to Taiwan Traditional.
///
/// Link targets and backticked spans are never touched; line and character
/// counts never change; text with nothing Simplified in it comes back
/// byte-identical. Call it once per run, on the final text: a change is traced
/// as `simplified_converted`. A converter that cannot be built, or that
/// panics, costs the conversion and nothing else.
pub fn to_traditional_markdown(text: &str) -> String {
    match guarded(|| converters().map(|c| c.markdown(text))) {
        Ok(Some(c)) if c.lines > 0 => {
            log_trace(
                "simplified_converted",
                json!({"lines": c.lines, "chars": c.chars, "samples": c.samples}),
            );
            c.text
        }
        Ok(_) => text.to_string(),
        Err(e) => {
            log_trace("simplified_convert_unavailable", json!({"error": e}));
            text.to_string()
        }
    }
}

/// The converter library unwraps internally; a digest must not die of it.
fn guarded<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|p| {
        p.downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| p.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic".to_string())
    })
}

fn converters() -> Option<&'static Converters> {
    static C: OnceLock<Option<Converters>> = OnceLock::new();
    C.get_or_init(|| {
        Converters::load()
            .map_err(|e| log_trace("simplified_convert_unavailable", json!({"error": e})))
            .ok()
    })
    .as_ref()
}

struct Converters {
    /// Decides whether one character is Simplified. Not S2tw: that one also
    /// rewrites Traditional variants (峯 裏 着), which are not Simplified.
    s2t: OpenCC,
    /// Supplies the replacement.
    s2tw: OpenCC,
    traditional: OnceLock<HashSet<char>>,
}

struct Converted {
    text: String,
    lines: usize,
    chars: usize,
    samples: Vec<serde_json::Value>,
}

impl Converters {
    fn load() -> Result<Converters, String> {
        let open = |c| OpenCC::from_config(c).map_err(|e| e.to_string());
        Ok(Converters {
            s2t: open(BuiltinConfig::S2t)?,
            s2tw: open(BuiltinConfig::S2tw)?,
            traditional: OnceLock::new(),
        })
    }

    /// Simplified, and not something Traditional text also writes.
    fn simplified_only(&self, c: char) -> bool {
        c >= '\u{3400}' && !AMBIGUOUS.contains(c) && !PROPER.contains(c) && one(&self.s2t, c) != c
    }

    /// The Traditional form of some simplified-only character, and not a
    /// convertible character itself.
    ///
    /// Derived from the basic plane only: the 1140 Simplified characters
    /// beyond it are too rare to sway a count.
    fn traditional_only(&self, c: char) -> bool {
        self.traditional
            .get_or_init(|| {
                ('\u{3400}'..='\u{9FFF}')
                    .filter(|c| self.simplified_only(*c))
                    .map(|c| one(&self.s2t, c))
                    .filter(|t| one(&self.s2t, *t) == *t)
                    .collect()
            })
            .contains(&c)
    }

    fn markdown(&self, text: &str) -> Converted {
        let mut done = Converted {
            text: String::with_capacity(text.len()),
            lines: 0,
            chars: 0,
            samples: Vec::new(),
        };
        for (i, line) in text.split('\n').enumerate() {
            if i > 0 {
                done.text.push('\n');
            }
            let Some(new) = self.line(line) else {
                done.text.push_str(line);
                continue;
            };
            done.lines += 1;
            done.chars += line
                .chars()
                .zip(new.chars())
                .filter(|(a, b)| a != b)
                .count();
            if done.samples.len() < MAX_SAMPLES {
                done.samples
                    .push(json!({"before": sample(line), "after": sample(&new)}));
            }
            done.text.push_str(&new);
        }
        done
    }

    /// The converted line, or `None` when nothing in it changes.
    fn line(&self, line: &str) -> Option<String> {
        let spans = visible_spans(line);
        let visible: String = spans.iter().map(|(a, b)| &line[*a..*b]).collect();
        let mut converted = self.segment(&visible)?.into_iter();
        let mut out = String::with_capacity(line.len());
        let mut at = 0;
        for (a, b) in spans {
            out.push_str(&line[at..a]);
            out.extend(converted.by_ref().take(line[a..b].chars().count()));
            at = b;
        }
        out.push_str(&line[at..]);
        Some(out)
    }

    /// One line's visible text, converted, or `None` when nothing changes.
    fn segment(&self, text: &str) -> Option<Vec<char>> {
        let orig: Vec<char> = text.chars().collect();
        let simplified: Vec<bool> = orig.iter().map(|c| self.simplified_only(*c)).collect();
        let s = simplified.iter().filter(|b| **b).count();
        if s == 0 {
            return None;
        }
        // A Japanese line writes 国 会 学 as its own characters. One の in a
        // Chinese line does not make it Japanese.
        if orig.iter().filter(|c| is_kana(**c)).count() > s {
            return None;
        }
        let t = orig.iter().filter(|c| self.traditional_only(**c)).count();
        let line: Vec<char> = self.s2tw.convert(text).chars().collect();
        let out = merge(&orig, &simplified, &line, s > t, |c| one(&self.s2tw, c));
        (out != orig).then_some(out)
    }
}

/// The character `cc` writes for `c` alone; `c` itself if it would not be one
/// character.
fn one(cc: &OpenCC, c: char) -> char {
    let out = cc.convert(c.encode_utf8(&mut [0; 4]));
    let mut chars = out.chars();
    match (chars.next(), chars.next()) {
        (Some(t), None) => t,
        _ => c,
    }
}

/// `orig` with the converted `line` laid over it.
///
/// A mostly Simplified line takes the whole conversion, less
/// [`TAIWAN_USAGE`]; otherwise only its simplified-only positions change. A
/// conversion that changed the character count cannot be laid over anything,
/// so those positions are then converted one by one.
fn merge(
    orig: &[char],
    simplified: &[bool],
    line: &[char],
    mostly_simplified: bool,
    single: impl Fn(char) -> char,
) -> Vec<char> {
    let aligned = line.len() == orig.len();
    (0..orig.len())
        .map(|i| match (aligned, mostly_simplified, simplified[i]) {
            (true, true, _) if !TAIWAN_USAGE.contains(&(orig[i], line[i])) => line[i],
            (true, false, true) => line[i],
            (false, _, true) => single(orig[i]),
            _ => orig[i],
        })
        .collect()
}

/// Byte ranges of `line` a reader reads: all of it but link targets
/// (`](…)`) and backticked spans.
fn visible_spans(line: &str) -> Vec<(usize, usize)> {
    let mut hidden: Vec<(usize, usize)> = md_link_re()
        .captures_iter(line)
        .map(|c| {
            (
                c.get(1).expect("label").end(),
                c.get(0).expect("match").end(),
            )
        })
        .collect();
    let ticks: Vec<usize> = line.match_indices('`').map(|(i, _)| i).collect();
    hidden.extend(ticks.chunks_exact(2).map(|p| (p[0], p[1] + 1)));
    hidden.sort_unstable();

    let mut spans = Vec::new();
    let mut at = 0;
    for (a, b) in hidden {
        if a > at {
            spans.push((at, a));
        }
        at = at.max(b);
    }
    if at < line.len() {
        spans.push((at, line.len()));
    }
    spans
}

fn sample(line: &str) -> String {
    clip_subprocess_text(
        &markdown_visible_text(&strip_links_keep_spacing(line)),
        SAMPLE_CHARS,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEVEL1: &str = include_str!("../tests/fixtures/big5-level1.txt");

    fn cc() -> &'static Converters {
        converters().expect("converters")
    }

    fn conv(text: &str) -> String {
        cc().markdown(text).text
    }

    #[test]
    fn no_common_traditional_character_changes_on_its_own() {
        let level1: Vec<char> = LEVEL1.chars().filter(|c| *c != '\n').collect();
        assert_eq!(level1.len(), 5401);
        let changed: String = level1
            .into_iter()
            .filter(|c| cc().simplified_only(*c))
            .collect();
        assert_eq!(changed, "");
    }

    #[test]
    fn every_exception_is_one_the_converter_would_otherwise_change() {
        let exceptions: Vec<char> = AMBIGUOUS.chars().chain(PROPER.chars()).collect();
        assert_eq!(exceptions.len(), 65 + 4);
        let dead: String = exceptions
            .iter()
            .filter(|c| one(&cc().s2t, **c) == **c)
            .collect();
        assert_eq!(dead, "");
        let common: String = AMBIGUOUS.chars().filter(|c| !LEVEL1.contains(*c)).collect();
        assert_eq!(common, "");
    }

    #[test]
    fn a_traditional_variant_is_not_simplified() {
        for c in "峯裏着羣麪癡".chars() {
            assert!(!cc().simplified_only(c), "{c}");
        }
    }

    #[test]
    fn traditional_only_means_a_conversion_result_that_is_not_itself_convertible() {
        // 苧 is what 苎 converts to and is itself convertible (to 薴).
        for (c, expected) in [
            ('國', true),
            ('現', true),
            ('佈', false), // 布 is not mapped on its own
            ('後', false), // 后 is ambiguous
            ('国', false),
            ('苧', false),
            ('台', false),
            ('光', false),
        ] {
            assert_eq!(cc().traditional_only(c), expected, "{c}");
        }
    }

    #[test]
    fn a_mostly_simplified_line_is_converted_with_context() {
        for (before, after) in [
            (
                "光電布局争议延烧，蔣万安质疑行政院长卓荣泰「心中只有乌贼」",
                "光電布局爭議延燒，蔣萬安質疑行政院長卓榮泰「心中只有烏賊」",
            ),
            (
                "沈伯洋选前夜现身北市府前，蔣万安阵营抢先卡位凯道",
                "沈伯洋選前夜現身北市府前，蔣萬安陣營搶先卡位凱道",
            ),
            (
                "后来皇后的头发与日历写进历史，公里标准获准后众人干杯",
                "後來皇后的頭髮與日曆寫進歷史，公里標準獲准後眾人乾杯",
            ),
            (
                "台湾公布上周台积电占比，雇主忧污染，恒隆发布",
                "台灣公布上周台積電占比，雇主憂污染，恒隆發布",
            ),
        ] {
            assert_eq!(conv(before), after);
        }
    }

    #[test]
    fn a_stray_simplified_character_changes_nothing_else_in_its_line() {
        // 台積電 and 公布 and 獲准 are what a whole-line conversion would damage.
        for (before, after) in [
            (
                "台積電公布評估，2027 年可能出现 AI 進步浪潮，蘇姿丰獲准發言",
                "台積電公布評估，2027 年可能出現 AI 進步浪潮，蘇姿丰獲准發言",
            ),
            // The replacement comes from the line, not from the character: 发
            // alone is 發.
            ("設計師談理发與護髮趨勢", "設計師談理髮與護髮趨勢"),
            // S2tw, not S2t: 峰 stays, and 峯 is not touched either.
            ("赤峯黃金計劃发展", "赤峯黃金計劃發展"),
        ] {
            assert_eq!(conv(before), after);
        }
    }

    #[test]
    fn context_is_used_only_with_more_simplified_than_traditional() {
        // 并 is ambiguous: kept in a stray line, converted with context.
        assert_eq!(conv("國并现"), "國并現");
        assert_eq!(conv("国并现"), "國並現");
        assert_eq!(conv("國际并现"), "國際並現");
    }

    #[test]
    fn correct_text_comes_back_byte_identical() {
        for t in [
            "台積電公布財報",
            "蘇姿丰訪台",
            "新藥獲准上市",
            "6周年慶",
            "恒指收跌，恒生科技指數走弱",
            "赤峯黃金漲停",
            "離市區三公里",
            "新的里程碑",
            "政府干預市場",
            "市場占比三成",
            "雇主責任",
            "范冰冰",
            "余承東",
            "于謙",
            "游錫堃",
            "郁達夫",
            "桃園議員涂權吉、前市長涂醒哲",
            "客庄浪漫台三線",
            "新創公司執行長李沄",
            "拮据的家庭與胜肽保養品",
            "経済産業省の沢田氏が説明した",
            "日本の国会が学会と会談したこと",
            "OpenAI ships GPT-6 to everyone",
            "🚀📰 ⚠\u{fe0f}",
            "",
            "\n",
        ] {
            assert_eq!(conv(t), t);
        }
    }

    #[test]
    fn one_kana_in_a_chinese_line_does_not_stop_the_repair() {
        assert_eq!(conv("鮮の芋仙宣布现身"), "鮮の芋仙宣布現身");
        assert_eq!(conv("ののの宣布现身"), "ののの宣布现身");
    }

    #[test]
    fn link_targets_and_backticked_spans_are_never_touched() {
        for (before, after) in [
            (
                "- 蔣万安质疑乌贼 [🔗](https://ex.tw/万安/现身?q=乌贼)",
                "- 蔣萬安質疑烏賊 [🔗](https://ex.tw/万安/现身?q=乌贼)",
            ),
            (
                "- [乌贼争议](https://ex.tw/乌贼) 与 [长荣](https://ex.tw/长荣) 现身",
                "- [烏賊爭議](https://ex.tw/乌贼) 與 [長榮](https://ex.tw/长荣) 現身",
            ),
            ("出现 `job-现身:7` 后 `乌贼`", "出現 `job-现身:7` 後 `乌贼`"),
            ("出现 ` 乌贼", "出現 ` 烏賊"),
            // Nothing Simplified outside the target: the line is untouched.
            (
                "- 公布 [🔗](https://ex.tw/万安/现身)",
                "- 公布 [🔗](https://ex.tw/万安/现身)",
            ),
        ] {
            assert_eq!(conv(before), after);
        }
    }

    #[test]
    fn a_target_neither_counts_as_evidence_nor_lends_context() {
        // Three Simplified characters in the target; one stray in the title.
        // Counted, they would make the line mostly Simplified and 准 → 準.
        let c = cc().markdown("- 國際獲准并购 [🔗](https://ex.tw/乌贼长荣现身)");
        assert_eq!(c.text, "- 國際獲准并購 [🔗](https://ex.tw/乌贼长荣现身)");
        assert_eq!((c.lines, c.chars), (1, 1));
    }

    #[test]
    fn lines_are_judged_one_by_one_and_none_is_added_or_lost() {
        let before = "**標題**\n\n- 台積電公布\n- 乌贼争议，公布台风\r\n\n- 出现 AI\n";
        let c = cc().markdown(before);
        assert_eq!(
            c.text,
            "**標題**\n\n- 台積電公布\n- 烏賊爭議，公布颱風\r\n\n- 出現 AI\n"
        );
        assert_eq!((c.lines, c.chars), (2, 7));
        assert_eq!(
            c.samples,
            vec![
                json!({"before": "- 乌贼争议，公布台风", "after": "- 烏賊爭議，公布颱風"}),
                json!({"before": "- 出现 AI", "after": "- 出現 AI"}),
            ]
        );
    }

    #[test]
    fn samples_are_five_at_most_clipped_and_without_links() {
        let long = "现".repeat(300);
        let text = format!(
            "- 现 [🔗](https://ex.tw/1)  ⚠ 付費牆\n- [乌](https://ex.tw/2)\n{long}\n现3\n现4\n现5\n现6"
        );
        let c = cc().markdown(&text);
        assert_eq!((c.lines, c.chars), (7, 306));
        assert_eq!(
            c.samples,
            vec![
                json!({"before": "- 现 ⚠ 付費牆", "after": "- 現 ⚠ 付費牆"}),
                json!({"before": "- 乌", "after": "- 烏"}),
                json!({"before": "现".repeat(240), "after": "現".repeat(240)}),
                json!({"before": "现3", "after": "現3"}),
                json!({"before": "现4", "after": "現4"}),
            ]
        );
    }

    #[test]
    fn a_conversion_that_changes_the_length_falls_back_to_single_characters() {
        let orig: Vec<char> = "公布现身准".chars().collect();
        let simplified = [false, false, true, false, false];
        let longer: Vec<char> = "公佈現現身準".chars().collect();
        for mostly in [true, false] {
            let out = merge(&orig, &simplified, &longer, mostly, |c| {
                assert_eq!(c, '现');
                '現'
            });
            assert_eq!(out.iter().collect::<String>(), "公布現身准");
        }
    }

    #[test]
    fn a_panic_is_an_error_and_not_a_lost_digest() {
        assert_eq!(guarded(|| 7), Ok(7));
        assert_eq!(
            guarded(|| -> i32 { panic!("boom") }),
            Err("boom".to_string())
        );
        let n = 3;
        assert_eq!(
            guarded(|| -> i32 { panic!("boom {n}") }),
            Err("boom 3".to_string())
        );
    }
}
