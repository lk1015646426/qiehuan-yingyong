# WorkBuddy 多账号切换与自动签到 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 Windows 桌面工具中安全管理任意数量 WorkBuddy 账号，并将启用账号以一个聚合 GitHub Secret 交给既有云端签到工作流。

**Architecture:** 云端 Python 配置层优先将 `WORKBUDDY_ACCOUNTS_JSON` 解析为带稳定键的动态账号，保留旧静态环境变量回退。桌面端在 `src-tauri` 建立独立的 WorkBuddy 模型、AES-256-GCM 快照库、可回滚切换事务、GitHub 聚合同步与监测器；React 通过独立 Store 和页面调用结构化命令，绝不接收认证原文。

**Tech Stack:** Python 3.11、unittest、GitHub Actions、Rust 2021、Tauri 2、serde、AES-256-GCM、sysinfo、React 19、TypeScript、Zustand、Vitest/node test。

## Global Constraints

- WorkBuddy 只支持 Windows、单实例切换；工具不执行或代替官方登录。
- 本地索引不得持久化 access token、refresh token、session state、完整手机号或认证 JSON。
- 快照详情必须复用 `secure_account_storage` 的 AES-256-GCM 信封；所有 UI/命令返回值必须脱敏。
- 切换只能完整原子替换 `workbuddy-desktop.info`，不得合并或构造 `accounts`、`allAccounts`。
- `WORKBUDDY_ACCOUNTS_JSON` 只包含 `version`、稳定 `key`、清理后的显示名和 access token；Secret 内容只能经 `gh` 的 stdin 传递。
- 聚合 Secret 缺失时兼容 `WB1_TOKEN`、`WB2_TOKEN`；聚合 Secret 合法空数组不回退旧变量。
- 本地切换成功与 GitHub 同步失败必须作为独立结果报告。

---

### Task 1: 云端聚合账号契约

**Files:**
- Modify: `C:/Users/10156/Desktop/脚本/云签到/common/config.py`
- Modify: `C:/Users/10156/Desktop/脚本/云签到/main.py`
- Modify: `C:/Users/10156/Desktop/脚本/云签到/.github/workflows/daily-checkin.yml`
- Create: `C:/Users/10156/Desktop/脚本/云签到/tests/test_workbuddy_accounts.py`

**Interfaces:**
- Produces: `Account(name: str, stable_key: str | None, token: str | None)`.
- Produces: `Config.sites()` 动态生成 `workbuddy` 账号；`parse_account_filter()` 保持 `site:selector` 格式。

- [ ] **Step 1: 写入聚合 Secret 的失败测试**

```python
def test_valid_aggregate_secret_supersedes_legacy_tokens(monkeypatch, logger):
    monkeypatch.setenv("WORKBUDDY_ACCOUNTS_JSON", '{"version":1,"accounts":[{"key":"wb-a1","name":"个人号","access_token":"token-a"}]}')
    monkeypatch.setenv("WB1_TOKEN", "legacy-token")
    accounts = _workbuddy_accounts(Config.load("config.yaml", logger))
    assert [(a.name, a.stable_key, a.token) for a in accounts] == [("个人号", "wb-a1", "token-a")]
```

- [ ] **Step 2: 运行失败测试并确认缺少动态账号解析**

Run: `python -m unittest tests.test_workbuddy_accounts.WorkBuddyAccountConfigTests.test_valid_aggregate_secret_supersedes_legacy_tokens -v`

Expected: FAIL，因为 `Account` 尚无 `stable_key` 且配置仍只读取静态 token 环境变量。

- [ ] **Step 3: 实现最小解析与筛选逻辑**

```python
def _parse_workbuddy_aggregate(raw: str) -> list[Account]:
    payload = json.loads(raw)
    if payload.get("version") != 1 or not isinstance(payload.get("accounts"), list):
        raise ValueError("WORKBUDDY_ACCOUNTS_JSON 格式无效")
    return [Account(name=_safe_name(item["name"]), stable_key=item["key"], token=item["access_token"])
            for item in payload["accounts"]]
```

仅在 `workbuddy` 站点读取该变量；合法空数组返回空集合、缺失才回退 YAML 的 `WB1_TOKEN`/`WB2_TOKEN`，格式错误记录不含原文的 warning 后回退。将 workflow 的 `WORKBUDDY_ACCOUNTS_JSON` 映射到环境，筛选时 WorkBuddy 用 `stable_key or name`。

- [ ] **Step 4: 增加错误、空集合、去重与筛选覆盖**

