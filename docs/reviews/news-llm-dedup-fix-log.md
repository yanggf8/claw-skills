# Review findings → fixes

- 日期：2026-07-13
- 來源：Claude test review + Claude impl review（皆 approve-with-changes）

## 實作

| 項 | 狀態 |
|----|------|
| M1 P2 union-find 傳遞誤刪 | **已改** greedy 保序（不做 CC） |
| m1 under-fill 無觀測 | **已加** `post_dedup_underfill` + `pick_min` 參數 |
| m2 trace 無 min_overlap | **已加** hints/post 皆記 |
| n1 重用 `_MARKER_RE` | **已做** |

## 測試

| 項 | 狀態 |
|----|------|
| M1 T07 AI 無 hint 假綠 | **已修** 餵 1/5 正向斷言 |
| M2 T03 pair 集合 | **已修** 精確 `assertEqual` |
| m1 custom cache 檔名 | **已修** |
| m2 post_dedup trace 欄位 | **已修** |
| m3 T18/T19 最終 lines | **已修** |
| m4 門檻/預設 on | **已修** |
| MC4 三則同事件 | **已加** |
| MC5 空/缺 title | **已加** |
| MC7 `#1,` 非法 marker | **已加** |
| underfill 測試 | **已加** |
| 重複 #N | **已加** |
| bridge 非傳遞 | **已加** |

## 驗證

```
python3 -m pytest news/scripts/test_run.py -q
# expect all green
```
