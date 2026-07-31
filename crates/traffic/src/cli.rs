//! CLI parsing and the LLM prompt.
//!
//! Extracted from main.rs so `cargo test` can reach argument handling without
//! spawning the binary or touching the network.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub origin: String,
    pub dest: String,
    pub via: Option<String>,
    pub deliver_to: Option<String>,
    pub account: String,
}

/// Parse CLI flags from an argv slice (no program name).
///
/// `--from` and `--to` are required, as in run.py:105-106. argparse exits 2 on
/// a missing required flag and on an unrecognised one; the caller maps these
/// errors onto that same exit code, which is also what the installer's smoke
/// probe checks for.
pub fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut origin: Option<String> = None;
    let mut dest: Option<String> = None;
    let mut via: Option<String> = None;
    let mut deliver_to: Option<String> = None;
    let mut account = "main".to_string();

    let mut i = 0;
    while i < argv.len() {
        let need = |i: usize| -> Result<String, String> {
            argv.get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{} requires a value", argv[i]))
        };
        match argv[i].as_str() {
            "--from" => {
                origin = Some(need(i)?);
                i += 2;
            }
            "--to" => {
                dest = Some(need(i)?);
                i += 2;
            }
            "--via" => {
                via = Some(need(i)?);
                i += 2;
            }
            "--deliver-to" => {
                deliver_to = Some(need(i)?);
                i += 2;
            }
            "--account" => {
                account = need(i)?;
                i += 2;
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }

    Ok(Args {
        origin: origin.ok_or("the following arguments are required: --from")?,
        dest: dest.ok_or("the following arguments are required: --to")?,
        via,
        deliver_to,
        account,
    })
}

/// The commute-advice prompt handed to the nullclaw agent.
///
/// Byte-identical to run.py:73-83. The trailing instruction about ncchoices is
/// load-bearing: without it the agent wraps replies in protocol markup that
/// would otherwise reach Telegram.
pub fn advice_prompt(route_label: &str, minutes: i64) -> String {
    format!(
        "你是通勤助理。以下是即時路況資料：\n\
         路線：{route_label}\n\
         預估行車時間：{minutes} 分鐘\n\n\
         請用繁體中文給出 1-2 句簡短的通勤建議。根據行車時間判斷路況：\n\
         - 若時間短（<25分鐘）：路況順暢，可簡單提醒\n\
         - 若時間中等（25-40分鐘）：提醒注意壅塞路段\n\
         - 若時間長（>40分鐘）：建議替代路線或出發時間調整\n\
         只回覆建議本身，不要重複路況資料。\
         只回覆純文字建議，勿附加 ncchoices、按鈕、選擇清單或任何標記。"
    )
}

/// Prefix non-empty sanitized advice with the bulb, matching run.py:91.
/// An empty reply drops the line entirely rather than emitting a bare 💡.
pub fn format_advice_line(advice: &str) -> Option<String> {
    if advice.is_empty() {
        None
    } else {
        Some(format!("💡 {advice}"))
    }
}
