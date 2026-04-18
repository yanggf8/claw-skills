# Persona 管理平台 — 設計紀錄

**狀態：** 已實作 2026-04-18（webapp + DB schema 已部署）
**日期：** 2026-04-17（原設計）→ 2026-04-18（實作對齊）
**實作倉庫：** `~/a/persona-webapp`
**設計文件（含圖表）：** `docs/superpowers/specs/persona-webapp-design.html`（原始版本，部分已過時，以本文為準）
**實作落差與後續計畫：** `docs/specs/2026-04-18-persona-webapp-reconciliation.md`

---

## 已確認需求

| 項目 | 決定 |
|------|------|
| 主要使用者 | 你（admin）+ Ping W.、Liko（author），未來可擴充 |
| 認證方式 | OAuth (Google / GitHub)，Invite-only |
| 部署 | Single Worker + Assets binding（SPA + API 同 origin，消除 CORS / SameSite 問題） |
| DB | Turso (libsql)，複用既有 |
| LLM | CF Workers 不呼叫 LLM；preview 由 claw / cc skill 生成後透過 Service Token 寫回 |
| 遷移策略 | Persona 數量少，drop + recreate schema + seed script |

---

## Section 1：系統架構 ✅

Single Worker 同時 serve SPA（Assets binding）和 API（/api/*），same-origin 消除 CORS 與 cookie 問題。

Skill host → CF Worker API（Service Token）→ Turso（不直接寫 DB）

## Section 2：資料模型 ✅（已更新 2026-04-18）

**新表**：`user`、`invite`、`audit_log`、`content_column`、`installment`、`stream`、`issue`

**修改**：`persona` 加 `user_id`、`preview_text`、`preview_updated_at`、`persona_updated_at`

**編輯計畫重構（取代原 `editorial_plan` / `editorial_topic`）**：
- `content_column`（有限期專欄）：`persona_slug`、`slug`、`title`、`theme`、`kind: finite|ongoing`、`status`、`skill`、`updated_at`
- `installment`（專欄單篇）：`column_id`、`week`、`target_date`、`title_hint`、`angle`、`lens`、`direction`、`key_question`、`status`、`history_id` — 等同原 `editorial_topic` 改名
- `stream`（長期連載）：`persona_slug`、`slug`、`title`、`cadence: daily|weekly|biweekly|monthly`、`status`、`skill`
- `issue`（連載單期）：`stream_id`、`target_date`、`title_hint`、`status`、`history_id`

差異動機：原 `editorial_plan` 無 persona 連結、強制月度、無 finite/ongoing 區分。新模型將「有限期專欄」與「長期連載」分離，適配 Ping W. 的 `心的演算法`（finite）與 Liko 的 `AI 決策週報`（ongoing）。

**關鍵設計**：
- `user`：`active` 欄位（停用）、`UNIQUE(oauth_provider, oauth_sub)`、email 存入前 toLowerCase
- `persona`：`persona_updated_at` 用於 preview staleness 判斷
- `audit_log`：`caller` 欄位區分 human vs service（service 為 unverified）
- 所有常用查詢欄位建 index

**Schema 初始化**：見 `docs/specs/2026-04-18-persona-webapp-reconciliation.md` P1（`lib/schema_init.py`，非 migration）

## Section 3：API 端點設計 ✅（已更新 2026-04-18）

**Human 端點（JWT cookie）**：
- CRUD：`/api/personas`、`/api/columns`（含 `/installments`）、`/api/streams`（含 `/issues`）
- 讀取：`/api/personas/:slug/history`、`/api/dashboard`（新增）、`/api/me`（新增）
- Admin-only：`/api/users`、`/api/audit`

**Skill write-back 端點（Service Token）**：
- `PUT /api/personas/:slug/preview` — 寫回 preview_text
- `POST /api/personas/:slug/history` — 寫回發佈記錄

端點允許清單見 `src/auth.ts` 的 `SERVICE_ALLOWED_PATHS`。Service Token 路徑以外的 bearer 要求直接回 403。

**`POST /api/personas/:slug/preview`（human）** — 重置 preview_text=NULL，觸發 skill 下次重新生成

Service Token 比對用 `crypto.subtle.timingSafeEqual()`（非 ===），含 length mismatch 時的 constant-time dummy compare（需 `nodejs_compat` flag）

**已取代**：原 `/api/personas/:slug/plans` 路由不存在，由 `/api/columns` + `/api/streams` 取代

## Section 4：OAuth 流程 ✅（已更新 2026-04-18）

- Single Worker 同 origin，SameSite=Lax 正常運作
- Invite-only：未登記 email → 403
- email 統一 toLowerCase 後存入
- Logout：POST /auth/logout → 伺服器 Set-Cookie Max-Age=0
- JWT 7 天，無 refresh，停用後最多 7 天失效
- **OAuth state CSRF**（實作時新增）：login 時 set `oauth_state_<provider>` httpOnly cookie（10min TTL、path 限定 `/auth/callback/<provider>`），callback 時驗證並刪除。state 不符 → 400。
- Callback 路由拆為 `/auth/callback/google` 與 `/auth/callback/github`（原設計為單一 callback）

## Section 5：寫作風格預覽機制 ✅

- CF Workers 不碰 LLM
- Staleness check：`persona_updated_at > preview_updated_at` → stale=true
- 三種 preview 狀態：NULL / stale / fresh
- "重新整理" 按鈕：POST reset → skill 掃 NULL → 生成 → PUT write-back

## Section 6：錯誤處理與測試 ✅

- Skill exit 0，失敗 [WARN]
- 9 個測試案例，含 service token scope 限制、staleness、email 大小寫

## Section 7：營運工具（待補充）

- DB 備份 / 復原策略
- Service Token rotation 流程
- 監控 / alerting

---

## 未來擴展

- Liko：財經週開收盤報，新增 `finance-weekly` skill
- 多 persona per user 已在 schema 支援

---

## 實作狀態（2026-04-18 更新）

原訂 6 步已完成：
1. ✅ seed.py（現有資料已在 Turso，無需重建；schema init script 待補，見 reconciliation P1）
2. ✅ Workers scaffold（Hono + Assets binding + JWT middleware）
3. ✅ OAuth flow（Google + GitHub + invite check + state CSRF）
4. ✅ Persona CRUD API
5. ✅ Skill write-back（preview + history，Service Token 路徑允許清單）
6. ✅ 前端 React（14 頁，Persona / Columns / Streams / Dashboard / History / Users / Audit）

後續工作見 `docs/specs/2026-04-18-persona-webapp-reconciliation.md`：
- P1 Schema init script（reproducibility）
- P2 Seed 腳本與資料分離
- P4 Persona-skill CLI 補齊 column/stream 指令
- P5 CLI↔webapp 寫作流程整合（待 Gemini review）
