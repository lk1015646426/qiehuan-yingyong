# 阶段 7 报告：四账号完整体验 + 后台会话同步

> 项目：TRAE Work CN 四账号切换器（cockpit-tools v1.3.16，分支 `codex/trae-work-cn-switcher`）
> 基线：阶段 0–6 已提交（`741ce86f`，tag `work-cn-stage-6`）
> 设计：`docs/STAGE7_DESIGN.md`（架构师高见远产出，主理人拍板 4 项决策）
> 实现：工程师寇豆码，严格 TDD（先测试失败 → 实现 → 通过）

## 一、阶段目标

完成四个账号的日常使用闭环：后台持续监测官方客户端的 `storage.json`，在客户端轮换 token 后自动把最新凭证回写账号库，并在需要时同步到 GitHub Secrets。**绝不触发签到/领取/计费**。

## 二、实现内容

### 2.1 后端新增 `modules/work_cn_session_watcher.rs`

- `ensure_started(app_handle)`：`AtomicBool` 防重入 + 首轮 10s 延迟 + 60s 循环 + `spawn_blocking` 包裹阻塞 I/O。
- `watch_once(runner)`：核心状态机（同步、可单测），顺序：
  1. 解析 storage 路径（override 优先）
  2. mtime 门禁（未变化直接 `Unchanged`，不解密）
  3. 会话失败退避门禁（15min）
  4. 切号锁 `try_lock`（占用 → `SwitchBusy`）
  5. 读 + 解密 → UID/email 匹配 → `NoMatch` / 加载账号
  6. `sync_account_tokens_from_storage_path` 回写 + `preserve_account_metadata` 保留设备快照 + `save_account_file`
  7. 释放切号锁后，token 变化才走 GitHub 同步（10min 退避门禁）
- 事件仅在 `{TokenUpdated, NoMatch, Failed, NoStorage}` 时发射，避免 60s 无谓重渲染。
- `get_work_cn_session_watch_status()`：供命令层读取 `LAST_STATUS`。

### 2.2 后端原语提升/新增（`trae_account.rs`）

| 原语 | 说明 |
| --- | --- |
| `try_lock_work_cn_switch()` | 暴露全局切号锁（`OwnedMutexGuard`），供 watcher 非阻塞探测 |
| `resolve_current_work_cn_storage_path()` | 当前 storage 路径（含测试 override） |
| `read_local_trae_auth_from_storage_path` | `fn` → `pub(crate)`（读+解密） |
| `find_work_cn_account_id_for_payload()` | UID→email 回退匹配账号库 |
| `sync_account_tokens_from_storage_path` | `fn` → `pub(crate)`（应用+检测 token 变化） |
| `save_account_file` | `fn` → `pub(crate)` |
| `sync_work_cn_github_for_switch` | 占位 `false` → 接上真实 `sync_account_secrets_if_bound` |

### 2.3 GitHub 同步逻辑上提（`work_cn_github.rs`）

- 新增 `sync_account_secrets_if_bound_with(runner, account)`（可测版）与 `sync_account_secrets_if_bound(account)`（生产入口）。
- 命令 `sync_work_cn_github_account` 重构为复用上提函数，切换链路、watcher、命令三处共用。

### 2.4 前端

- 新增 `components/work-cn/WorkCnStatusBanner.tsx`：按 `WorkCnSessionWatchOutcome` + GitHub 结果映射灰/绿/黄/红四态。
- `useWorkCnStore.ts`：新增 `sessionWatchStatus` 状态 + `loadSessionWatchStatus`/`applySessionWatchStatus` action；token 更新事件触发 `loadAccounts` + `loadGitHubConfig` 刷新。
- `WorkCnSwitcherPage.tsx`：挂载时加载状态、注册 `listen` 订阅 `work-cn:session-watch` 事件、渲染横幅。
- `types/workCn.ts` / `services/workCnService.ts`：新增状态类型、`getWorkCnSessionWatchStatus` invoke、事件名常量。

## 三、关键工程决策

1. **`preserve_account_metadata`（设计未覆盖的关键坑）**：`sync_account_tokens_from_storage_path` 底层 `apply_payload` 会用 storage 里的认证 payload 覆盖账号全部字段，而 storage 不含设备密钥对/platformId 等切换必需元数据。监测器必须在回写前恢复这些字段，否则账号将丢失设备密钥而无法再次切换。恢复范围：`checkin_device_id`/`machine_id`/`auth_device_id`、`trae_auth_raw` 的 `platformId`/`deviceInfo`/`deviceKeyPair`、`trae_server_raw`/`entitlement`/`usage`/`profile`/`usertag`、`plan_type` 等；`access_token`/`refresh_token`/`status`/`status_reason` 保持同步后新值。
2. **GitHub 同步在释放切号锁后执行**：避免 `gh auth status` 网络慢阻塞用户发起的切号。
3. **失败退避**：会话失败 15min、GitHub 失败 10min，退避期内静默不刷屏。

## 四、测试（TDD）

`cargo test --offline -p cockpit-tools --lib work_cn`：**53 passed / 0 failed**（含既有阶段 2–6 全量回归）。

本阶段新增测试 17 项：

- `trae_account.rs`：`work_cn_try_lock_switch_free_then_busy`、`work_cn_resolve_current_storage_path_honors_override`、`work_cn_find_account_id_prefers_uid_then_email`
- `work_cn_github.rs`：`work_cn_github_sync_if_bound_{disabled,unbound,bound,not_authed}`（4 项）
- `work_cn_session_watcher.rs`（9 项）：`no_storage`、`unchanged_mtime_skips_decrypt`、`switch_busy_when_lock_held`、`no_match_when_uid_unknown`、`token_updated_syncs_github`、`token_unchanged_no_github`、`never_claims_checkin`、`failure_marks_backoff`、`github_failure_marks_backoff`
- `models/work_cn.rs`：`work_cn_watch_outcome_serializes_screaming_snake_case`

## 五、验证门禁

| 门禁 | 命令 | 结果 |
| --- | --- | --- |
| Rust 测试 | `cargo test --offline -p cockpit-tools --lib work_cn` | ✅ 53 passed / 0 failed |
| Rust 编译 | `cargo build --offline --no-default-features -p cockpit-tools` | ✅ 0 error（11m27s） |
| TS 类型检查 | `npm run typecheck` | ✅ 0 error |
| 前端构建 | `env -u NODE_OPTIONS -u CODEBUDDY_SAFE_DELETE_SANDBOX npm run build` | ✅ 0 error |

## 六、文件清单

新增：
- `src-tauri/src/modules/work_cn_session_watcher.rs`
- `src/components/work-cn/WorkCnStatusBanner.tsx`
- `docs/STAGE7_DESIGN.md`、`docs/STAGE7_REPORT.md`

修改（11 个）：
- `src-tauri/src/modules/mod.rs`、`lib.rs`、`modules/trae_account.rs`、`modules/work_cn_github.rs`、`commands/work_cn.rs`、`commands/work_cn_github.rs`、`models/work_cn.rs`
- `src/types/workCn.ts`、`services/workCnService.ts`、`stores/useWorkCnStore.ts`、`pages/WorkCnSwitcherPage.tsx`

## 七、后续

- 阶段 7 的四账号验收矩阵（A↔D 切换、GitHub 未登录/网络断开/Token 轮换/吊销、管理器重启）需在 `npm run tauri:dev` 下人工 E2E 验证，并至少连续 3 天使用（总纲要求）。
- 下一步：阶段 8（清理/发布/安装包）。
