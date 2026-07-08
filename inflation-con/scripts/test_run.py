from pathlib import Path
import sys
import types

import pytest

sys.path.insert(0, str(Path(__file__).parent))

import run


# ---- synthetic series helpers ---------------------------------------------

def monthly(values: list[float]) -> list[tuple[str, float]]:
    """Build a monthly (date, value) series, oldest first."""
    return [(f"2026-{(i % 12) + 1:02d}-01", v) for i, v in enumerate(values)]


def level_series(base: float, monthly_pct: float, n: int) -> list[tuple[str, float]]:
    """A price index compounding at `monthly_pct`% per month for n obs.

    A steady monthly_pct maps to an annualized rate of
    ((1 + p/100)**12 - 1) * 100 on any 3-mo or 6-mo window.
    """
    vals = [base]
    for _ in range(n - 1):
        vals.append(vals[-1] * (1.0 + monthly_pct / 100.0))
    return monthly(vals)


def daily(values: list[float]) -> list[tuple[str, float]]:
    return [(f"2026-06-{(i % 28) + 1:02d}", v) for i, v in enumerate(values)]


def state(core_pce, core_cpi=None, breakeven=None):
    if core_cpi is None:
        core_cpi = core_pce
    if breakeven is None:
        breakeven = daily([2.2] * 70)
    return {
        "core_pce": core_pce,
        "core_cpi": core_cpi,
        "breakeven_10y": breakeven,
    }


# monthly_pct that annualizes to a target: solve (1+p)**12 - 1 = target.
# ~0.16%/mo ≈ 2.0% annual, ~0.29%/mo ≈ 3.5% annual, ~0.33%/mo ≈ 4.0% annual.
FLAT_2PCT = 0.165
HOT_3PCT = 0.247   # ~3.0% annual
HOT_3_5PCT = 0.287  # ~3.5% annual
HOT_4PCT = 0.327   # ~4.0% annual


# ---- annualization math ----------------------------------------------------

def test_annualized_matches_compound_formula():
    s = level_series(100.0, HOT_3_5PCT, 12)
    a3 = run.annualized(s, 3)
    a6 = run.annualized(s, 6)
    assert a3 == pytest.approx(3.5, abs=0.1)
    assert a6 == pytest.approx(3.5, abs=0.1)


def test_annualized_none_when_too_short():
    assert run.annualized(monthly([1.0, 2.0]), 3) is None


# ---- status ladder ---------------------------------------------------------

def test_insufficient_data_under_7_obs():
    s = state(level_series(100.0, HOT_4PCT, 6))
    status, details = run.classify(s, "restrictive")
    assert status == "INSUFFICIENT_DATA"
    assert details["core_pce_obs"] == 6


def test_ok_when_below_threshold():
    s = state(level_series(100.0, FLAT_2PCT, 12))
    status, details = run.classify(s, "neutral")
    assert status == "OK"


def test_ok_when_falling_even_if_recently_hot():
    # 6-mo window catches an old hot patch, but the last 3 months cool off →
    # 3-mo pace < 6-mo pace = disinflating = OK.
    hot = level_series(100.0, HOT_4PCT, 9)
    last = hot[-1][1]
    cool = [(f"2026-{m:02d}-01", last * (1.0 + FLAT_2PCT / 100.0) ** k)
            for k, m in enumerate([10, 11, 12], start=1)]
    s = state(hot + cool)
    status, details = run.classify(s, "restrictive")
    # 3-mo cools below 6-mo → OK with the falling reason.
    assert status == "OK"
    assert any("disinflat" in r for r in details["reasons"])


def test_watch_on_one_hot_print():
    # 3-mo hot (>=2.5%) but 6-mo stays cool → WATCH.
    cool = level_series(100.0, FLAT_2PCT, 9)
    last = cool[-1][1]
    hot = [(f"2026-{m:02d}-01", last * (1.0 + HOT_4PCT / 100.0) ** k)
           for k, m in enumerate([10, 11, 12], start=1)]
    s = state(cool + hot)
    status, details = run.classify(s, "neutral")
    assert status == "WATCH"


def test_yellow_persistent_above_target_but_not_red_levels():
    # Core PCE 3-mo & 6-mo ~3.0–3.4% (>=3.0 but <3.5) + core CPI hot → YELLOW.
    s = state(level_series(100.0, HOT_3PCT, 12))
    status, details = run.classify(s, "restrictive")
    assert status == "YELLOW"


