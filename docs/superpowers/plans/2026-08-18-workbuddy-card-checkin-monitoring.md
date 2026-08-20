# WorkBuddy 卡片、云端签到与按需监测 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 统一 TRAE/WorkBuddy 卡片和云端签到信息架构，按需启动 WorkBuddy 监测，并让 GitHub 状态准确区分待同步、成功和真实失败。

**Architecture:** 前端新增无副作用的卡片展示与分组函数，页面继续保留产品适配逻辑但共享结构类名。WorkBuddy Store 缓存会话加载状态，Tauri 通过显式命令幂等启动监测。GitHub 层将前置条件缺失映射为 pending，只有执行远端命令失败才写 failed 和脱敏原因。

**Tech Stack:** React 19、TypeScript 5.8、Zustand 5、Node `node:test`、Tauri 2、Rust 2021、Python 3.11 `unittest`、CSS。

## Global Constraints

- 不自动推送 GitHub 远端仓库，不修改线上 Secret 或工作流。
- 不暴露 token、认证 JSON、命令 stdin 或完整手机号。
- 保留当前脏工作区中的既有改动；生产文件不做整文件提交。
- 所有行为变更先写失败测试并确认按预期失败。
- WorkBuddy 未被查看前不得启动会话监测；页面不得保留 30 秒全量轮询。

---

### Task 1: 统一展示模型、UID 和产品分组

**Files:**
- Create: `src/utils/accountCardPresentation.ts`
- Create: `src/utils/accountCardPresentation.test.ts`
- Modify: `src/utils/checkinProducts.ts`
- Modify: `src/utils/checkinProducts.test.ts`

**Interfaces:**
- Produces: `compactUid(uid: string | null | undefined): string`
- Produces: `githubSyncPresentation(state, error): { label: string; tone: 'pending' | 'syncing' | 'synced' | 'failed'; detail: string | null }`
- Produces: `groupCheckinProducts(items, filter): Array<{ product: 'trae' | 'workbuddy'; items: CheckinAccount[] }>`

- [ ] **Step 1: Write failing presentation tests**

```ts
test('UID 中间省略但短 UID 保持完整', () => {
  assert.equal(compactUid('541b69abcdef84a5'), '541b69…84a5');
  assert.equal(compactUid('uid-1'), 'uid-1');
});

test('GitHub 只有真实失败才使用 failed 文案', () => {
  assert.equal(githubSyncPresentation('pending', null).label, 'GitHub 待同步');
  assert.equal(githubSyncPresentation('failed', '网络不可用').detail, '网络不可用');
});
```

- [ ] **Step 2: Run tests and verify RED**

Run: `node --test src/utils/accountCardPresentation.test.ts src/utils/checkinProducts.test.ts`

Expected: FAIL because the new module and grouping function do not exist.

- [ ] **Step 3: Implement minimal pure functions**

```ts
export function compactUid(uid?: string | null): string {
  if (!uid) return '未知';
  return uid.length <= 12 ? uid : `${uid.slice(0, 6)}…${uid.slice(-4)}`;
}
```

`githubSyncPresentation` must treat unknown values as pending and must return the supplied sanitized detail only for failed. `groupCheckinProducts` must preserve TRAE-before-WorkBuddy product order and original account order.

- [ ] **Step 4: Run tests and verify GREEN**

Run: `node --test src/utils/accountCardPresentation.test.ts src/utils/checkinProducts.test.ts`

Expected: all tests PASS.

### Task 2: WorkBuddy GitHub 状态与错误原因

**Files:**
- Modify: `src-tauri/src/models/workbuddy.rs`
- Modify: `src-tauri/src/modules/workbuddy_account.rs`
- Modify: `src-tauri/src/modules/workbuddy_github.rs`
- Modify: `src-tauri/src/commands/workbuddy.rs`
- Modify: `src/types/workbuddy.ts`
- Modify: `src/stores/useWorkBuddyStore.ts`

**Interfaces:**
- Adds: `WorkBuddyAccountView.lastGithubSyncError: string | null`
- Changes: `mark_github_sync(account_id, state, error)` persists a sanitized optional error.
- Produces: structured sync outcome for local-success/remote-pending/remote-failed feedback.

- [ ] **Step 1: Write Rust failing tests**

```rust
#[test]
fn missing_config_marks_enabled_accounts_pending_not_failed() {
    let result = classify_sync_precondition(false, "");
    assert_eq!(result, SyncPrecondition::Pending("GitHub 签到仓库尚未配置".into()));
}

#[test]
fn failed_sync_error_is_redacted_and_success_clears_it() {
    mark_github_sync(id, "failed", Some("token=secret-value")).unwrap();
    assert!(!load_view(id).last_github_sync_error.unwrap().contains("secret-value"));
    mark_github_sync(id, "synced", None).unwrap();
    assert!(load_view(id).last_github_sync_error.is_none());
}
```

- [ ] **Step 2: Run Rust test and verify RED**

Run: `cargo test --manifest-path src-tauri/Cargo.toml workbuddy_github -- --test-threads=1`

Expected: FAIL because the error field and precondition classifier do not exist.

- [ ] **Step 3: Implement state semantics**

Configuration disabled/empty returns a pending outcome without running `gh`. Payload or actual `gh` failures set failed plus `redact_for_log(error)`. Successful Secret update sets synced and clears the old error. Import/update commands must not swallow a failed outcome without logging or returning user feedback.

- [ ] **Step 4: Run Rust tests and verify GREEN**

Run: `cargo test --manifest-path src-tauri/Cargo.toml workbuddy_github -- --test-threads=1`

Expected: all WorkBuddy GitHub tests PASS.

