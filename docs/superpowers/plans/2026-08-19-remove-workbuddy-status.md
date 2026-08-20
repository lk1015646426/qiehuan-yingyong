# Remove WorkBuddy Status Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Completely remove WorkBuddy credits, today's reward, and consecutive check-in status querying and display.

**Architecture:** Remove the dedicated backend status boundary first, then remove its frontend consumers and UI. A source-contract test proves the deleted feature cannot be accidentally left registered or displayed.

**Tech Stack:** Rust, Tauri 2, React, TypeScript, Zustand, Node test runner

## Global Constraints

- Only WorkBuddy status functionality is removed.
- WorkBuddy switching, launching, check-in, monitoring, settings, and GitHub synchronization remain.
- Trae/Work CN credit functionality remains unchanged.

---

### Task 1: Add the removal contract

**Files:**
- Modify: `src/utils/workBuddyLifecycle.test.ts`

- [ ] Replace status-query behavior tests with assertions that the backend command/module, frontend service/store fields, status labels, refresh button, and retry timer are absent.
- [ ] Run `node --test src/utils/workBuddyLifecycle.test.ts` and confirm the new contract fails before implementation.

### Task 2: Remove the backend status boundary

**Files:**
- Delete: `src-tauri/src/modules/workbuddy_status.rs`
- Modify: `src-tauri/src/modules/mod.rs`
- Modify: `src-tauri/src/commands/workbuddy.rs`
- Modify: `src-tauri/src/models/workbuddy.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/modules/workbuddy_account.rs`

- [ ] Remove the status command, model, module registration, invoke registration, and parser integration test.
- [ ] Run the WorkBuddy Rust tests and confirm compilation succeeds.

### Task 3: Remove frontend state and UI

**Files:**
- Modify: `src/types/workbuddy.ts`
- Modify: `src/services/workBuddyService.ts`
- Modify: `src/stores/useWorkBuddyStore.ts`
- Modify: `src/pages/WorkBuddyPage.tsx`

- [ ] Remove the status type and service function.
- [ ] Remove status maps, request generations, refresh action, and deletion cleanup.
- [ ] Remove automatic refresh/retry effects, three metrics, status error copy, and refresh-status button.

### Task 4: Verify zero residue and package

**Files:**
- Test: `src/utils/workBuddyLifecycle.test.ts`

- [ ] Run the WorkBuddy frontend tests, TypeScript typecheck, Rust WorkBuddy tests, and production build.
- [ ] Search the WorkBuddy files for all removed identifiers and labels; expect no matches.
- [ ] Build the NSIS installer and report its path and SHA-256.
