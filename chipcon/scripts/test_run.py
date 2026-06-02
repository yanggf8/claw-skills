from pathlib import Path
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