```python
def test_empty_valid_aggregate_disables_legacy_fallback(monkeypatch, logger):
    monkeypatch.setenv("WORKBUDDY_ACCOUNTS_JSON", '{"version":1,"accounts":[]}')
    monkeypatch.setenv("WB1_TOKEN", "legacy-token")
    assert _workbuddy_accounts(Config.load("config.yaml", logger)) == []
```

- [ ] **Step 5: 运行云端测试**

Run: `python -m unittest discover -s tests -v`

Expected: PASS，既有 TRAE/WorkBuddy 行为与新增聚合账号测试全部通过。

- [ ] **Step 6: 提交云端契约**

```bash
git -C "C:/Users/10156/Desktop/脚本/云签到" add common/config.py main.py .github/workflows/daily-checkin.yml tests/test_workbuddy_accounts.py
git -C "C:/Users/10156/Desktop/脚本/云签到" commit -m "feat: load WorkBuddy accounts from aggregate secret"
```

### Task 2: WorkBuddy 加密快照库与脱敏模型

**Files:**
- Create: `C:/Users/10156/Desktop/脚本/切换应用/source/src-tauri/src/models/workbuddy.rs`
- Create: `C:/Users/10156/Desktop/脚本/切换应用/source/src-tauri/src/modules/workbuddy_account.rs`
- Modify: `C:/Users/10156/Desktop/脚本/切换应用/source/src-tauri/src/models/mod.rs`
- Modify: `C:/Users/10156/Desktop/脚本/切换应用/source/src-tauri/src/modules/mod.rs`

**Interfaces:**
- Produces: `WorkBuddyAccountView`, `WorkBuddyInstallation`, `WorkBuddySnapshotValidation`, `WorkBuddyErrorCode`。
- Produces: `import_current_workbuddy_account()`, `list_workbuddy_accounts()`, `update_workbuddy_account()`, `remove_workbuddy_account()`。

- [ ] **Step 1: 为导入与加密快照写失败测试**

```rust
#[test]
fn import_dedupes_uid_preserves_preferences_and_never_serializes_token_in_index() {
    let first = import_snapshot(sample_auth("uid-a", "token-old"), Some("个人号")).unwrap();
    set_checkin_enabled(&first.id, false).unwrap();
    let updated = import_snapshot(sample_auth("uid-a", "token-new"), None).unwrap();
    assert_eq!(updated.id, first.id);
    assert!(!updated.checkin_enabled);
    assert!(!std::fs::read_to_string(index_path()).unwrap().contains("token-new"));
}
```

- [ ] **Step 2: 运行失败测试**

Run: `cargo test workbuddy_import_dedupes_uid --manifest-path src-tauri/Cargo.toml`

Expected: FAIL，因为新模型和账号模块尚不存在。

- [ ] **Step 3: 实现最小安全账号库**

定义认证文件 DTO，只接受非空 `account.uid` 与 `auth.accessToken`；由 `sha256(uid)` 派生 `wb-` 前缀稳定 ID。索引保存文档定义的非敏感字段；详情文件使用 `secure_account_storage::serialize_account_file("workbuddy", &snapshot)` 原子写入。所有读写持有 WorkBuddy 独立 mutex，UI 视图只暴露掩码手机号、UID 显示值与状态。

- [ ] **Step 4: 增加解析异常和视图脱敏测试**

```rust
#[test]
fn account_view_does_not_serialize_authentication_fields() {
    let json = serde_json::to_string(&build_account_view(sample_record())).unwrap();
    assert!(!json.contains("accessToken"));
    assert!(!json.contains("refreshToken"));
    assert!(!json.contains("13800138000"));
}
```

- [ ] **Step 5: 运行模块测试**

Run: `cargo test workbuddy_account --manifest-path src-tauri/Cargo.toml`

Expected: PASS。

### Task 3: WorkBuddy 检测和可回滚切换事务

**Files:**
- Create: `C:/Users/10156/Desktop/脚本/切换应用/source/src-tauri/src/modules/workbuddy_switch.rs`
- Modify: `C:/Users/10156/Desktop/脚本/切换应用/source/src-tauri/src/modules/workbuddy_account.rs`
- Modify: `C:/Users/10156/Desktop/脚本/切换应用/source/src-tauri/Cargo.toml`

**Interfaces:**
- Produces: `detect_workbuddy_installation()`, `begin_switch(account_id)`, `continue_switch(transaction_id, force_close)`。
- Produces: structured `FORCE_CLOSE_REQUIRED`, `BUSY`, `VERIFY_ACCOUNT_MISMATCH`, `ROLLBACK_FAILED` errors。