def test_red_when_confirmed_with_breakeven_context():
    # Core PCE 3m & 6m >= 3.5%, core CPI confirms, breakeven >= 2.5%, not easing.
    s = state(
        level_series(100.0, HOT_4PCT, 12),
        breakeven=daily([2.6] * 70),
    )
    status, details = run.classify(s, "restrictive")
    assert status == "RED"
    assert any("3.5%" in r for r in details["reasons"])


def test_boundary_yellow_when_levels_red_but_context_easing():
    # Levels reach RED (>=3.5%) and core CPI confirms, but breakeven < 2.5% and
    # stance is easing → context clause fails → stays YELLOW, with a note that
    # the human resolves via policy_stance.
    s = state(
        level_series(100.0, HOT_4PCT, 12),
        breakeven=daily([2.2] * 70),
    )
    status, details = run.classify(s, "easing")
    assert status == "YELLOW"
    assert any("RED" in r and "context" in r for r in details["reasons"])


def test_red_needs_core_cpi_confirmation():
    # Core PCE hot (>=3.5%) + breakeven high, but core CPI stays cool → not RED.
    s = state(
        level_series(100.0, HOT_4PCT, 12),
        core_cpi=level_series(100.0, FLAT_2PCT, 12),
        breakeven=daily([2.7] * 70),
    )
    status, details = run.classify(s, "restrictive")
    assert status != "RED"


# ---- fetch degradation -----------------------------------------------------

CFG_SERIES = dict(run.DEFAULT_SERIES)


def test_fetch_all_raises_when_core_pce_empty(monkeypatch):
    def fake(series_id, **kwargs):
        return [] if series_id == "PCEPILFE" else monthly([1.0, 2.0, 3.0])
    monkeypatch.setattr(run, "fred_fetch", types.SimpleNamespace(fetch_series=fake))
    with pytest.raises(RuntimeError):
        run.fetch_all(CFG_SERIES)


def test_fetch_all_degrades_on_secondary_series(monkeypatch):
    def fake(series_id, **kwargs):
        if series_id == "DGS10":
            raise RuntimeError("fred down")
        return monthly([100.0, 101.0, 102.0])
    monkeypatch.setattr(run, "fred_fetch", types.SimpleNamespace(fetch_series=fake))
    rows, warning = run.fetch_all(CFG_SERIES)
    assert warning is not None
    assert "DGS10" in warning
    assert rows["nominal_10y"] == []
    assert rows["core_pce"]  # primary succeeded


# ---- delivery is plain text ------------------------------------------------

def test_emit_sends_plain_text(monkeypatch):
    captured = {}

    def fake_deliver(chat_id, body, *, account="main", parse_mode="Markdown", **kwargs):
        captured["parse_mode"] = parse_mode
        captured["body"] = body
        return True

    monkeypatch.setattr(run, "deliver_or_fail", fake_deliver)
    monkeypatch.setattr(run, "emit_skill_status", lambda status: None)
    monkeypatch.setattr(run, "emit_trace", lambda: None)
    monkeypatch.setenv("NULLCLAW_JOB_ID", "job-xyz")

    s = state(level_series(100.0, HOT_4PCT, 12), breakeven=daily([2.6] * 70))
    status, details = run.classify(s, "restrictive")
    message, _ = run.format_message(status, details, {}, warning=None)

    class Args:
        deliver_to = "123"
        account = "main"

    run.emit(message, "ok", Args())
    assert captured["parse_mode"] is None
    assert "INFLATION-CON" in captured["body"]
    assert "SIGNAL-ONLY" in captured["body"]
    assert "job-xyz" in captured["body"]


def test_report_never_prescribes_a_trade():
    # Boundary guard: the RED report classifies but must not carry a buy/sell
    # verb or a share/dollar target.
    s = state(level_series(100.0, HOT_4PCT, 12), breakeven=daily([2.6] * 70))
    status, details = run.classify(s, "restrictive")
    message, _ = run.format_message(status, details, {}, warning=None)
    low = message.lower()
    for banned in ("buy ", "sell ", "un-gate", "allocate ", "shares", "$"):
        assert banned not in low, f"report leaked action verb: {banned!r}"
