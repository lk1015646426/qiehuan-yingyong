# WorkBuddy Real Credits Implementation Plan

> **For agentic workers:** Execute inline with test-driven development. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show every saved WorkBuddy account's real credits, today's reward, and streak in the desktop switcher using read-only upstream requests.

**Architecture:** A new Rust status module owns upstream querying and response parsing. A Tauri command exposes one-account queries; the frontend service/store coordinates sequential refreshes and the existing account card renders compact per-account state.

**Tech Stack:** Rust, Tauri 2, reqwest, Serde JSON, React, TypeScript, Zustand, Node test runner

## Global Constraints

- Query only `get-user-resource` and `checkin-activity-status`.
- Never call `daily-checkin` from the status feature.
- Keep successful partial data when the other endpoint fails.
- Preserve all unrelated dirty-worktree changes.

---

### Task 1: Define the positive source contract

**Files:**
- Modify: `src/utils/workBuddyLifecycle.test.ts`
- Test: `src/utils/workBuddyLifecycle.test.ts`

- [ ] Replace the removal assertion with required command, module, model, service, store, UI labels, refresh-all, and per-account refresh assertions.
- [ ] Assert the new status module source does not contain the daily check-in endpoint.
- [ ] Run `node --test src/utils/workBuddyLifecycle.test.ts` and confirm failure because the feature is absent.

### Task 2: Implement and test the Rust read-only status boundary

**Files:**
- Create: `src-tauri/src/modules/workbuddy_status.rs`
- Modify: `src-tauri/src/modules/mod.rs`
- Modify: `src-tauri/src/models/workbuddy.rs`
- Modify: `src-tauri/src/commands/workbuddy.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Produces: `query_account_status(account_id: &str) -> WorkBuddyAccountStatus`
- Produces: `get_workbuddy_account_status(account_id: String) -> Result<WorkBuddyAccountStatus, String>`

- [ ] Add parser tests for real package filtering, precise decimals, activity fields, and partial errors.
- [ ] Implement two read-only POST requests with bearer authentication and 30-second timeout.
- [ ] Register the module and Tauri command.
- [ ] Run `cargo test workbuddy_status --manifest-path src-tauri/Cargo.toml`.

### Task 3: Implement frontend state and compact UI

**Files:**
- Modify: `src/types/workbuddy.ts`
- Modify: `src/services/workBuddyService.ts`
- Modify: `src/stores/useWorkBuddyStore.ts`
- Modify: `src/pages/WorkBuddyPage.tsx`
- Modify: `src/styles/pages/workbuddy.css`

**Interfaces:**
- Produces: `getWorkBuddyAccountStatus(accountId)`
- Produces: `refreshAccountStatus(accountId)` and `refreshAllStatuses()`

- [ ] Add per-account status/loading/error maps and sequential refresh-all behavior.
- [ ] Refresh all accounts in the background after account loading.
- [ ] Render credits, today's reward, streak, partial error, refresh-all, and per-account refresh.
- [ ] Keep card dimensions stable and responsive at the existing mobile breakpoint.

### Task 4: Verify and package

- [ ] Run WorkBuddy Node tests and TypeScript typecheck.
- [ ] Run Rust WorkBuddy tests and production build.
- [ ] Query all seven real saved accounts through the new desktop-side code path or an equivalent compiled probe.
- [ ] Build the NSIS installer and report path, size, and SHA-256.
