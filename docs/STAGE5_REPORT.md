# 阶段 5 报告：积分查询与展示（TDD）

> 分支：`codex/trae-work-cn-switcher` · 上游基线：cockpit-tools v1.3.16
> 上一阶段：`work-cn-stage-4`（一键切换与回滚）
> 本阶段提交：待提交后补 tag `work-cn-stage-5`

## 1. 阶段目标（开发指南 §8.4）

查询并展示每个已导入 Work CN 账号的剩余积分，且**只查询、绝不签到**（不调用任何 check-in / claim 接口）。查询失败时明确提示「access token 可能已失效，请重新登录后重新导入」，绝不清空本地账号、绝不阻断切换。

## 2. TDD 过程

| 步骤 | 内容 |
|---|---|
| 红 | 先写 10 个积分测试：8 个纯函数 `parse_work_cn_credits_from_usage`（单包 / 多包求和 / 无限包 / 隐藏包跳过 / 缺字段判暂无数据 / 无 limit 不计 0 / 剩余不为负 / 嵌套在 Result.payload 下）+ 2 个命令测试（读缓存 / 未知账号报错）。编译报出 12 处错误（缺 `Deserialize`、测试模块重复 import、`&Value` 误当 `&str`、测试 `usage` 误为 `String`），见 §5。 |
| 绿 | 修复后，**27 个 work_cn 测试全绿**（含 10 个新增积分测试，0 failed）。 |

## 3. 已实现内容

### 3.1 后端解析（`src-tauri/src/modules/trae_account.rs`）

- `parse_work_cn_credits_from_usage(usage_raw: &Option<Value>) -> WorkCnCreditsSummary`（纯函数，无副作用、无需账号库）：
  1. `usage_raw` 为 `None` 或非法 JSON → 返回 `no_data`（`total/remaining=None, used=0, unlimited=false`）。
  2. 兼容 `data` / `Result` / `payload` 多层嵌套，取 `user_entitlement_pack_list`。
  3. 跳过隐藏包（`is_hide=true`）、显式停用（`is_active=false`）、`status<=0` 或 `"inactive"/"disabled"/"expired"` 的包。
  4. 累加 `entitlement_base_info.quota.credits_limit`（容忍 `quota.credits_limit` / `credits_limit` 平铺）作为 `total`，累加 `usage.credits_amount`（容忍嵌套）作为 `used`。
  5. `credits_limit == -1` → 标记 `unlimited=true`（UI 显示「无限」）。
  6. 找不到 `credits_limit` 的包**直接忽略，绝不误记 0**（指南 §8.4 规则 7）。
  7. `remaining = (total - used).max(0)`（不允许为负）。
  8. `updated_at` = 当前 Unix 秒。
- `get_work_cn_credits(account_id, force_refresh) -> Result<WorkCnCreditsSummary, WorkCnCommandError>`：
  - 账号不存在 → `AccountNotFound`。
  - `force_refresh=false`（默认）：只读本地已缓存的 `trae_usage_raw`，**完全不联网、绝不签到**（列表载入即走此路径）。
  - `force_refresh=true`：「刷新积分」按钮触发，复用上游 `refresh_account_usage_only_async`（仅用量刷新，非 claim）；token 失效 → `SnapshotIncomplete` + 友好 detail，且不清除本地账号。
- `work_cn_credits_now_secs()` 取 `chrono::Utc::now().timestamp()`。

### 3.2 模型（`src-tauri/src/models/work_cn.rs`）

- `WorkCnCreditsSummary { total: Option<i64>, used: i64, remaining: Option<i64>, unlimited: bool, updated_at: i64 }`，派生 `Serialize + Deserialize`（camelCase）。
- `WorkCnErrorCode` 已含 `AccountNotFound` / `SnapshotIncomplete`（阶段 4 已加），本阶段复用。

### 3.3 命令与注册

- `commands/work_cn.rs`：`get_work_cn_credits` 包装，错误经 `command_error_to_string` 返回。
- `lib.rs`：注册 `commands::work_cn::get_work_cn_credits`。

### 3.4 前端

