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


def test_no_crash_when_levels_reach_red_and_breakeven_is_empty():
    """Regression: the YELLOW boundary note read be_last[1] unguarded.

    fetch_all is per-series tolerant — a failed T10YIE fetch stores an empty list
    and only warns. So "inflation hot enough to reach the RED levels" plus "the
    breakeven fetch failed this morning" reached an f-string that subscripted
    None and raised TypeError. Both conditions are needed, which is why it went
    unnoticed: neither alone shows it.
    """
    s = state(level_series(100.0, HOT_4PCT, 12), breakeven=[])
    status, details = run.classify(s, "restrictive")
    assert status == "YELLOW"
    # The note must still be produced — the point is that it survives missing data.
    assert any("RED" in r and "context" in r for r in details["reasons"])
    assert details["breakeven"] is None
    assert details["breakeven_rising"] is None


def test_breakeven_note_reports_the_value_when_it_is_present():
    """The normal path must keep printing the actual number, not a placeholder.

    Guarding the None case is only correct if it does not degrade the case that
    already worked.
    """
    s = state(level_series(100.0, HOT_4PCT, 12), breakeven=daily([2.21] * 70))
    status, details = run.classify(s, "restrictive")
    assert status == "YELLOW"
    note = next(r for r in details["reasons"] if "context" in r)
    assert "2.21" in note, note


def test_breakeven_shorter_than_the_lookback_still_classifies() -> None:
    """rising_over needs 64 observations; fewer must give None, not an error."""
    s = state(level_series(100.0, HOT_4PCT, 12), breakeven=daily([2.2] * 10))
    status, details = run.classify(s, "restrictive")
    assert status == "YELLOW"
    assert details["breakeven_rising"] is None
    assert details["breakeven"] == 2.2


def test_red_still_requires_breakeven_data_or_a_rising_trend():
    """An empty breakeven cannot satisfy the RED context clause.

    breakeven_ge is False and be_rising is None, so context_not_easing is False
    however hot the levels are. Pins that the crash fix did not accidentally let
    a missing series count as confirmation.
    """
    s = state(level_series(100.0, HOT_4PCT, 12), breakeven=[])
    status, _ = run.classify(s, "restrictive")
    assert status != "RED", "missing breakeven must not confirm a RED regime"


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


# ---- load_config + deploy path contracts (drift fix) -----------------------
# Test plan: docs/reviews/2026-07-13-inflation-con-drift-test-plan.md
# Codex execute: approve-with-changes (no silent loss of policy_stance).

import json
import os


def test_load_config_missing_path_returns_unclear(tmp_path):
    missing = tmp_path / "nope" / "config.json"
    cfg = run.load_config(missing)
    assert cfg["policy_stance"] == "unclear"
    for key in run.DEFAULT_SERIES:
        assert key in cfg["series"]
        assert cfg["series"][key] == run.DEFAULT_SERIES[key]


def test_load_config_empty_object_stance_unclear(tmp_path):
    p = tmp_path / "config.json"
    p.write_text("{}", encoding="utf-8")
    cfg = run.load_config(p)
    assert cfg["policy_stance"] == "unclear"
    assert set(cfg["series"]) == set(run.DEFAULT_SERIES)


def test_load_config_restrictive(tmp_path):
    p = tmp_path / "config.json"
    p.write_text(json.dumps({"policy_stance": "restrictive"}), encoding="utf-8")
    cfg = run.load_config(p)
    assert cfg["policy_stance"] == "restrictive"
    assert set(cfg["series"]) == set(run.DEFAULT_SERIES)


def test_load_config_stance_case_normalized(tmp_path):
    p = tmp_path / "config.json"
    p.write_text(json.dumps({"policy_stance": "RESTRICTIVE"}), encoding="utf-8")
    assert run.load_config(p)["policy_stance"] == "restrictive"


def test_load_config_invalid_stance_becomes_unclear(tmp_path):
    p = tmp_path / "config.json"
    p.write_text(json.dumps({"policy_stance": "hawkish"}), encoding="utf-8")
    assert run.load_config(p)["policy_stance"] == "unclear"


def test_load_config_series_partial_override(tmp_path):
    p = tmp_path / "config.json"
    p.write_text(
        json.dumps({
            "policy_stance": "neutral",
            "series": {"core_pce": "CUSTOM_PCE"},
        }),
        encoding="utf-8",
    )
    cfg = run.load_config(p)
    assert cfg["series"]["core_pce"] == "CUSTOM_PCE"
    assert cfg["series"]["core_cpi"] == run.DEFAULT_SERIES["core_cpi"]
    assert cfg["policy_stance"] == "neutral"


def test_load_config_unknown_keys_ignored_safely(tmp_path):
    p = tmp_path / "config.json"
    p.write_text(
        json.dumps({
            "policy_stance": "easing",
            "_policy_stance_note": "human note",
            "extra": 1,
        }),
        encoding="utf-8",
    )
    cfg = run.load_config(p)
    assert cfg["policy_stance"] == "easing"
    assert cfg.get("extra") == 1


