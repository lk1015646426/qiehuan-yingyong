# 阶段 6 验收报告 — GitHub Secrets 同步

> 项目：TRAE Work CN 四账号切换器（cockpit-tools v1.3.16 分支 `codex/trae-work-cn-switcher`）
> 日期：2026-08-14
> 开发模式：TDD（先写测试 → 确认失败 → 实现 → 通过），不提前做后续阶段

## 阶段目标（开发总纲 §8.5）

为已导入的 TRAE Work CN 账号提供 **GitHub Secrets 同步**能力：

- 一个 GitHub 仓库（`owner/repo`）下，把每个账号的 `access_token` 与 `checkin_device_id` 写入 **4 个仓库 Secrets**（`TRAE1_TOKEN` / `TRAE1_DEVICE_ID` … `TRAE4_TOKEN` / `TRAE4_DEVICE_ID`），供 CI 使用。
- **绝不做任何会触发 GitHub 签到/计费/领取的操作**——只写入 Secrets。
- 通过 `gh` CLI 写入（secret 走 **stdin**，不进命令行参数，避免泄密）。
- 配置可持久化（启用开关、仓库、每个账号所占 slot 1–4）。
- 提供 CLI 可用性/登录态探测。
- 通过 **runner 抽象 + fake runner** 做 TDD，真实 `gh` 不在测试环境执行。

## 新增/修改文件

### 后端 Rust（`src-tauri/src`）

- **`modules/work_cn_github.rs`（新增）** — 核心模块：
  - `GitHubRunner` trait：`run(&self, args: &[&str], stdin: Option<&str>) -> Result<GitHubRunOutput, String>`。
  - `RealGitHubRunner`：spawn `gh`，stdin pipe 写 secret，stderr 经 `redact_for_log` 脱敏后返回。
  - `FakeGitHubRunner`：记录每次 `calls`（args + stdin），按 `auth`/`secret`/`--version` 分支返回，供测试断言。
  - `parse_jwt_exp(token) -> Option<i64>`：base64url(`URL_SAFE_NO_PAD`) 解 payload 读 `exp`。
  - `redact_for_log(s)`：将 `[A-Za-z0-9_-]{20,}` 长串替换为 `[REDACTED]`，防止日志泄密。
  - `validate_repository(repo) -> Result<(), String>`：`owner/repo` 形式、字符白名单校验。
  - `validate_github_config(config)`：slot 范围 1–4、无重复 slot / 重复 account、secret 名 `[A-Z0-9_]+`、启用时仓库非空。
  - `sync_account_secrets(runner, account, slot, repository)`：
    - token 过期（JWT `exp` 已过）→ `skipped=true`，**非错误**；
    - `checkin_device_id` 为空 → `skipped=true`；
    - 调 `gh auth status` 探测登录态，未登录 → **硬错误** `Err`；
    - 先写 `TRAE{N}_TOKEN`（stdin），再写 `TRAE{N}_DEVICE_ID`（stdin）；
    - 任一 secret 写入失败 → 立即返回 `Err`，**不算整体成功**。
  - `github_config_path()` / `load_github_config()` / `save_github_config()`：配置文件 `github.json` 落于 `get_data_dir()` 旁。
  - `github_cli_status()`：探测 `gh --version` 与 `gh auth status`。
  - 测试模块（9 个）：见下。
- **`commands/work_cn_github.rs`（新增）** — 4 个 Tauri 命令：
  - `get_work_cn_github_config() -> Result<WorkCnGitHubConfig, String>`
  - `save_work_cn_github_config(config) -> Result<(), String>`（先 `validate_github_config`）
  - `github_cli_status() -> Result<WorkCnGitHubCliStatus, String>`
  - `sync_work_cn_github_account(account_id) -> Result<WorkCnGitHubSyncResult, String>`：用 `RealGitHubRunner`，`load_account`，`find_slot_for_account`，未启用/未绑定返回 `skipped` 的 `Ok`，未找到/gh 失败返回 `Err`。
- **`models/work_cn.rs`** — 新增 4 个结构体（camelCase）：
  - `WorkCnGitHubSlot { slot, accountId, tokenSecret, deviceSecret }`
  - `WorkCnGitHubConfig { enabled, repository, slots }`（含 `Default` 实现）
  - `WorkCnGitHubSyncResult { accountId, synced, skipped, skipReason, error, syncedAt }`
  - `WorkCnGitHubCliStatus { available, authed, detail }`
- **`modules/mod.rs`** — 增加 `pub mod work_cn_github;`
- **`commands/mod.rs`** — 增加 `pub mod work_cn_github;`
- **`lib.rs`** — 注册上述 4 个命令。