### Task 3: 按需监测和会话级加载

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/commands/workbuddy.rs`
- Modify: `src-tauri/src/modules/workbuddy_session_watcher.rs`
- Modify: `src/services/workBuddyService.ts`
- Modify: `src/stores/useWorkBuddyStore.ts`
- Modify: `src/pages/WorkBuddyPage.tsx`
- Modify: `src/pages/CheckinPanelPage.tsx`

**Interfaces:**
- Produces Tauri command: `start_workbuddy_session_watcher() -> WorkBuddySessionWatchStatus`
- Produces frontend service: `startWorkBuddySessionWatcher()`
- Store adds: `accountsLoaded`, `monitorStarted`, `ensureAccountsLoaded()`, `ensureMonitoringStarted()`.

- [ ] **Step 1: Write failing lifecycle tests**

Rust tests assert the monitor starts only after explicit activation and repeated activation is idempotent. Type-level/pure state tests assert `ensureAccountsLoaded` skips a second fetch after a successful first load.

- [ ] **Step 2: Run tests and verify RED**

Run: `cargo test --manifest-path src-tauri/Cargo.toml workbuddy_session_watcher -- --test-threads=1`

Run: `node --test src/utils/workBuddyLoading.test.ts`

Expected: FAIL because explicit activation and load-state helper do not exist.

- [ ] **Step 3: Implement lazy activation**

Remove `workbuddy_session_watcher::ensure_started()` from Tauri setup. Register `start_workbuddy_session_watcher`. WorkBuddy page activates once on mount, loads accounts through the guarded Store action, and removes its 30-second timer. Cloud check-in activates only when filter is `all` or `workbuddy`.

- [ ] **Step 4: Verify lifecycle GREEN**

Run both commands from Step 2.

Expected: all lifecycle tests PASS.

### Task 4: 统一本地和云端卡片

**Files:**
- Modify: `src/pages/WorkCnSwitcherPage.tsx`
- Modify: `src/pages/WorkBuddyPage.tsx`
- Modify: `src/pages/CheckinPanelPage.tsx`
- Modify: `src/styles/pages/work-cn.css`
- Modify: `src/styles/pages/workbuddy.css`
- Modify: `src/styles/pages/checkin.css`

**Interfaces:**
- Consumes: `compactUid`, `githubSyncPresentation`, `groupCheckinProducts`.
- Produces shared class contract: `account-card`, `account-card__head`, `account-card__identity`, `account-card__metrics`, `account-card__status`, `account-card__actions`.

- [ ] **Step 1: Add a failing source contract test**

`src/utils/accountCardContract.test.ts` reads the three page modules and asserts TRAE/WorkBuddy cards use the same six structural class names and both identity rows include UID presentation.

- [ ] **Step 2: Run and verify RED**

Run: `node --test src/utils/accountCardContract.test.ts`

Expected: FAIL because current pages use `wc-slot`, `wb-detail-grid`, and `ck-card` structures independently.

- [ ] **Step 3: Implement shared visual structure**

Keep product-specific data/actions inside adapters, but use the same semantic sections. `全部` renders two titled groups. TRAE and WorkBuddy filters render only relevant product commands. Remove the header-only TRAE verification action and keep account-specific actions on cards.

- [ ] **Step 4: Run contract, presentation, type and build checks**

Run: `node --test src/utils/accountCardContract.test.ts src/utils/accountCardPresentation.test.ts src/utils/checkinProducts.test.ts`

Run: `npm run typecheck`

Expected: tests and typecheck PASS.

### Task 5: 云签到聚合契约

**Files:**
- Modify: `C:/Users/10156/Desktop/脚本/云签到/common/config.py`
- Modify: `C:/Users/10156/Desktop/脚本/云签到/main.py`
- Modify: `C:/Users/10156/Desktop/脚本/云签到/.github/workflows/daily-checkin.yml`
- Test: `C:/Users/10156/Desktop/脚本/云签到/tests/test_workbuddy_accounts.py`

**Interfaces:**
- Consumes `WORKBUDDY_ACCOUNTS_JSON` version 1.
- Produces stable `Account.stable_key` filtering and legacy fallback.

- [ ] **Step 1: Run existing aggregate tests as baseline**

Run: `python -m unittest tests.test_workbuddy_accounts -v`

Expected: all tests PASS or expose a concrete contract gap.

- [ ] **Step 2: Add workflow mapping test**

Test reads `.github/workflows/daily-checkin.yml` and asserts it contains `WORKBUDDY_ACCOUNTS_JSON: ${{ secrets.WORKBUDDY_ACCOUNTS_JSON }}`.

- [ ] **Step 3: Run complete cloud tests**

Run: `python -m unittest discover -s tests -v`

Expected: all tests PASS. Do not commit or push this repository automatically.

### Task 6: 完整验证与视觉验收

**Files:**
- Modify only files required by failing checks.

- [ ] **Step 1: Run frontend verification**

Run: `node --test src/utils/*.test.ts`

Run: `npm run typecheck`

Run: `npm run build`

- [ ] **Step 2: Run Rust verification**

Run: `cargo test --manifest-path src-tauri/Cargo.toml workbuddy -- --test-threads=1`

Run: `cargo check --manifest-path src-tauri/Cargo.toml`

- [ ] **Step 3: Start the local application and inspect desktop/narrow layouts**

Run: `npm run tauri:dev`

Capture WorkBuddy and cloud check-in at desktop and narrow viewport widths. Verify card dimensions, UID ellipsis, grouped products, non-overlapping actions, and GitHub failure detail.

- [ ] **Step 4: Final repository checks**

Run: `git diff --check`

Run: `git status --short`

Report verification evidence and list cloud-repository files that remain unpushed.
