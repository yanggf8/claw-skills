from pathlib import Path
import subprocess
import sys

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


def cp(code=0, stdout="", stderr=""):
    return subprocess.CompletedProcess(args=[], returncode=code, stdout=stdout, stderr=stderr)


def test_parse_price_history_tsv_groups_rows():
    parsed = run.parse_price_history_tsv(
        "SMH\t2026-06-01\t620.0\tstooq\n"
        "QQQ\t2026-06-01\t528.4\tstooq\n"
        "SMH\t2026-06-02\t625.18\tstooq\n"
    )
    assert parsed == {
        "SMH": [["2026-06-01", 620.0], ["2026-06-02", 625.18]],
        "QQQ": [["2026-06-01", 528.4]],
    }


def test_price_cli_resolution_prefers_env_config_then_path(monkeypatch):
    monkeypatch.setenv("CHIPCON_PRICE_CLI", "/tmp/price-env")
    assert run.price_cli_path({"price_cli_path": "/tmp/price-config"}) == "/tmp/price-env"

    monkeypatch.delenv("CHIPCON_PRICE_CLI")
    assert run.price_cli_path({"price_cli_path": "/tmp/price-config"}) == "/tmp/price-config"

    monkeypatch.setattr(run.shutil, "which", lambda name: "/usr/local/bin/price" if name == "price" else None)
    assert run.price_cli_path({}) == "/usr/local/bin/price"


def test_update_state_uses_price_cli_fetch_then_history(monkeypatch):
    calls = []

    def fake_run_price_cli(cfg, args):
        calls.append(args)
        if args[0] == "fetch":
            return cp(0, "SMH\t2026-06-02\t625.18\tstooq\n")
        return cp(0, (
            "SMH\t2026-06-01\t620.0\tstooq\n"
            "SMH\t2026-06-02\t625.18\tstooq\n"
            "QQQ\t2026-06-02\t528.4\tstooq\n"
            "SOXX\t2026-06-02\t290.1\tstooq\n"
        ))

    monkeypatch.setattr(run, "run_price_cli", fake_run_price_cli)
    state, warning = run.update_state({"symbols": {"SMH": "SMH", "QQQ": "QQQ", "SOXX": "SOXX"}})

    assert calls == [
        ["fetch", "SMH", "QQQ", "SOXX"],
        ["history", "SMH", "QQQ", "SOXX"],
    ]
    assert warning is None
    assert state["SMH"] == [["2026-06-01", 620.0], ["2026-06-02", 625.18]]


def test_update_state_degrades_on_price_cli_partial_fetch(monkeypatch):
    def fake_run_price_cli(cfg, args):
        if args[0] == "fetch":
            return cp(2, stderr="price fetch SOXX: no usable price data\n")
        return cp(0, "SMH\t2026-06-02\t625.18\tstooq\n")

    monkeypatch.setattr(run, "run_price_cli", fake_run_price_cli)
    state, warning = run.update_state({"symbols": {"SMH": "SMH"}})

    assert state["SMH"] == [["2026-06-02", 625.18]]
    assert "price fetch SOXX" in warning


def test_update_state_fails_on_price_cli_registry_error(monkeypatch):
    def fake_run_price_cli(cfg, args):
        return cp(1, stderr="price: turso registry unavailable\n")

    monkeypatch.setattr(run, "run_price_cli", fake_run_price_cli)
    try:
        run.update_state({"symbols": {"SMH": "SMH"}})
    except RuntimeError as exc:
        assert "turso registry unavailable" in str(exc)
    else:
        raise AssertionError("expected registry failure to raise RuntimeError")


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
