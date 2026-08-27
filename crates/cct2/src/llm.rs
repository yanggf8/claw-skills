//! Calling the two models.

use crate::json::extract;

/// M2.7 until 2026-08-27. It intermittently spent the whole `MAX_TOKENS` budget
/// on its thinking block and returned no text, so the report shipped backup-only
/// — measured 1 run in 3. The nullclaw agents had already moved to M3 on
/// 2026-06-30 against the same symptom. A host with no `config.json` falls back
/// to this constant, so it tracks the config rather than lagging behind it.
pub const DEFAULT_PRIMARY_MODEL: &str = "MiniMax-M3";
pub const DEFAULT_BACKUP_MODEL: &str = "GLM-5.1";
/// Both providers stream; a long reasoning pass legitimately takes minutes.
const LLM_TIMEOUT_S: u64 = 120;

/// Both upstreams speak the Anthropic messages API, so one client serves both.
///
/// The Python called MiniMax through `nullclaw agent --provider
/// anthropic-custom:minimax`, and that provider string is not valid — nullclaw
/// requires an absolute URL after `anthropic-custom:` and rejects it with
///
/// ```text
/// Config error: … must be absolute http(s) URLs …
/// ```
///
/// so the primary model never answered a single run. Going direct removes the
/// subprocess, the provider-string indirection, and that entire class of
/// failure. Keys come from the same nullclaw config the agent would have read.
#[derive(Debug, Clone)]
pub struct Endpoint {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

pub const MINIMAX_BASE: &str = "https://api.minimax.io/anthropic";
pub const BIGMODEL_BASE: &str = "https://open.bigmodel.cn/api/anthropic";
/// Free tier, tried once when the configured model answers 429.
const OVERLOAD_FALLBACK_MODEL: &str = "glm-4-flash";

fn log(msg: &str) {
    eprintln!("[cct2] {msg}");
}

/// `export KEY=value` lines out of `~/.secrets`.
pub fn load_secrets() -> Vec<(String, String)> {
    let path = match std::env::var("HOME") {
        Ok(h) => format!("{h}/.secrets"),
        Err(_) => return Vec::new(),
    };
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("export ")?;
            let (k, v) = rest.split_once('=')?;
            Some((
                k.trim().to_string(),
                v.trim().trim_matches(|c| c == '\'' || c == '"').to_string(),
            ))
        })
        .collect()
}