def test_load_config_invalid_json_raises(tmp_path):
    p = tmp_path / "config.json"
    p.write_text("{not-json", encoding="utf-8")
    with pytest.raises(json.JSONDecodeError):
        run.load_config(p)


def test_default_config_path_shape():
    expected = Path.home() / ".nullclaw" / "skills" / "inflation-con" / "config.json"
    assert run.DEFAULT_CONFIG == expected


def test_load_config_via_skills_symlink_layout(tmp_path, monkeypatch):
    """T10: absolute skills path resolves through symlink to repo config file."""
    home = tmp_path / "home"
    repo_skill = tmp_path / "repo" / "inflation-con"
    skills = home / ".nullclaw" / "skills"
    skills.mkdir(parents=True)
    repo_skill.mkdir(parents=True)
    cfg_path = repo_skill / "config.json"
    cfg_path.write_text(json.dumps({"policy_stance": "restrictive"}), encoding="utf-8")
    os.symlink(repo_skill, skills / "inflation-con")

    # DEFAULT_CONFIG is import-time; rebind to the fake skills path (do not
    # pretend Path.home patch rewrites it — load_config never calls Path.home).
    skills_cfg = home / ".nullclaw" / "skills" / "inflation-con" / "config.json"
    monkeypatch.setattr(run, "DEFAULT_CONFIG", skills_cfg)
    assert skills_cfg.is_symlink() or skills_cfg.parent.is_symlink()
    assert skills_cfg.resolve() == cfg_path.resolve()
    cfg = run.load_config(skills_cfg)
    assert cfg["policy_stance"] == "restrictive"


def test_load_config_symlink_layout_missing_config_unclear(tmp_path, monkeypatch):
    """T11: symlink deploy without config file → silent unclear (the bug we prevent)."""
    home = tmp_path / "home"
    repo_skill = tmp_path / "repo" / "inflation-con"
    skills = home / ".nullclaw" / "skills"
    skills.mkdir(parents=True)
    repo_skill.mkdir(parents=True)
    # deliberately no config.json in repo_skill
    os.symlink(repo_skill, skills / "inflation-con")
    monkeypatch.setattr(
        run,
        "DEFAULT_CONFIG",
        home / ".nullclaw" / "skills" / "inflation-con" / "config.json",
    )
    cfg = run.load_config(run.DEFAULT_CONFIG)
    assert cfg["policy_stance"] == "unclear"


def test_gitignore_scopes_inflation_con_config():
    """T15: red until implement adds exact ignore line (not a comment substring)."""
    root = Path(__file__).resolve().parents[2]
    lines = [
        ln.strip()
        for ln in (root / ".gitignore").read_text(encoding="utf-8").splitlines()
        if ln.strip() and not ln.strip().startswith("#")
    ]
    assert "inflation-con/config.json" in lines


def test_config_example_documents_policy_stance():
    root = Path(__file__).resolve().parents[1]
    example = json.loads((root / "config.example.json").read_text(encoding="utf-8"))
    assert "policy_stance" in example
    assert example["policy_stance"] in run.VALID_STANCES
    assert "_policy_stance_note" in example


def test_live_deploy_loads_restrictive_or_skip():
    """T12 host-gated acceptance for symlink deploy.

    Default: skip if still a pre-fix real copy (not red noise in unit runs).
    Force red-before-deploy / hard accept with:
      INFLATION_CON_REQUIRE_DEPLOY=1 pytest ... -k live_deploy
    """
    skill = Path.home() / ".nullclaw" / "skills" / "inflation-con"
    require = os.environ.get("INFLATION_CON_REQUIRE_DEPLOY") == "1"
    if not skill.exists():
        if require:
            pytest.fail("INFLATION_CON_REQUIRE_DEPLOY=1 but skill path missing")
        pytest.skip("no live inflation-con deploy")
    cfg_path = run.DEFAULT_CONFIG
    if not skill.is_symlink():
        if require:
            pytest.fail(
                "INFLATION_CON_REQUIRE_DEPLOY=1: inflation-con is still a COPY; "
                "expected symlink into repo with restrictive config"
            )
        if cfg_path.exists():
            cfg = run.load_config(cfg_path)
            assert cfg["policy_stance"] in run.VALID_STANCES
        pytest.skip("live skill is still a copy; deploy not executed yet")
    # Post-deploy acceptance (always hard when symlink).
    assert cfg_path.resolve().is_file(), "symlink deploy missing config.json"
    repo = Path(__file__).resolve().parents[1]
    assert str(cfg_path.resolve()).startswith(str(repo.resolve())), (
        f"config resolves outside repo: {cfg_path.resolve()}"
    )
    cfg = run.load_config(cfg_path)
    assert cfg["policy_stance"] == "restrictive", cfg


def test_chipcon_remains_copy_when_present():
    """T13 scope guard: do not convert chipcon in this workstream."""
    chip = Path.home() / ".nullclaw" / "skills" / "chipcon"
    if not chip.exists():
        pytest.skip("no live chipcon")
    assert chip.is_dir()
    assert not chip.is_symlink()
