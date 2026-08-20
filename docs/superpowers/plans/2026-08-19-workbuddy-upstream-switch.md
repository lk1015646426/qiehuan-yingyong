# WorkBuddy Upstream Switch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the WorkBuddy transaction switch with the cockpit-tools instance-scoped close, account injection, launch, and window activation flow.

**Architecture:** `commands/workbuddy.rs` orchestrates one blocking operation. `workbuddy_account.rs` owns atomic snapshot injection and verification. `process.rs` identifies WorkBuddy by command line/user-data directory and uses the existing scoped close and window activation helpers.

**Tech Stack:** Rust, Tauri, serde_json, Windows Win32 APIs, existing atomic-write/process helpers.

## Global Constraints

- Preserve unrelated user changes.
- Keep custom WorkBuddy auth path settings supported.
- Never return `FORCE_CLOSE_REQUIRED` from the new switch.
- Verify with Rust tests, frontend typecheck/build, and Tauri installer build.

### Task 1: Account Injection

- Add a failing test for clearing the logout marker, atomic writing, and UID/token verification.
- Implement `write_account_to_default_client(account_id)` using the existing encrypted snapshot, configured auth path, and `atomic_write`.

### Task 2: Process Detection

- Add a failing test that rejects an embedded `node.exe` as the WorkBuddy launch executable.
- Use `~/.workbuddy/app` as the default user-data directory and detect WorkBuddy process trees by WorkBuddy command-line markers rather than exact executable path.

### Task 3: Switch Command

- Remove the transaction module and continue/cancel commands.
- Orchestrate account lookup, scoped close, injection, default launch, window activation, last-used/tray state, and structured errors.

### Task 4: Verification and Packaging

- Run formatting, Rust tests, frontend checks, and `tauri build`.
- Record installer path, size, SHA-256, and remaining platform limitations.
