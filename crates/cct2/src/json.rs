//! Pulling a JSON object out of an LLM reply.

/// Extract the outermost balanced `{...}` from text, tolerating markdown fences
/// and a trailing comma before a closing brace.
///
/// Ports `extract_json` from run.py:265. Models wrap answers in ```json fences
/// and leave trailing commas; both are common enough that failing on them would
/// throw away usable replies.
pub fn extract(text: &str) -> Option<serde_json::Value> {
    let mut s = text.trim();

    if s.starts_with("```") {
        let lines: Vec<&str> = s.lines().collect();
        let body = if lines.last().map(|l| l.trim()) == Some("```") {
            &lines[1..lines.len().saturating_sub(1)]
        } else {
            &lines[1..]
        };
        return extract_balanced(body.join("\n").trim());
    }
    // Reborrow so the non-fenced path reads the same way.
    s = s.trim();
    extract_balanced(s)
}

fn extract_balanced(s: &str) -> Option<serde_json::Value> {
    let chars: Vec<char> = s.chars().collect();
    let mut depth = 0usize;
    let mut start: Option<usize> = None;
    for (i, ch) in chars.iter().enumerate() {
        match ch {
            '{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    if let Some(st) = start {
                        let candidate: String = chars[st..=i].iter().collect();
                        if let Ok(v) = serde_json::from_str(&candidate) {
                            return Some(v);
                        }
                        if let Ok(v) = serde_json::from_str(&strip_trailing_commas(&candidate)) {
                            return Some(v);
                        }
                        start = None;
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Remove a comma that sits directly before a `}` or `]`, ignoring whitespace.
/// Equivalent to the Python's `re.sub(r",\s*([}\]])", r"\1", ...)`.
fn strip_trailing_commas(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == ',' {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && (chars[j] == '}' || chars[j] == ']') {
                i += 1; // drop the comma, keep the whitespace
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}
