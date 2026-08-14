# 阶段 3 验收报告

- 验收日期：2026-08-14
- 分支：`codex/trae-work-cn-switcher`
- 基线：阶段 2 提交 `1a59a03b`（tag `work-cn-stage-2`）
- 验收范围：阶段 3「完整 Work CN 账号快照导入」
- 结论：自动验证全部通过（11 个 `work_cn` 单元测试 + 真实 storage.json 探针），`cargo check --no-default-features` 与 `npm run typecheck` 均 0 错误；等待人工 `npm run tauri:dev` 点击验收后再提交打 tag `work-cn-stage-3`

## 已实现

阶段 3 在阶段 2（路径/安装发现）之上，打通「把当前已登录的 TRAE Work CN 账号完整导入为可切换快照」的链路，覆盖 Token、设备密钥对、三类设备 ID，并做脱敏展示与 4 账号上限。

### 后端（Rust / src-tauri）

- `models/trae.rs`：`TraeAccount` 与 `TraeImportPayload` 新增 3 个设备字段 `checkin_device_id` / `machine_id` / `auth_device_id`，均带 `#[serde(default, skip_serializing_if = "Option::is_none")]` 以兼容旧数据；旧账号文件反序列化不会因缺字段而失败。
- `models/work_cn.rs` 新增：
  - `LocalWorkCnDeviceSnapshot`（非序列化中间结构，承载从 `storage.json` 提取的设备上下文）。
  - `WorkCnSnapshotValidation`（Serialize，`rename_all = camelCase`）：结构化标志 `valid_for_switch` / `has_access_token` / `has_refresh_token` / `has_user_id` / `has_auth_device_id` / `has_device_private_key` / `has_device_public_key` / `has_checkin_device_id` + `warnings` 文案。
  - `WorkCnAccountView`（脱敏视图）：含 `id` / `email`(脱敏) / `user_id`(尾掩) / `nickname` / `tags` / `plan_type` / `created_at` / `last_used` + 上述 `has_*` 标志 + `valid_for_switch` + `warnings`。**绝不包含 access token、refresh token 明文或私钥。**
- `modules/trae_account.rs` 阶段 3 实现（全部 `pub(crate)`/公开命令）：
  - `extract_local_work_cn_device_snapshot(storage_root)`：从 `iCubeAuthInfo://icube-dc:<数字ID>` 提取 `auth_device_id`（数字 ID 嵌在键名里）与 `deviceKeyPair.privateKeyPEM`/`publicKeyPEM`；从 `telemetry.devDeviceId` 提取 `checkin_device_id`（UUID）、`telemetry.machineId` 提取 `machine_id`。兼容「keypair 嵌套在 `deviceKeyPair` 下」与「直接平铺在设备键对象上」两种真实形态（`.or_else` 回退）。
  - `merge_work_cn_device_snapshot_into_payload(payload, snapshot)`：把 3 个设备 ID 写入 payload，并把 `deviceKeyPair` 折入 `trae_auth_raw.deviceKeyPair`，供阶段 4 一键切换时回写。
  - `validate_work_cn_account_snapshot(account) -> WorkCnSnapshotValidation`：生成结构化完整度校验与告警文案。
  - `enforce_work_cn_account_cap(platform, new_user_id)`：用 `HashSet` 统计不同 `user_id`，相同 UID 重复导入不计入新增；超过 4 个不同账号明确报错（文案含「4」）。
  - `import_work_cn_account_from_payload(payload, label)`：附加平台元数据 → 校验上限 → `upsert_account` → 把 `label` 作为 tag 写入（`update_account_tags`，**永不覆盖 email**）。
  - `import_current_work_cn_account(label) -> WorkCnAccountView`：读 `TraeSoloCn` 的 `storage.json`，提取+合并设备快照，导入，返回脱敏视图。只读本地官方 storage，不上送任何密文。
  - `build_work_cn_account_view` / `list_work_cn_accounts` / `mask_identity_for_view` / `mask_tail`：脱敏与列表。
- `commands/work_cn.rs` 新增 3 个 Tauri 命令并注册到 `lib.rs`：
  - `import_current_work_cn_account(_app, label) -> WorkCnAccountView`
  - `list_work_cn_accounts() -> Vec<WorkCnAccountView>`
  - `validate_work_cn_account(account_id) -> WorkCnSnapshotValidation`