/// `~/.secrets`, then the environment, then nullclaw's glm-cn provider config.
pub fn bigmodel_key() -> Option<String> {
    if let Some((_, v)) = load_secrets()
        .into_iter()
        .find(|(k, _)| k == "BIGMODEL_API_KEY")
    {
        if !v.is_empty() {
            return Some(v);
        }
    }
    if let Ok(v) = std::env::var("BIGMODEL_API_KEY") {
        if !v.is_empty() {
            return Some(v);
        }
    }
    let cfg_path = std::env::var("CLAW_CONFIG").unwrap_or_else(|_| {
        format!(
            "{}/.nullclaw/config.json",
            std::env::var("HOME").unwrap_or_default()
        )
    });
    let text = std::fs::read_to_string(cfg_path).ok()?;
    let cfg: serde_json::Value = serde_json::from_str(&text).ok()?;
    cfg.get("models")?
        .get("providers")?
        .get("glm-cn")?
        .get("api_key")?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Look up an api_key in nullclaw's provider table.
fn provider_key(cfg: &serde_json::Value, name: &str) -> Option<String> {
    cfg.get("models")?
        .get("providers")?
        .get(name)?
        .get("api_key")?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn nullclaw_config() -> Option<serde_json::Value> {
    let path = std::env::var("CLAW_CONFIG").unwrap_or_else(|_| {
        format!("{}/.nullclaw/config.json", std::env::var("HOME").unwrap_or_default())
    });
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Resolve the two endpoints.
///
/// Keys are read from nullclaw's own provider table, so there is one place to
/// rotate them and this skill does not introduce a second. `~/.secrets` and the
/// environment still win for BigModel, preserving the Python's order.
pub fn endpoints(primary_model: &str, backup_model: &str) -> (Option<Endpoint>, Option<Endpoint>) {
    let cfg = nullclaw_config();

    let minimax_key = cfg
        .as_ref()
        .and_then(|c| provider_key(c, &format!("anthropic-custom:{MINIMAX_BASE}")))
        .or_else(|| cfg.as_ref().and_then(|c| provider_key(c, "minimax")));

    let bigmodel_key = load_secrets()
        .into_iter()
        .find(|(k, _)| k == "BIGMODEL_API_KEY")
        .map(|(_, v)| v)
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var("BIGMODEL_API_KEY").ok().filter(|v| !v.is_empty()))
        .or_else(|| cfg.as_ref().and_then(|c| provider_key(c, &format!("anthropic-custom:{BIGMODEL_BASE}"))))
        .or_else(|| cfg.as_ref().and_then(|c| provider_key(c, "glm-cn")));

    let base = |env: &str, default: &str| std::env::var(env).unwrap_or_else(|_| default.to_string());

    (
        minimax_key.map(|api_key| Endpoint {
            base_url: base("CCT2_MINIMAX_BASE", MINIMAX_BASE),
            api_key,
            model: primary_model.to_string(),
        }),
        bigmodel_key.map(|api_key| Endpoint {
            base_url: base("CCT2_BIGMODEL_BASE", BIGMODEL_BASE),
            api_key,
            model: backup_model.to_string(),
        }),
    )
}

/// Token budget per reply.
///
/// The Python used 512, which suits a model that answers directly. MiniMax-M2.7
/// emits a `thinking` block first, and on the real five-ticker prompt it spends
/// 1775 output tokens reaching an answer — measured, not guessed. At 512 and at
/// 2048 the reply came back `stop_reason: max_tokens` carrying a thinking block
/// and no text at all, which is indistinguishable from the model declining.
///
/// 4096 leaves headroom above the measured figure. Both endpoints are billed on
/// tokens produced, not on the ceiling, so a generous limit costs nothing when
/// the model stops early.
const MAX_TOKENS: u32 = 4096;

fn post_once(ep: &Endpoint, model: &str, prompt: &str) -> Result<serde_json::Value, u16> {
    let body = serde_json::json!({
        "model": model,
        "max_tokens": MAX_TOKENS,
        "messages": [{"role": "user", "content": prompt}],
    });
    let resp = claw_core::http::agent(std::time::Duration::from_secs(LLM_TIMEOUT_S))
        .post(&format!("{}/v1/messages", ep.base_url))
        .set("x-api-key", &ep.api_key)
        .set("anthropic-version", "2023-06-01")
        .set("Content-Type", "application/json")
        .send_string(&body.to_string())
        .map_err(|e| match e {
            ureq::Error::Status(code, _) => code,
            ureq::Error::Transport(_) => 0,
        })?;
    serde_json::from_str(&resp.into_string().map_err(|_| 0u16)?).map_err(|_| 0u16)
}

/// Pull the reply text out of an Anthropic-shaped response.
///
/// The first block with `type == "text"`, not `content[0]`. MiniMax-M2.7 puts a
/// `thinking` block first and the answer second:
///
/// ```text
/// [0] thinking  The user asks: "Reply with exactly this JSON…
/// [1] text      {"AAPL":{"sentiment":"bullish",…
/// ```
///
/// The Python read `content[0]["text"]`, which raises on that shape. It never
/// did, because the provider string it used for MiniMax was invalid and no
/// response ever came back. This is the half that would have broken the moment
/// the other half started working.
pub fn anthropic_text(payload: &serde_json::Value) -> Option<String> {
    let blocks = payload.get("content")?.as_array()?;
    blocks
        .iter()
        .find(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
        .or_else(|| blocks.iter().find(|b| b.get("text").is_some()))?
        .get("text")?
        .as_str()
        .map(|s| s.trim().to_string())
}

/// One model's answer, or None with the reason on stderr.
///
/// A 429 retries once on the free tier — BigModel's overload response. MiniMax
/// has no such tier, and asking it for `glm-4-flash` would be nonsense, so the
/// retry is scoped to the endpoint that offers it.
pub fn ask(ep: &Endpoint, label: &str, prompt: &str) -> Option<serde_json::Value> {
    match post_once(ep, &ep.model, prompt) {
        Ok(v) => {
            let text = anthropic_text(&v);
            if text.is_none() {
                // Say why. "no text block" alone sent me chasing the wrong
                // thing once: the cause was stop_reason=max_tokens, the whole
                // budget spent on a thinking block.
                let stop = v.get("stop_reason").and_then(|s| s.as_str()).unwrap_or("?");
                let kinds: Vec<&str> = v
                    .get("content")
                    .and_then(|c| c.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|b| b.get("type").and_then(|t| t.as_str()))
                            .collect()
                    })
                    .unwrap_or_default();
                log(&format!(
                    "WARN {label}: no text block (stop_reason={stop}, blocks={kinds:?})"
                ));
            }
            let parsed = extract(&text?);
            if parsed.is_none() {
                log(&format!("WARN {label}: no JSON in reply"));
            }
            parsed
        }
        Err(429) if ep.base_url.contains("bigmodel") && ep.model != OVERLOAD_FALLBACK_MODEL => {
            log(&format!(
                "WARN {label} 429 (overload), retrying with {OVERLOAD_FALLBACK_MODEL}"
            ));
            match post_once(ep, OVERLOAD_FALLBACK_MODEL, prompt) {
                Ok(v) => extract(&anthropic_text(&v)?),
                Err(c) => {
                    log(&format!("WARN {label} fallback failed: {c}"));
                    None
                }
            }
        }
        Err(c) => {
            log(&format!("WARN {label} failed: {c}"));
            None
        }
    }
}

/// Both models at once. Threads rather than a runtime: two blocking calls is
/// the whole of the concurrency, matching the Python's
/// ThreadPoolExecutor(max_workers=2).
pub fn run_dual(
    prompt: &str,
    primary: Option<&Endpoint>,
    backup: Option<&Endpoint>,
) -> (Option<serde_json::Value>, Option<serde_json::Value>) {
    std::thread::scope(|s| {
        let p = s.spawn(|| primary.and_then(|ep| ask(ep, "primary", prompt)));
        let b = s.spawn(|| backup.and_then(|ep| ask(ep, "backup", prompt)));
        (p.join().unwrap_or(None), b.join().unwrap_or(None))
    })
}
