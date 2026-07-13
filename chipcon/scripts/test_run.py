from pathlib import Path
import sys
import types

import pytest

sys.path.insert(0, str(Path(__file__).parent))

import run


def rows(start: float, n: int, step: float = 1.0):
    return [[f"2026-01-{(i % 28) + 1:02d}", start + i * step] for i in range(n)]


def state_with(smh, qqq=None, soxx=None):
    return {
        "SMH": smh,
        "QQQ": qqq if qqq is not None else rows(100, len(smh), 0.2),
        "SOXX": soxx if soxx is not None else rows(100, len(smh), 0.8),
    }


def test_insufficient_history_until_20_rows():
    status, details = run.classify(state_with(rows(100, 19)))
    assert status == "INSUFFICIENT_HISTORY"
    assert details["rows"] == 19


def test_ok_when_trend_is_intact():
    status, details = run.classify(state_with(rows(100, 60, 1.0)))
    assert status == "OK"
    assert not details["reasons"]


def test_red_when_below_50dma():
    smh = rows(100, 60, 1.0)
    smh[-1][1] = 80.0
    status, details = run.classify(state_with(smh))
    assert status == "RED"
    assert any("50DMA" in reason for reason in details["reasons"])


def test_yellow_when_underperforming_qqq():
    smh = rows(100, 60, 0.5)
    qqq = rows(100, 60, 1.5)
    status, details = run.classify(state_with(smh, qqq=qqq))
    assert status in {"YELLOW", "ORANGE", "RED"}
    assert details["rel_qqq5"] is not None


UNSORTED_YAHOO_ROWS = [
    ("2026-01-03", 103.0),
    ("2026-01-01", 101.0),
    ("2026-01-02", 102.0),
]

SORTED_YAHOO_ROWS = [
    ["2026-01-01", 101.0],
    ["2026-01-02", 102.0],
    ["2026-01-03", 103.0],
]

CFG = {"symbols": {"SMH": "SMH", "QQQ": "QQQ", "SOXX": "SOXX"}}


def test_update_state_yahoo_success_sorts_and_shapes(monkeypatch):
    def fake_fetch_history(symbol, **kwargs):
        return list(UNSORTED_YAHOO_ROWS)

    monkeypatch.setattr(
        run,
        "oil_fetch",
        types.SimpleNamespace(fetch_history=fake_fetch_history),
        raising=False,
    )
    state, warning = run.update_state(CFG)

    assert warning is None
    assert state["SMH"] == SORTED_YAHOO_ROWS
    assert state["QQQ"] == SORTED_YAHOO_ROWS
    assert state["SOXX"] == SORTED_YAHOO_ROWS


def test_update_state_raises_when_smh_fetch_errors(monkeypatch):
    def fake_fetch_history(symbol, **kwargs):
        if symbol == "SMH":
            raise RuntimeError("yahoo down")
        return list(UNSORTED_YAHOO_ROWS)

    monkeypatch.setattr(
        run,
        "oil_fetch",
        types.SimpleNamespace(fetch_history=fake_fetch_history),
        raising=False,
    )
    with pytest.raises(RuntimeError):
        run.update_state(CFG)


def test_update_state_raises_when_smh_empty(monkeypatch):
    def fake_fetch_history(symbol, **kwargs):
        if symbol == "SMH":
            return []
        return list(UNSORTED_YAHOO_ROWS)

    monkeypatch.setattr(
        run,
        "oil_fetch",
        types.SimpleNamespace(fetch_history=fake_fetch_history),
        raising=False,
    )
    with pytest.raises(RuntimeError):
        run.update_state(CFG)


def test_update_state_degrades_on_secondary_ticker_error(monkeypatch):
    def fake_fetch_history(symbol, **kwargs):
        if symbol == "QQQ":
            raise RuntimeError("yahoo down")
        return list(UNSORTED_YAHOO_ROWS)

    monkeypatch.setattr(
        run,
        "oil_fetch",
        types.SimpleNamespace(fetch_history=fake_fetch_history),
        raising=False,
    )
    state, warning = run.update_state(CFG)

    assert "yahoo fetch QQQ" in warning
    assert state["QQQ"] == []
    assert state["SMH"] == SORTED_YAHOO_ROWS


def test_update_state_warns_on_secondary_empty(monkeypatch):
    def fake_fetch_history(symbol, **kwargs):
        if symbol == "SOXX":
            return []
        return list(UNSORTED_YAHOO_ROWS)

    monkeypatch.setattr(
        run,
        "oil_fetch",
        types.SimpleNamespace(fetch_history=fake_fetch_history),
        raising=False,
    )
    state, warning = run.update_state(CFG)

    assert warning is not None
    assert "SOXX" in warning
    assert "no rows" in warning
    assert state["SOXX"] == []
    assert state["SMH"] == SORTED_YAHOO_ROWS


# Real Stooq anti-scraping garbage that broke Telegram Markdown parsing
# (HTTP 400 "can't parse entities") in the live 2026-07-08 chipcon run.
STOOQ_GARBAGE_WARN = (
    'price fetch SMH: parse error: bad close '
    '\'"0")).join("");if(x.startsWith(t))break;n++}const r=await fetch("/__verify"'
)


def test_emit_sends_plain_text_so_garbage_warning_cannot_break_delivery(monkeypatch):
    # The chipcon report is plain text (status underscores + possibly Stooq
    # anti-scraping garbage in the WARN). emit() must send with parse_mode=None
    # so Telegram never tries to parse it as Markdown ("can't parse entities"
    # HTTP 400 that flipped the run to degraded on 2026-07-08).
    captured = {}

    def fake_deliver(chat_id, body, *, account="main", parse_mode="Markdown", **kwargs):
        captured["parse_mode"] = parse_mode
        captured["body"] = body
        return True

    monkeypatch.setattr(run, "deliver_or_fail", fake_deliver)
    monkeypatch.setattr(run, "emit_skill_status", lambda status: None)
    monkeypatch.setattr(run, "emit_trace", lambda: None)
    monkeypatch.setenv("NULLCLAW_JOB_ID", "job-xyz")

    message, _ = run.format_message(
        "INSUFFICIENT_HISTORY", {"rows": 5}, {}, warning=STOOQ_GARBAGE_WARN
    )

    class Args:
        deliver_to = "123"
        account = "main"

    run.emit(message, "degraded", Args())

    assert captured["parse_mode"] is None
    # The garbage is delivered verbatim (plain text) — no sanitization needed.
    assert "parse error" in captured["body"]
    # job_id is appended as plain text, not a backtick code span.
    assert "job-xyz" in captured["body"]
    assert "`job-xyz`" not in captured["body"]

def test_format_message_is_observation_not_exit_review():
    message, skill_status = run.format_message(
        "INSUFFICIENT_HISTORY", {"rows": 5}, {}, warning=None
    )
    assert skill_status == "ok"
    assert "觀測" in message
    assert "退場" not in message
    assert "出場" not in message
    assert "不是交易指令" in message