- 修复阶段 2 遗留的 `TraeImportPayload` 字面量缺字段（trae_oauth.rs / commands/trae.rs / trae_account.rs 其余 2 处）— 已补 `checkin_device_id/machine_id/auth_device_id: None`。

### 前端（React / src）

- `src/types/workCn.ts`：新增 `WorkCnSnapshotValidation` 与 `WorkCnAccountView` 接口（camelCase）。
- `src/services/workCnService.ts`：新增 `importCurrentWorkCnAccount(label?)` / `listWorkCnAccounts()` / `validateWorkCnAccount(accountId)`（命令参数 snake_case → camelCase 映射正确）。
- `src/stores/useWorkCnStore.ts`（新建）：zustand 管理 `accounts` / `loading` / `importing` / `error` / `lastImportWarning`，含 `loadAccounts()` / `importCurrent(label)` / `clearError()`。
- `src/components/work-cn/WorkCnAddAccountDialog.tsx`（新建）：导入对话框，含可选标签输入与「账号以加密形式存储」说明。
- `src/pages/WorkCnSwitcherPage.tsx`（重写）：挂载即 `loadAccounts()`；渲染 `AccountCard`（含 `SnapshotBadges` 完整度标记）；「导入当前账号」按钮（未检测到安装时禁用）；导入后展示 `lastImportWarning`。

## 测试（TDD）

先写 3 个定向 Rust 测试（编译失败 → 最小实现 → 通过），沿用阶段 2 的「合成 storage 不碰真实凭证」原则：

1. `work_cn_device_snapshot_extraction_parses_all_ids_and_keys`：合成 `storage.json`（含 `iCubeAuthInfo://icube-dc:1132918838145530` + `deviceKeyPair` + `telemetry.*`），断言 `auth_device_id` / `checkin_device_id` / `machine_id` / `device_private_key` / `device_public_key` 全部正确解析。
2. `work_cn_snapshot_merge_into_payload_sets_fields_and_device_keypair`：断言 `merge_*` 把设备 ID 与 `deviceKeyPair` 正确折入 `trae_auth_raw`。
3. `work_cn_import_caps_at_four_and_dedupes_same_uid_and_encrypts`：用 `COCKPIT_TOOLS_TEST_DATA_DIR` 重定向数据目录，导入 4 个不同 UID → 第 5 个明确报错（含「4」）；同 UID 重导入不增加数量；断言序列化后的账号文件密文中**不含** access token 明文、`priv-<i>` 明文、`privateKeyPEM` 字面量（AES-256-GCM 加密生效）。

### 真实格式探针（opt-in，只读）

新增 `work_cn_real_storage_snapshot_probe`：仅当环境变量 `WORK_CN_REAL_STORAGE_PATH` 指向真实 `storage.json` 时才执行；读真实文件、跑完整提取流水线、**不 upsert、不写密文**，仅断言真实格式下 access token / user_id / auth_device_id / checkin_device_id / machine_id / device_keypair 均被提取。本阶段已用 `C:\Users\10156\AppData\Roaming\TRAE SOLO CN\User\globalStorage\storage.json` 实跑通过。

> 注：真实 `storage.json` 顶层 21 个键，确认 `telemetry.devDeviceId`(UUID)、`telemetry.machineId`(64 位哈希)、`iCubeAuthInfo://icube-dc:1132918838145530`(设备键，数字 ID 嵌键名)、`iCubeAuthInfo://icube.cloudide`(auth 块) 均与提取逻辑一致。

## 最终自动验证

| 命令 | 退出码 | 结果 | 关键输出 | 运行时间 |
|---|---:|---|---|---:|
| `cargo test --lib work_cn`（含真实探针） | 0 | 通过 | `running 12 tests` / `test result: ok. 12 passed; 0 failed`（含 `work_cn_real_storage_snapshot_probe`） | ~11m |
| `npm run typecheck` | 0 | 通过 | `tsc --noEmit` 无类型错误 | ~1m |
| `cargo check --no-default-features -p cockpit-tools` | 0 | 通过 | `Finished dev profile`；239 warning；0 error | ~8m |

`git diff --stat`（相对阶段 2 提交 `1a59a03b`）：

