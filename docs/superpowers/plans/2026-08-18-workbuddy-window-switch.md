# WorkBuddy Window Switch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 WorkBuddy 切换并打开流程可靠激活目标窗口，并覆盖窗口与启动异常。

**Architecture:** 在 WorkBuddy 运行时接口中加入窗口激活边界；Windows 通过实时进程树和 Win32 顶层窗口枚举恢复/置前窗口并校验前台 PID，切换事务在启动和账号校验成功后调用该边界，失败沿用备份回滚。

**Tech Stack:** Rust、Tauri、Windows user32 API（通过 PowerShell/C# 调用以保持现有依赖边界）、现有 WorkBuddy 进程解析器、Cargo tests。

## Global Constraints

- 不覆盖工作区已有未提交改动。
- 不缓存跨事务 HWND；每次激活都重新验证句柄和前台 PID。
- 保留现有优雅关闭、强制关闭确认和认证备份回滚语义。
- 所有新增行为必须有先失败后通过的回归测试。

### Task 1: Add the failing activation contract test

**Files:**
- Modify: `src-tauri/src/modules/workbuddy_switch.rs` test module

- [ ] **Step 1: Extend the fake runtime with activation state and add tests**

Add an `activated` counter and `activate_window` method to `FakeRuntime`; add tests asserting both running and not-running flows call activation after launch, and an activation error returns `WINDOW_ACTIVATION_FAILED` and restores the original file.

- [ ] **Step 2: Run the focused test and verify RED**

Run `cargo test -p cockpit --lib modules::workbuddy_switch -- --nocapture` (or the workspace package selected by `Cargo.toml`). Expected: compilation/test failure because the trait and error code do not yet exist.

### Task 2: Implement robust WorkBuddy activation

**Files:**
- Modify: `src-tauri/src/models/workbuddy.rs`
- Modify: `src-tauri/src/modules/workbuddy_switch.rs`
- Modify: `src-tauri/src/modules/process.rs`

- [ ] **Step 1: Add `WindowActivationFailed` error code and trait method**

Add `activate_window(&self) -> Result<(), String>` to `WorkBuddySwitchRuntime`; implement the fake method and the real method.

- [ ] **Step 2: Add Windows window enumeration and foreground verification**

Implement a WorkBuddy-specific helper that re-collects matching process IDs, enumerates visible/enabled top-level windows, restores minimized windows, calls `BringWindowToTop` and `SetForegroundWindow`, then verifies `GetForegroundWindow` belongs to a matching PID. Retry until a bounded timeout and report permission/empty-window errors.

- [ ] **Step 3: Call activation from the switch transaction**

After `launch` and account verification, call `activate_window`; map errors to `WINDOW_ACTIVATION_FAILED` so existing rollback handles failed activation.

- [ ] **Step 4: Run focused tests and verify GREEN**

Run the focused WorkBuddy switch test command and confirm all new and existing cases pass.

### Task 3: Full verification

**Files:**
- No additional source files unless a test exposes a required scoped fix.

- [ ] **Step 1: Run Rust formatting and tests**

Run `cargo fmt --all -- --check` and `cargo test --workspace --all-targets`.

- [ ] **Step 2: Run frontend tests/build**

Run `npm test -- --runInBand` when supported by `package.json`, then `npm run build`.

- [ ] **Step 3: Inspect the final diff and report limits**

Run `git diff --stat` and `git diff --check`; record actual command results and note OS foreground restrictions or unavailable real-account verification.