- [ ] **Step 1: 写入强制关闭确认和回滚失败测试**

```rust
#[test]
fn close_timeout_requires_confirmation_without_modifying_auth_file() {
    let before = read_auth_file(&fixture.auth_path).unwrap();
    let outcome = switch_with(&fixture, "wb-target", false).unwrap_err();
    assert_eq!(outcome.code, WorkBuddyErrorCode::ForceCloseRequired);
    assert_eq!(read_auth_file(&fixture.auth_path).unwrap(), before);
}
```

- [ ] **Step 2: 运行失败测试**

Run: `cargo test workbuddy_close_timeout_requires_confirmation --manifest-path src-tauri/Cargo.toml`

Expected: FAIL，因为事务状态机尚不存在。

- [ ] **Step 3: 实现最小切换事务**

使用注入的 `WorkBuddyProcessController` 和文件路径，按“保存当前快照 → 正常关闭 → 可确认强杀 → 同目录备份 → 临时文件 + 原子重命名 → UID 回读 → 启动 + 轮询验证 → 失败回滚”执行。替换前后都校验目标 UID；所有失败将先恢复原文件，并只在替换前允许强制结束。

- [ ] **Step 4: 增加成功、并发、写入失败和 UID 不匹配测试**

```rust
#[test]
fn concurrent_switch_returns_busy() {
    let _guard = SWITCH_TRANSACTION.lock().unwrap();
    assert_eq!(begin_switch("wb-target").unwrap_err().code, WorkBuddyErrorCode::Busy);
}
```

- [ ] **Step 5: 运行切换模块测试**

Run: `cargo test workbuddy_switch --manifest-path src-tauri/Cargo.toml`

Expected: PASS。

### Task 4: GitHub 聚合 Secret 与后台凭证监测

