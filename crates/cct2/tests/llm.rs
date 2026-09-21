//! Tests for the model-call layer.
//!
//! These pin the three faults the port found, each of which was invisible
//! because another one hid it.

use cct2::llm::anthropic_text;

#[test]
fn a_thinking_block_before_the_answer_does_not_hide_it() {
    // MiniMax-M2.7's real shape. The Python read content[0]["text"] and would
    // have raised here; it never did, because the provider string it used was
    // invalid and no response ever came back.
    let reply = serde_json::json!({
        "content": [
            {"type": "thinking", "thinking": "The user asks…", "signature": "x"},
            {"type": "text", "text": "{\"AAPL\":{\"sentiment\":\"bullish\"}}"}
        ],
        "stop_reason": "end_turn"
    });
    assert_eq!(
        anthropic_text(&reply).as_deref(),
        Some("{\"AAPL\":{\"sentiment\":\"bullish\"}}")
    );
}

#[test]
fn a_plain_text_first_reply_still_works() {
    // BigModel's shape, and what the Python assumed universally.
    let reply = serde_json::json!({"content": [{"type": "text", "text": "hello"}]});
    assert_eq!(anthropic_text(&reply).as_deref(), Some("hello"));
}

#[test]
fn a_block_without_a_type_but_with_text_is_accepted() {
    let reply = serde_json::json!({"content": [{"text": "hello"}]});
    assert_eq!(anthropic_text(&reply).as_deref(), Some("hello"));
}

#[test]
fn a_reply_that_is_only_thinking_yields_nothing() {
    // stop_reason=max_tokens with the whole budget spent thinking. Returning
    // the thinking text here would feed the model's scratchpad to the JSON
    // parser and, worse, could put its reasoning in front of a reader.
    let reply = serde_json::json!({
        "content": [{"type": "thinking", "thinking": "Let me consider…"}],
        "stop_reason": "max_tokens"
    });
    assert!(anthropic_text(&reply).is_none());
}

#[test]
fn an_empty_content_array_yields_nothing() {
    assert!(anthropic_text(&serde_json::json!({"content": []})).is_none());
}

#[test]
fn the_reply_text_is_trimmed() {
    let reply = serde_json::json!({"content": [{"type": "text", "text": "  {\"a\":1}\n"}]});
    assert_eq!(anthropic_text(&reply).as_deref(), Some("{\"a\":1}"));
}
