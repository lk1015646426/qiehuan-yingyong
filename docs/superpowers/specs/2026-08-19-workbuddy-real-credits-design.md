# WorkBuddy Real Credits Design

## Scope

Restore read-only WorkBuddy account status in the desktop switcher. Each saved account shows its real credit balance, today's reward, consecutive check-in days, last refresh time, and an account-scoped query error when applicable.

The feature must never call the daily check-in endpoint. Automatic check-in, manual cloud check-in, account switching, GitHub synchronization, and TRAE credit behavior remain unchanged.

## Architecture

Add a dedicated Rust `workbuddy_status` module. It reads the encrypted saved account token through `workbuddy_account::access_token`, calls only `get-user-resource` and `checkin-activity-status`, parses each response independently, and returns partial data when one endpoint fails.

Expose `get_workbuddy_account_status(account_id)` as an async Tauri command. The React service and Zustand store keep status, loading, and error state per stable account ID. Account loading triggers a sequential background refresh to avoid request bursts; the page also provides refresh-all and per-account retry actions.

## Data Contract

`WorkBuddyAccountStatus` contains:

- `credits: number | null`
- `todayReward: number | null`
- `streakDays: number | null`
- `updatedAt: number`
- `creditsError: string | null`
- `activityError: string | null`

Real credits are the sum of packages where `CapacityType == 1` and `Status == 0`, preferring `CapacityRemainPrecise` over `CapacityRemain`. The activity parser accepts both `today_credit` and the legacy `daily_credit` field.

## UI

Keep the existing compact account card. Add one unframed metrics row beneath identity/token metadata with three scan-friendly values: remaining credits, today's reward, and streak. Loading uses stable placeholder text; partial failures leave successful values visible and show one compact inline error. Header refresh-all and per-account refresh buttons use the existing button vocabulary.

## Success Criteria

- All seven saved WorkBuddy accounts can query independently without switching the official client account.
- No status query sends a request to `daily-checkin`.
- A failed account or endpoint does not hide other accounts' successful data.
- TypeScript tests, Rust WorkBuddy tests, typecheck, production build, and NSIS packaging pass.
