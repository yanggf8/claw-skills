# Persona 管理平台 — 設計進度

**狀態：** 設計完成（Gemini reviewed），待實作
**日期：** 2026-04-17
**設計文件（含圖表）：** `docs/superpowers/specs/persona-webapp-design.html`

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

## Section 2：資料模型 ✅

**新表**：`user`、`invite`、`audit_log`

**修改（seed 重建）**：`persona` 加 `user_id`、`preview_text`、`preview_updated_at`、`persona_updated_at`；`editorial_plan` 加 `created_at`、`updated_at`

**關鍵設計**：
- `user`：`active` 欄位（停用）、`UNIQUE(oauth_provider, oauth_sub)`、email 存入前 toLowerCase
- `persona`：`persona_updated_at` 用於 preview staleness 判斷
- `audit_log`：`caller` 欄位區分 human vs service（service 為 unverified）
- `PRAGMA foreign_keys = ON` 每次連線執行
- 所有常用查詢欄位建 index

**遷移**：備份現有 persona → drop tables → CREATE TABLE → seed script（全部歸 admin user_id=1）

## Section 3：API 端點設計 ✅

**Human 端點（JWT cookie）**：CRUD personas/plans/history，admin-only users/audit

**Skill write-back 端點（Service Token）**：
- `PUT /api/personas/:slug/preview` — 寫回 preview_text
- `POST /api/personas/:slug/history` — 寫回發佈記錄

**`POST /api/personas/:slug/preview`（human）** — 重置 preview_text=NULL，觸發 skill 下次重新生成

Service Token 比對用 `crypto.subtle.timingSafeEqual()`（非 ===）

## Section 4：OAuth 流程 ✅

- Single Worker 同 origin，SameSite=Lax 正常運作
- Invite-only：未登記 email → 403
- email 統一 toLowerCase 後存入
- Logout：POST /auth/logout → 伺服器 Set-Cookie Max-Age=0
- JWT 7 天，無 refresh，停用後最多 7 天失效

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

## 實作順序建議

1. seed.py（備份 + 重建 schema + 插入初始資料）
2. Workers scaffold（Hono + Assets binding + JWT middleware）
3. OAuth flow（Google + GitHub + invite check）
4. Persona CRUD API
5. Skill write-back（preview-pending mode + Service Token）
6. 前端 React（Persona Editor → Preview → Plans → History）
