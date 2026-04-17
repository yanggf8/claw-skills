# Persona 管理平台 — 設計進度（WIP）

**狀態：** 設計完成，待實作
**日期：** 2026-04-17
**設計文件（含圖表）：** `docs/superpowers/specs/persona-webapp-design.html`

---

## 已確認的需求

| 項目 | 決定 |
|------|------|
| 主要使用者 | 你 + 作者們（Ping W.、Liko） |
| 認證方式 | OAuth (Google/GitHub)，Invite-only |
| 功能範圍 | Persona 欄位編輯、寫作風格預覽、Editorial plan 管理、發佈歷史查看 |
| 後端架構 | SPA + Workers API |
| 前端框架 | React + Tailwind |
| 部署方式 | Cloudflare Pages + Workers |
| 架構方案 | **方案 A — Cloudflare Full-Stack**（全在 CF 生態內，Turso 複用） |
| LLM 呼叫 | CF Workers **不呼叫任何 LLM**，preview 由 claw / cc skill 生成後寫回 Turso |

---

## Section 1：系統架構 ✅

- React SPA (CF Pages) ↔ Hono Workers API ↔ Turso
- 4 頁面：Persona Editor、Writing Preview、Editorial Plan、Publish History
- 2 角色：admin（全部）、author（own only）

## Section 2：資料模型 ✅

**既有表**：`persona`、`persona_history`、`editorial_plan`

**新增表**：
```sql
CREATE TABLE user (
  id INTEGER PRIMARY KEY,
  email TEXT UNIQUE NOT NULL,
  oauth_provider TEXT,
  oauth_sub TEXT,
  role TEXT,          -- 'admin' | 'author'
  created_at TEXT
);

CREATE TABLE invite (
  id INTEGER PRIMARY KEY,
  email TEXT UNIQUE NOT NULL,
  role TEXT NOT NULL DEFAULT 'author',
  invited_by INTEGER NOT NULL,
  used_at TEXT,
  created_at TEXT NOT NULL
);

CREATE TABLE audit_log (
  id INTEGER PRIMARY KEY,
  user_id INTEGER NOT NULL,
  action TEXT NOT NULL,
  entity TEXT NOT NULL,
  entity_id TEXT NOT NULL,
  diff_json TEXT,
  created_at TEXT NOT NULL
);
```

**Schema 異動（既有表）**：
```sql
ALTER TABLE persona ADD COLUMN user_id INTEGER NOT NULL;    -- owner
ALTER TABLE persona ADD COLUMN preview_text TEXT;           -- NULL = 未生成
ALTER TABLE persona ADD COLUMN preview_updated_at TEXT;
```

**關鍵修復**：`mindfulness-spirit/scripts/run.py:712` — expression、mental_models、heuristics 未注入 writer_prompt，實作時一併修。

## Section 3：API 端點設計 ✅

**Human 端點（OAuth JWT）**：
- Auth: `/auth/login/google`, `/auth/login/github`, `/auth/callback`, `/auth/logout`
- Personas: `GET/POST /api/personas`, `GET/PUT/DELETE /api/personas/:slug`
- Plans: `GET/POST /api/personas/:slug/plans`, `PUT/DELETE .../plans/:id`
- History: `GET /api/personas/:slug/history`
- Preview: `GET /api/personas/:slug/preview`（純讀 preview_text，不呼叫 LLM）
- Users: `GET /api/users`, `POST /api/users/invite`, `PUT /api/users/:id`（admin only）
- Audit: `GET /api/audit`（admin only）

**Skill-facing 端點（Service Token）**：
- `PUT /api/personas/:slug/preview` — skill 寫回 preview_text
- `POST /api/personas/:slug/history` — skill 寫回發佈記錄

Skill 不直接寫 Turso，走 CF Workers API，audit_log 由 middleware 自動記錄。

```
# claw .env
PERSONA_API_URL=https://api.persona.yourdomain.workers.dev
PERSONA_SERVICE_TOKEN=<secret>
```

**Scope middleware**：author 存取時強制 `WHERE persona.user_id = me`，不符合回 403。Admin 跳過。Service token 只允許上述兩個 write-back 端點。

## Section 4：OAuth 流程 ✅

- HttpOnly Cookie 存 JWT（7 天），無 refresh token
- Invite-only：未登記 email OAuth 進不來
- JWT payload: `{ sub, email, role, iat, exp }`
- Role 變更需重新登入才生效

## Section 5：寫作風格預覽機制 ✅

**CF Workers 不碰 LLM。** 流程：
1. Persona 建立後 `preview_text = NULL`
2. Webapp 查詢時若 NULL → 顯示提示「請在 skill 跑過後再查看」
3. Skill `--mode preview-pending` 掃 NULL persona → 用 claw/cc model 生成 → 寫回 Turso
4. Webapp 下次查詢顯示結果

## Section 6：錯誤處理與測試策略 ✅

**API 錯誤**：400 / 401 / 403 / 404 / 409 / 500，不洩漏 DB 細節。

**Skill 端**：exit 0，失敗印 `[WARN]`，preview 失敗維持 NULL 等下次重試。

**測試優先項目**：
1. Author 存取他人 slug → 403
2. 未邀請 email OAuth → 403
3. audit_log 每次 mutation 都寫入
4. preview_text NULL → webapp 顯示提示
5. preview-pending 不重複覆蓋已有值

---

## 未來擴展（已規劃，未設計）

- **Liko** — 財經週報（開盤/收盤），新增 `finance-weekly` skill
- 每個 user 可擁有多個 persona（已在 schema 支援）
- Author 之間完全隔離，只有 admin 可跨 user 查看

---

## 接手步驟

1. `git pull`
2. 開 `docs/superpowers/specs/persona-webapp-design.html`（含圖表）
3. 實作順序建議：DB migration → Workers scaffold → OAuth → Persona CRUD → Skill preview-pending → 前端