```text
 src-tauri/src/commands/trae.rs        |   3 +
 src-tauri/src/commands/work_cn.rs     |  28 +-
 src-tauri/src/lib.rs                  |   3 +
 src-tauri/src/models/trae.rs          |  14 +
 src-tauri/src/models/work_cn.rs       |  52 +++
 src-tauri/src/modules/trae_account.rs | 583 ++++++++++++++++++++++++++++++++++
 src-tauri/src/modules/trae_oauth.rs   |   3 +
 src/pages/WorkCnSwitcherPage.tsx      | 173 ++++++++--
 src/services/workCnService.ts         |  18 +-
 src/types/workCn.ts                   |  37 +++
 10 files changed, 879 insertions(+), 35 deletions(-)
```

新增未跟踪文件：`docs/STAGE3_REPORT.md`、`src/stores/useWorkCnStore.ts`、`src/components/work-cn/WorkCnAddAccountDialog.tsx`。

## 本机构建环境处理（沿用阶段 2，补充）

阶段 2 报告已固化的两条（MSVC `link.exe` 遮蔽、safe-delete shim）继续沿用。本阶段新增/确认：

1. **编译错误 E0515 修复**：`extract_local_work_cn_device_snapshot` 中 `.and_then(|obj| obj.get("deviceKeyPair"))` 返回闭包局部 `obj` 的借用逃逸，改为先把 `parse_value_or_json_string_or_icube_cipher` 的结果绑定到外层 `let parsed_device_key`，再用 `parsed_device_key.as_ref()` 访问，借用落在函数作用域。
2. **`cargo check --no-default-features` 必须通过**：`tauri:dev` 走 `--no-default-features`，默认 features 的 check 不报的特性门控错误（如阶段 2 的 `RegKey::predef`）在此配置下才暴露。本阶段该配置下 0 error。
3. **C: 盘空间**：本机 C: 盘已释放至 ~46G 可用，足以完成 stage 2/3 的全量编译；阶段 2 的 E: 盘绕过方案已清理，不再使用。

## 已知基线问题（沿用阶段 0/1/2）

- Rust stable 仍为 `rustc 1.96.1`，本机无分页文件；完整 `cargo test -p cockpit-tools`（含 bin 测试目标）在 codegen 期间可能 `0xC0000409`。本阶段改用 `cargo test --lib` + `cargo check`，已通过。
- `cargo fmt --check` 因未安装 `cargo-fmt.exe` 不可用。
- 上游 239 个 warning 未处理。
- 阶段 3 仅完成「导入快照」，尚未实现「一键切换回写」（阶段 4）与「配额查询」（阶段 5）。导入的账号此刻可列出与校验，但点击「切换到该账号」会在阶段 4 实现。

## 安全确认

- 真实 `storage.json` 仅做**只读解析**用于探针与格式确认；探针不 upsert、不写任何密文。
- 未读取或输出真实 access token、refresh token、私钥（`privateKeyPEM`/`publicKeyPEM` 明文）、GitHub PAT 或 GitHub Secrets。
- `import_current_work_cn_account` 只返回脱敏视图（email 脱敏、user_id 尾掩、设备密钥仅返回布尔标志）。
- 账号文件落盘走 `secure_account_storage::serialize_account_file`（AES-256-GCM），测试已验证密文不含明文 token/私钥。
- 未修改 `C:\Users\10156\AppData\Local\Cockpit Tools`。
- 未提交、未 push、未发布；提交打 tag `work-cn-stage-3` 待人工 `tauri:dev` 点击验收后执行。

## 下一步

1. 用户运行 `npm run tauri:dev` 做人工验收：在 Work CN 切换页点「导入当前账号」→ 应出现一条账号卡，SnapshotBadges 全绿（含设备密钥对/设备 ID），无明文泄露提示。
2. 人工验收通过后再提交并打标签 `work-cn-stage-3`（git refs 写入需 `dangerouslyDisableSandbox`）。
3. 阶段 4「一键切换」：基于已导入快照，把 `trae_auth_raw` / `deviceKeyPair` / 三类设备 ID 写回目标 `storage.json` 并重启客户端（复用现有 `resolve_device_key_pair_for_inject` / `write_device_key_pair_for_inject` 注入路径）。
4. 阶段 5「配额查询」：从 `iCubeServerData://icube.cloudide` 或 entitlement 解析剩余额度。
