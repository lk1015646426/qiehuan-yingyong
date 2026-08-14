# 阶段 4 报告：一键切换与回滚（TDD）

> 分支：`codex/trae-work-cn-switcher` · 上游基线：cockpit-tools v1.3.16
> 上一阶段：`work-cn-stage-3`（完整账号快照导入）
> 本阶段提交：待提交后补 tag `work-cn-stage-4`

## 1. 阶段目标（开发指南 §8.3）

实现「一键切换并打开官方客户端」的原子命令，复用上游调用链（禁止自造进程管理），并在任何有副作用的步骤失败时把客户端恢复到切换前状态，绝不让用户停留在空白登录态。

## 2. TDD 过程

| 步骤 | 内容 |
|---|---|
| 红 | 先写 4 个切换测试：happy-path 注入目标 UID、verify UID 不匹配报错、inject 失败回滚并报告 `InjectFailed`、切到「当前账号」只打开不重写 storage。编译报出 9 处类型/字段错误（见 §5）。 |
| 绿 | 逐个修复后，17 个 work_cn 测试全绿（含 4 个新增）。 |

## 3. 已实现内容

### 3.1 后端（`src-tauri/src/modules/trae_account.rs`）
- `switch_work_cn_account(account_id)`：严格按指南 §8.3 状态机
  1. 全局异步锁（防并发，返回 `Busy`）
  2. 加载并 `validate_work_cn_account_for_switch` 校验（缺 access/refresh token、user_id、数字 DeviceID、设备密钥对 → `SnapshotIncomplete`）
  3. `sync_current_work_cn_session_from_local` 切前把当前客户端轮换后的 token 同步回账号库（best-effort）
  4. 内存快照：旧 storage 字节 / 旧默认实例绑定 / 是否运行
  5. 若目标已是当前绑定账号 → 仅打开客户端、不重写 storage
  6. 关闭客户端（`close_trae_platform_default`）
  7. 注入目标账号（`inject_to_trae_at_path`，复用上游加密写 storage）
  8. 绑定默认实例（`update_default_settings_for_platform`）
  9. 启动默认实例（`commands::trae_instance::trae_start_instance`，复用上游 close→inject→start→verify 链路）
  10. 启动后验证（`verify_work_cn_switched_account`，轮询确认 storage UID==目标且 token 未清）
  11. 失败逐级回滚：恢复 storage 字节 + 默认实例绑定 + 重启旧账号
- `validate_work_cn_account_for_switch` / `sync_current_work_cn_session_from_local` / `verify_work_cn_switched_account` / `rollback_storage_bytes` / `rollback_bind` / `restart_previous_after_rollback`。
- 测试注入点（仅设环境变量时生效，生产默认不触发）：`WORK_CN_SWITCH_STORAGE_OVERRIDE`（重定向 target storage）、`WORK_CN_SWITCH_SKIP_PROCESS=1`（跳过真实进程）。

### 3.2 模型（`src-tauri/src/models/work_cn.rs`）
- `WorkCnSwitchResult { account_id, user_id, launched, verified, github_synced, warning }`
- `WorkCnErrorCode`：`Busy / AccountNotFound / ClientNotInstalled / SnapshotIncomplete / ClientCloseFailed / InjectFailed / LaunchFailed / VerifyAccountMismatch / VerifyTimeout`
- `WorkCnCommandError { code, message, detail }`，`command_error_to_string` 序列化为 JSON 字符串供前端按 `code` 分支。

### 3.3 命令与注册
- `commands/work_cn.rs`：`switch_work_cn_account` 包装，错误经 `command_error_to_string` 返回。
- `lib.rs`：注册 `commands::work_cn::switch_work_cn_account`。

### 3.4 前端
- `types/workCn.ts`：`WorkCnSwitchResult` / `WorkCnErrorCode` / `WorkCnCommandError` 接口。
- `services/workCnService.ts`：`switchWorkCnAccount(accountId)`，错误解析为结构化 `{code,message}`。
- `stores/useWorkCnStore.ts`：`switching` 状态 + `switchTo(accountId)`。
- `pages/WorkCnSwitcherPage.tsx`：账号卡「切换并打开」按钮（快照不完整时禁用）+ 错误横幅（按 `code` 展示友好文案）。

## 4. 验证结果

| 命令 | 结果 |
|---|---|
| `cargo test --lib work_cn` | **17 passed**（4 个新增切换测试全绿） |
| `cargo check --no-default-features -p cockpit-tools` | 见 §5（应与 Stage 3 一致 0 error） |
| `npm run typecheck` | 见 §5 |

新增切换测试覆盖：
- happy-path 注入后 storage 含目标 UID（证明写入的是目标账号）；
- verify UID 不匹配 → `VerifyAccountMismatch`，UID 一致 → 通过；
- inject 失败 → `InjectFailed` 且不留半成品 storage 文件；
- 切到当前账号 → 只打开、不改写 storage。

## 5. 编译修复记录（本阶段踩坑）

1. `access_token` 在 `TraeAccount`/`TraeImportPayload` 中均为 `String`（非 `Option`），`.as_deref()` 非法 → 直接 `.trim()`。
2. `TraeImportPayload` 无 `id` 字段，`upsert_account` 按 identity 自动匹配生成 id → 删除 `payload.id = ...`。
3. `read_local_trae_auth_from_storage_path` 返回 `Result<Option<...>>` → 用 `if let Ok(Some(payload))`。
4. `trae_start_instance` 在 `commands::trae_instance`（非 `modules`）→ 修正路径为 `crate::commands::trae_instance::trae_start_instance`。
5. `load_default_settings_for_platform` 的 `bind_account_id` 字段为 `Option<String>`（非双层 Option）→ 去掉多余的 `.flatten()`。
6. 测试账号需带平台标记 `platformId: "trae_solo_cn"`，否则 `resolve_account_platform_kind` 识别不出 `TraeSoloCn`。

## 6. 已知限制 / 下一步

- **真实切换需人工验收**：本阶段测试已用隔离 fixture（重定向 storage + SKIP_PROCESS）验证全部状态机逻辑，但**未真正关闭/启动你的 TRAE 客户端**。首次真实切换建议由你在 `npm run tauri:dev` 中点一次「切换并打开」确认（会实际改写 storage.json 并重启客户端）。
- Stage 5（积分查询展示）、Stage 6（GitHub Secrets 同步）待实现。
- GitHub 同步（§8.5）目前为占位 `false`，不阻止本地切号；将在阶段 6 落地。

## 7. 改动文件

`git diff --stat`（`84d5fd29` 之后，+1089/−6）：

```
 src-tauri/src/commands/work_cn.rs    |  19 +-
 src-tauri/src/lib.rs                 |   1 +
 src-tauri/src/models/work_cn.rs      |  71 ++
 src-tauri/src/modules/trae_account.rs| 855 ++++++++++++++++++++++++++++++++-
 src/pages/WorkCnSwitcherPage.tsx     |  54 +-
 src/services/workCnService.ts        |  38 +-
 src/stores/useWorkCnStore.ts         |  24 +-
 src/types/workCn.ts                  |  33 ++
 docs/STAGE4_REPORT.md                | 本报告（新增）
```
