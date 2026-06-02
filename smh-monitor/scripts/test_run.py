from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).parent))

import run


def cfg():
    return {"ticker": "SMH", "entry_price": 100.0, "position_usd": 3000.0}


def test_signal_boundaries():
    assert run.signal_for(-7.99) == "OK"
    assert run.signal_for(-8.0) == "REVIEW"
    assert run.signal_for(-12.0) == "REDUCE_REVIEW"
    assert run.signal_for(-15.0) == "EXIT_BIAS"
    assert run.signal_for(15.0) == "RAISE_STOP"
    assert run.signal_for(25.0) == "TAKE_PROFIT_1"
    assert run.signal_for(35.0) == "TAKE_PROFIT_2"


def test_render_includes_contract_markers():
    text = run.render(cfg(), 92.0, "manual", "manual")
    assert "狀態：REVIEW" in text
    assert "[skill-status:ok]" in text
    assert "[trace:" in text
