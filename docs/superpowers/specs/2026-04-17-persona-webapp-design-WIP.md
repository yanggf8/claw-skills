# Persona 管理平台 — 設計進度（WIP）

**狀態：** 腦力激盪進行中，尚未完成設計。換機器後可接手繼續。

**日期：** 2026-04-17
**使用技能：** superpowers:brainstorming

---

## 已確認的需求

透過 AskUserQuestion 與 user 互動確認：

| 項目 | 決定 |
|------|------|
| 主要使用者 | 你 + 作者們（如 Ping W.）都能登入 |
| 認證方式 | OAuth (Google/GitHub) |
| 功能範圍 | Persona 欄位編輯、寫作風格預覽、Editorial plan 管理、發佈歷史查看、系統管理建議 |
| 後端架構 | SPA + Workers API |
| 前端框架 | React + Tailwind |
| 部署方式 | Cloudflare Pages + Workers |
| 架構方案 | **方案 A — Cloudflare Full-Stack**（全在 CF 生態內，Turso 複用） |

---

## 已展示的設計 Section

### ✅ Section 1: 系統架構總覽（user 已 OK）

**架構**：
```
React SPA (Cloudflare Pages)
    ↕ HTTPS
Cloudflare Workers API (Hono)
    ↕ libsql HTTP
Turso (libsql)
```

**4 個頁面**：
1. Persona Editor — 編輯 role, expression, mental_models, heuristics, antipatterns
2. Writing Preview — 用 persona + LLM 生範例段落預覽聲音
3. Editorial Plan — 管理 angle, lens, direction, key_question
4. Publish History — 已發佈文章清單

**2 個角色**：
| Role | Persona Editor | Writing Preview | Editorial Plan | Publish History |
|------|----------------|-----------------|----------------|-----------------|
| admin (你) | ✅ 全部 CRUD | ✅ | ✅ 全部 CRUD | ✅ |
| author (Ping W.) | ✏️ 自己的 only | ✅ | 👁️ Read + 自己的 | 👁️ Read only |

### 🟡 Section 2: 資料模型（展示了但 user 尚未確認 — 在此中斷）

**既有表直接複用（不改 schema）**：
- `persona` — slug, role, name, expression, mental_models, heuristics, antipatterns, limits
- `persona_history` — 發佈記錄
- `editorial_plan` — angle, lens, direction, key_question, status

**新增表**：
```sql
CREATE TABLE user (
  id INTEGER PRIMARY KEY,
  email TEXT UNIQUE NOT NULL,
  oauth_provider TEXT,  -- 'google' | 'github'
  oauth_sub TEXT,
  role TEXT,  -- 'admin' | 'author'
  persona_slug TEXT,  -- NULL for admin
  created_at TEXT
);

CREATE TABLE audit_log (
  id INTEGER PRIMARY KEY,
  user_id INTEGER,
  action TEXT,
  entity TEXT,
  entity_id TEXT,
  diff_json TEXT,
  created_at TEXT
);
```

**關鍵附帶修復**：
- 檔案：`mindfulness-spirit/scripts/run.py:712`
- 目前問題：`expression`、`mental_models`、`heuristics` 被 resolve 但**未注入** writer_prompt
- 修復：此專案實作時一併注入這三個欄位，否則網站編輯了文章風格也不會變

**待確認的問題**：audit_log 要不要加？

---

## 尚未展示的設計 Section

換機器後繼續這幾個：

3. **Section 3: API 端點設計** — Hono routes, request/response shapes, auth middleware
4. **Section 4: OAuth 流程** — 如何自建 JWT、CF Workers 怎麼 handle callback、session 怎麼管
5. **Section 5: 寫作風格預覽機制** — 預覽用哪個 LLM、prompt 怎麼組、cost 怎麼控
6. **Section 6: 錯誤處理與測試策略**

---

## 換機器後接手步驟

1. `cd /home/yanggf/a/claw-skills`
2. `git pull`（如果你有 push 這份 WIP）
3. 讀這份檔案：`docs/superpowers/specs/2026-04-17-persona-webapp-design-WIP.md`
4. 讀這份檔案裡「已確認的需求」和「Section 1 / 2」
5. 告訴 Claude：「繼續設計 Section 3（API 端點設計）」
6. Claude 應該用 `superpowers:brainstorming` 的狀態繼續，到 Section 6 結束後寫正式 spec 並轉交 writing-plans

---

## 其他產出（已完成）

- **Excel 問卷**：`ping-w-questionnaire.xlsx`（要寄給 Ping 填）
- **問卷生成腳本**：`tmp_build_questionnaire.py`
