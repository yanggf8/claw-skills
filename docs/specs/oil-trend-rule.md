# Oil Trend Rule

Use a simple trend-shape rule, not an off-low snapshot.

Inputs (all from oilcon's daily WTI closes):
- Current WTI price
- 50-day moving average
- 50-day moving average direction (rising or not)
- Distance from the recent 60-day high

## States

### Uptrend
- Current price is above the 50-day moving average.
- The 50-day moving average is rising.
- Current price is not more than 10% below the 60-day high.

### Weakening Uptrend
- Current price is above the 50-day moving average.
- The 50-day moving average is rising.
- Current price is more than 10% below the 60-day high.

### Rollover
- Current price falls below the 50-day moving average, OR
- The 50-day moving average turns flat or down.

### No Uptrend
- Current price is below the 50-day moving average, AND
- The 50-day moving average is flat or falling.

**Important:** Distance from the 1-year low is context only. It is NOT proof of an uptrend.

## Quick human check (weekly)

Three questions: (1) WTI above 50MA? (2) 50MA rising? (3) WTI more than 10% below the 60-day high?

| Above 50MA | 50MA rising | >10% below high | State |
|---|---|---|---|
| yes | yes | no | Uptrend |
| yes | yes | yes | Weakening uptrend |
| no | — | — | Rollover / No uptrend |
| yes | no | — | Rollover risk |

## Use for JETS (second layer — separate judgment, not this rule)

This rule answers ONLY: is the oil price itself still in an uptrend? It does NOT decide JETS.

| Oil state | Next thing to look at |
|---|---|
| Uptrend | Judge whether demand-driven or supply-driven |
| Weakening uptrend | No rush; fuel-cost pressure not clearly worsening |
| Rollover / No uptrend | Fuel-cost pressure easing — friendlier to JETS cost side |

Then:

| Cause | What to verify |
|---|---|
| Demand-driven rise | RASM / fare yield / load factor |
| Supply-driven rise | Jet fuel crack spread / geopolitical supply risk |

Shortest form: oil trend = 50MA + 60d-high guard. JETS judgment then splits — demand side look at RASM, supply side look at jet-fuel cost shock. These are signals for the human to judge and verify before any action.