**Files:**
- Create: `C:/Users/10156/Desktop/脚本/切换应用/source/src-tauri/src/modules/workbuddy_github.rs`
- Create: `C:/Users/10156/Desktop/脚本/切换应用/source/src-tauri/src/modules/workbuddy_session_watcher.rs`
- Modify: `C:/Users/10156/Desktop/脚本/切换应用/source/src-tauri/src/modules/mod.rs`
- Modify: `C:/Users/10156/Desktop/脚本/切换应用/source/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `GitHubRunner` from `work_cn_github` and encrypted WorkBuddy account records.
- Produces: `sync_workbuddy_accounts()`, `trigger_workbuddy_checkin(key)`, `WorkBuddySessionWatchStatus`。

- [ ] **Step 1: 写入聚合 JSON 和 stdin 传递的失败测试**

```rust
#[test]
fn aggregate_secret_is_stably_sorted_and_never_put_in_gh_arguments() {
    let runner = FakeGitHubRunner::new();
    sync_accounts_with(&runner, &[account("wb-z"), account("wb-a")]).unwrap();
    assert_eq!(runner.stdin_for("secret").unwrap(), expected_aggregate_json());
    assert!(!runner.all_arguments().contains(&"access-token".to_string()));
}
```

- [ ] **Step 2: 运行失败测试**

Run: `cargo test workbuddy_aggregate_secret_is_stably_sorted --manifest-path src-tauri/Cargo.toml`

Expected: FAIL，因为聚合构建器尚不存在。

- [ ] **Step 3: 实现同步、运行触发和监测器**

构造版本为 1 的排序 JSON，拒绝启用账号中任何缺 token 的集合；以 SHA-256 摘要记录同步状态。复用 `GitHubRunner` 的 stdin 能力更新 `WORKBUDDY_ACCOUNTS_JSON`，触发动作传入 `workbuddy:<stable-key>`。监测器仅在认证文件元数据或安全摘要变化时稳定读取两次；匹配 UID 才更新快照，token 变化且启用时进行防抖同步。

- [ ] **Step 4: 增加空集合、同步失败和监测回退测试**

```rust
#[test]
fn valid_empty_enabled_set_overwrites_remote_secret() {
    let runner = FakeGitHubRunner::new();
    sync_accounts_with(&runner, &[]).unwrap();
    assert_eq!(runner.stdin_for("secret").unwrap(), r#"{"accounts":[],"version":1}"#);
}
```

- [ ] **Step 5: 运行 GitHub 与监测器测试**

Run: `cargo test workbuddy_github --manifest-path src-tauri/Cargo.toml`

Expected: PASS。

### Task 5: Tauri 命令与 React WorkBuddy 页面

**Files:**
- Create: `C:/Users/10156/Desktop/脚本/切换应用/source/src-tauri/src/commands/workbuddy.rs`
- Modify: `C:/Users/10156/Desktop/脚本/切换应用/source/src-tauri/src/commands/mod.rs`
- Modify: `C:/Users/10156/Desktop/脚本/切换应用/source/src-tauri/src/lib.rs`
- Create: `C:/Users/10156/Desktop/脚本/切换应用/source/src/types/workbuddy.ts`
- Create: `C:/Users/10156/Desktop/脚本/切换应用/source/src/services/workBuddyService.ts`
- Create: `C:/Users/10156/Desktop/脚本/切换应用/source/src/stores/useWorkBuddyStore.ts`
- Create: `C:/Users/10156/Desktop/脚本/切换应用/source/src/pages/WorkBuddyPage.tsx`
- Create: `C:/Users/10156/Desktop/脚本/切换应用/source/src/components/workbuddy/WorkBuddyAddAccountDialog.tsx`
- Create: `C:/Users/10156/Desktop/脚本/切换应用/source/src/components/workbuddy/ForceCloseConfirmDialog.tsx`
- Modify: `C:/Users/10156/Desktop/脚本/切换应用/source/src/App.tsx`
- Modify: `C:/Users/10156/Desktop/脚本/切换应用/source/src/styles/pages.css`

**Interfaces:**
- Produces: Tauri commands `get_workbuddy_installation`, `import_current_workbuddy_account`, `switch_workbuddy_account`, `continue_workbuddy_switch`, `sync_workbuddy_github`, `trigger_workbuddy_checkin`.
- Produces: a dedicated `workbuddy` route and Zustand operations keyed by account ID.

- [ ] **Step 1: 写入前端类型与强制关闭分支的失败测试**

```ts
test('force-close error opens confirmation and cancel releases the switch transaction', async () => {
  mockedSwitch.mockRejectedValue({ code: 'FORCE_CLOSE_REQUIRED', transactionId: 'tx-1' });
  await useWorkBuddyStore.getState().switchTo('wb-a');
  expect(useWorkBuddyStore.getState().pendingForceClose?.transactionId).toBe('tx-1');
  await useWorkBuddyStore.getState().cancelForceClose();
  expect(mockCancel).toHaveBeenCalledWith('tx-1');
});
```

- [ ] **Step 2: 运行失败测试**

Run: `npm run typecheck`

Expected: FAIL，因为 WorkBuddy 类型、服务和 Store 尚不存在。

- [ ] **Step 3: 实现最小命令边界与页面**

命令将结构化错误序列化为前端可解析的安全对象。页面以独立侧栏入口展示检测状态、当前账号和 GitHub 状态；卡片展示备注、掩码号码、到期时间、余额和同步状态，并在导入、切换、同步、删除、立即签到期间禁用重复动作。确认框只有“继续强制关闭”和“取消”，取消必须释放事务。

- [ ] **Step 4: 增加多个账号与窄窗口样式测试**

```ts
test('account card actions remain available in a narrow viewport', () => {
  render(<WorkBuddyPage />, { viewport: { width: 360, height: 800 } });
  expect(screen.getByRole('button', { name: '切换并打开' })).toBeVisible();
});
```

- [ ] **Step 5: 运行前端检查**

Run: `npm run typecheck`

Expected: PASS。

### Task 6: 全链路验证与文档更新

**Files:**
- Modify: `C:/Users/10156/Desktop/脚本/云签到/README.md`
- Modify: `C:/Users/10156/Desktop/脚本/切换应用/source/README.md`

- [ ] **Step 1: 更新用户文档**

明确聚合 Secret 的最小权限、旧变量回退规则、WorkBuddy 官方登录前置条件、强制关闭确认语义与不会上传的字段。

- [ ] **Step 2: 运行云端完整测试**

Run: `python -m unittest discover -s tests -v`

Expected: PASS。

- [ ] **Step 3: 运行桌面端 Rust 完整测试和前端类型检查**

Run: `cargo test --manifest-path src-tauri/Cargo.toml; npm run typecheck`

Expected: 两条命令均以退出码 0 结束。

- [ ] **Step 4: 运行桌面构建**

Run: `cargo check --manifest-path src-tauri/Cargo.toml; npm run build`

Expected: 两条命令均以退出码 0 结束。

- [ ] **Step 5: Windows 手工验收**

在两个真实 WorkBuddy 账号间往返两轮；验证超时取消不改认证文件、启动失败能回滚、单账号 Actions 筛选正确、日志与索引无 token/refresh token/完整手机号。真实凭证和 GitHub 远端状态不写入测试、日志或提交。