- `types/workCn.ts`：`WorkCnCreditsSummary` 接口（mirrors 后端）。
- `services/workCnService.ts`：`getWorkCnCredits(accountId, forceRefresh?)`，错误解析为结构化 `{code,message}`。
- `stores/useWorkCnStore.ts`：`creditsById` / `creditsErrorById` / `refreshingCreditsId` 状态 + `refreshCredits(accountId, forceRefresh?)`；`loadAccounts` 载入后**仅本地解析**各账号缓存积分（不联网、不签到）。
- `pages/WorkCnSwitcherPage.tsx`：`renderCredits` 渲染「剩余 X 积分 / 已用 Y 总 Z」（无限包显示「无限」、无数据显「暂无积分数据」、查询失败显红字）；每张账号卡新增「刷新积分」按钮（title「仅查询积分，绝不签到」，刷新中禁用）。

## 4. 验证结果

| 命令 | 结果 |
|---|---|
| `cargo test --lib work_cn` | **27 passed**（10 个新增积分测试全绿，0 failed） |
| `cargo check --no-default-features -p cockpit-tools` | 见 §5（0 error） |
| `npm run typecheck` | 见 §5（0 error） |

新增积分测试覆盖：
- 单包：`total=1000, used=250, remaining=750`；
- 多包求和 `1500 / 350 / 1150`；
- 无限包（`-1`）→ `unlimited=true, total/remaining=None`；
- 隐藏包（`is_hide`）被忽略，只计可见包；
- 缺 `user_entitlement_pack_list` / 空 → 判暂无积分数据；
- 包无 `credits_limit` → 误显 0 防护（total=None）；
- `used>total` → `remaining` 不为负（=0）；
- 列表嵌套在 `Result.payload` 下仍能解析；
- 命令读缓存返回正确余额；未知账号 → `AccountNotFound`。

## 5. 编译修复记录（本阶段踩坑）

1. `WorkCnCreditsSummary` 用了 `Deserialize` 派生但 `models/work_cn.rs` 只 `use serde::Serialize` → 补 `Deserialize`。
2. 测试模块把 `WorkCnCommandError` / `WorkCnErrorCode` 重复 import（与既有 `6507` 行冲突）→ `E0252`，收敛为单行按需 import。
3. `parse_work_cn_credits_from_usage` 把入参 `&Value` 当 `&str` 调 `serde_json::from_str` → `E0308`，改为直接用 `raw`（`usage_response_payload_root(raw)`）。
4. 测试里 `serde_json::json!({...}).to_string()` 得到 `String`，但函数要 `&Option<Value>` → 去掉 `.to_string()`，让 `usage` 保持 `Value`。
5. 前端 `WorkCnSwitcherPage.tsx` 漏 import `WorkCnCreditsSummary`、用了 React 19 下不存在的 `JSX` 命名空间（`→ ReactElement`）、`AccountCard` 调用处漏传 `credits/creditsError/refreshingCredits/onRefreshCredits` 四个 props → 全部补齐。

## 6. 已知限制 / 下一步

- **真实联网刷新需人工验收**：默认路径只读缓存、绝不签到；点「刷新积分」走上游用量刷新接口，需账号 access token 有效。token 失效时给出明确提示（重新登录后重新导入），不破坏本地数据。
- Stage 6（GitHub Secrets 同步）待实现。
- 开发指南 §8.5 的 `github_synced` 字段目前恒为 `false`，不阻止本地切号；将在阶段 6 落地。

## 7. 改动文件

`git diff --stat`（`48223bb6` 之后）：

```
 src-tauri/src/commands/work_cn.rs    |  +20 -
 src-tauri/src/lib.rs                 |   +1
 src-tauri/src/models/work_cn.rs      |  +13
 src-tauri/src/modules/trae_account.rs| +180 -（含 10 个测试）
 src/pages/WorkCnSwitcherPage.tsx     |  +70 -
 src/services/workCnService.ts        |  +22
 src/stores/useWorkCnStore.ts         |  +55
 src/types/workCn.ts                  |  +18
 docs/STAGE5_REPORT.md                |  本报告（新增）
```