### 前端（`src`）

- **`types/workCn.ts`** — 新增 `WorkCnGitHubSlot` / `WorkCnGitHubConfig` / `WorkCnGitHubSyncResult` / `WorkCnGitHubCliStatus` 接口。
- **`services/workCnService.ts`** — 新增 `getWorkCnGitHubConfig` / `saveWorkCnGitHubConfig` / `getWorkCnGitHubCliStatus` / `syncWorkCnGitHubAccount`（错误抛原始 message）。
- **`stores/useWorkCnStore.ts`** — 新增 `githubConfig` / `githubCliStatus` / `githubSyncingById` / `githubSyncResultById` 及动作 `loadGitHubConfig` / `saveGitHubConfig` / `refreshGitHubCliStatus` / `syncGitHub`。
- **`components/work-cn/WorkCnSettingsDialog.tsx`（新增）** — 设置弹窗：启用开关、仓库输入、gh CLI 状态文本、每个账号 slot 下拉（未绑定 / 1–4），保存时做重复 slot / 仓库格式校验。
- **`pages/WorkCnSwitcherPage.tsx`** — `AccountCard` 增加 GitHub 状态行 + “同步 GitHub” 按钮，`WorkCnSettingsDialog` 接入，“设置”按钮打开弹窗，effect 内 `loadGitHubConfig()`。

## TDD 过程

1. 先写 `FakeGitHubRunner` 与全部 9 个测试 → 确认编译失败（目标模块未实现）。
2. 实现 runner 抽象、JWT 解析、配置校验、`sync_account_secrets`、配置读写、CLI 状态。
3. 编译期出现 **5 处 `E0308`**（`validate_repository` / `validate_github_config` 中 `Err("...")` 返回 `Result<_, String>` 但给了 `&str`）→ 全部加 `.to_string()` 修正。
4. 复跑 `cargo test --lib work_cn_github` → **9 passed, 0 failed**。

## 测试结果

- `cargo test --lib work_cn_github`：**9 passed, 0 failed**（9 个测试函数全绿）。
- 测试覆盖：
  - `parse_jwt_exp_roundtrip`：base64url payload 解析往返。
  - `validate_repository_rules`：owner/repo 形式与字符校验。
  - `validate_config_rejects_dup_slot_and_bad_secret`：重复 slot / 非法 secret 名拒绝。
  - `sync_calls_two_secrets_with_stdin_not_args`：两个 secret 均经 stdin 写入，不出现在 args。
  - `first_secret_failure_is_not_overall_success`：首个 secret 失败 → 整体 `Err`。
  - `expired_token_is_skipped_not_error`：过期 token → `skipped`，非错误。
  - `missing_device_id_is_skipped`：缺 `checkin_device_id` → `skipped`。
  - `not_authed_is_hard_error`：未登录 → 硬错误 `Err`。
  - `redact_masks_long_secrets`：长串脱敏。

## 验收（本次会话内完成）

| 检查项 | 命令 | 结果 |
| --- | --- | --- |
| 阶段 6 单元测试 | `cargo test --lib work_cn_github` | ✅ 9 passed, 0 failed |
| 特性门编译（tauri:dev 真实路径） | `cargo build --no-default-features -p cockpit-tools` | ✅ 0 错误 |
| 前端类型检查 | `npm run typecheck` | ✅ exit 0 |

> 说明：`cargo build --no-default-features` 复刻了 `tauri:dev` 的真实编译配置（无默认 feature），是本阶段端到端可用性的关键验收门槛。

## 已知限制 / 后续

- **真实同步验收需 `npm run tauri:dev` 手动跑**（自动化仅覆盖 runner 抽象、JWT 解析、配置校验、fake runner 下的 secret 写入逻辑与跳过/错误分支）。真实 `gh` 的退出码、网络、GitHub 403/限流等仅在手动验收中体现。
- Secret 名固定为 `TRAE{N}_TOKEN` / `TRAE{N}_DEVICE_ID`，与上游 CI 约定保持一致；如 CI 侧改名需同步此处。
- 4 个 slot 上限与账号 4 上限一致；slot 与 account 一一对应，重复绑定会被 `validate_github_config` 拒绝。
- 阶段 5（积分查询）与阶段 6（GitHub 同步）均无“签到/领取/计费”语义，符合“只查不领”硬约束。

## 提交

- commit：`work-cn-stage-6`（见 `git log`）
- tag：`work-cn-stage-6`
- 全部 6 个阶段（0–6）已完成：路径发现 → 快照导入 → 一键切换/回滚 → 积分查询 → GitHub 同步。
